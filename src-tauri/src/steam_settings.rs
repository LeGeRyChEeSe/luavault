//! LuaVault's settings inside the Steam client (experimental edition).
//!
//! The « LV » pill of the client's title bar opens a panel drawn by the main window's script
//! (`scripts/steam-client-pill.js`). It asks `settings`; the application answers
//! `lvSettings(<json>)` with [`View`]. A change comes back as `set:<key>:<value>` under a closed set
//! ([`Change::parse`]) and is written by **the very commands the window's Settings call**, so a switch
//! flipped in Steam registers exactly what the same switch in the window registers. Only what makes
//! sense from Steam is here; folders, backups and the switch that would take the panel away
//! (« LuaVault dans Steam ») stay in the window, which the panel opens.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::config::{self, AppConfig};

/// What the panel shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub version: String,
    pub experimental: bool,
    /// `open_steam_tool`, `classic` or `none` (`detect::Flavor`).
    pub steamtools: &'static str,
    pub locale: &'static str,
    pub auto_update: bool,
    pub background_notify: bool,
    pub background_guard: bool,
    pub watch_downloads: bool,
    pub library_block: bool,
    pub collection: bool,
    pub portable: bool,
    /// The outcome of the last change, when it failed (shown under the panel's title).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn view_of(cfg: &AppConfig, steamtools: crate::detect::Flavor, lang: &'static str) -> View {
    View {
        version: env!("CARGO_PKG_VERSION").to_string(),
        experimental: config::EXPERIMENTAL,
        steamtools: match steamtools {
            crate::detect::Flavor::OpenSteamTool => "open_steam_tool",
            crate::detect::Flavor::Classic => "classic",
            crate::detect::Flavor::None => "none",
        },
        locale: lang,
        auto_update: cfg.auto_update_chosen(),
        background_notify: cfg.background_notify == Some(true),
        background_guard: cfg.background_guard != Some(false),
        watch_downloads: crate::commands::downloads_watch_wanted(cfg),
        library_block: cfg.steam_library != Some(false),
        collection: cfg.steam_collection != Some(false),
        portable: config::is_portable(),
        error: None,
    }
}

/// One change the panel may ask. Values are parsed here; the page names neither a file nor a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    AutoUpdate(bool),
    BackgroundNotify(bool),
    BackgroundGuard(bool),
    WatchDownloads(bool),
    LibraryBlock(bool),
    Collection(bool),
    Locale(&'static str),
}

impl Change {
    /// `<key>:<value>`, the part after `set:`.
    pub fn parse(rest: &str) -> Option<Change> {
        let (key, value) = rest.split_once(':')?;
        let flag = || match value {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        };
        Some(match key {
            "auto_update" => Change::AutoUpdate(flag()?),
            "background_notify" => Change::BackgroundNotify(flag()?),
            "background_guard" => Change::BackgroundGuard(flag()?),
            "watch_downloads" => Change::WatchDownloads(flag()?),
            "library_block" => Change::LibraryBlock(flag()?),
            "collection" => Change::Collection(flag()?),
            "locale" => Change::Locale(match value {
                "fr" => "fr",
                "en" => "en",
                _ => return None,
            }),
            _ => return None,
        })
    }
}

/// The view as it stands now.
pub fn current(app: &AppHandle) -> View {
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    let flavor = crate::commands::resolve_steam(&cfg).map(|s| crate::detect::flavor(&s)).unwrap_or(crate::detect::Flavor::None);
    view_of(&cfg, flavor, crate::steam_store::lang_of(&cfg))
}

pub fn settings_call(view: &View) -> String {
    let json = serde_json::to_string(view).unwrap_or_else(|_| "null".to_string());
    format!("window.lvSettings && window.lvSettings({json})")
}

/// Send the panel what it shows. It may be drawn by the main window or, while a page of Valve's is
/// shown, by the web view laid over it.
pub fn send(app: &AppHandle, error: Option<String>) {
    let mut view = current(app);
    view.error = error;
    let call = settings_call(&view);
    crate::steam_client::to_store(call.clone());
    crate::steam_client::to_window(call);
}

/// Apply one change through the window's own command, then send the panel the new view.
pub async fn apply(app: &AppHandle, change: Change) {
    let outcome = run(app, change.clone()).await;
    match &outcome {
        Err(e) => log::warn!("LuaVault dans Steam : réglage {change:?} refusé ({e})"),
        Ok(()) => log::info!("LuaVault dans Steam : réglage {change:?}"),
    }
    send(app, outcome.err());
}

const LIB_RESET: &str = "window.lvLibReset && window.lvLibReset()";

fn save_with(app: &AppHandle, edit: impl FnOnce(&mut AppConfig)) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let mut cfg = state.config.lock().unwrap().clone();
    edit(&mut cfg);
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(())
}

async fn run(app: &AppHandle, change: Change) -> Result<(), String> {
    use crate::commands;
    let state = app.state::<crate::AppState>();
    match change {
        Change::AutoUpdate(on) => commands::set_auto_update(state, on).await.map(|_| ()),
        Change::BackgroundNotify(on) => commands::set_background_notify(state, on).await.map(|_| ()),
        Change::BackgroundGuard(on) => commands::set_background_guard(app.clone(), state, on).await.map(|_| ()),
        Change::WatchDownloads(on) => commands::set_downloads_watch(app.clone(), state, on).map(|_| ()),
        Change::Locale(lang) => {
            commands::set_locale(state, lang.to_string()).await?;
            // The play bar's texts are asked again in the new language.
            crate::steam_client::to_window(LIB_RESET.to_string());
            crate::steam_library::send_dots(app).await;
            Ok(())
        }
        Change::LibraryBlock(on) => {
            save_with(app, |cfg| cfg.steam_library = Some(on))?;
            crate::steam_client::to_window(LIB_RESET.to_string());
            crate::steam_library::send_dots(app).await;
            Ok(())
        }
        Change::Collection(on) => {
            save_with(app, |cfg| cfg.steam_collection = Some(on))?;
            // Only the shared context is equipped again: it fills the collection, or removes it.
            crate::steam_client::refresh_collection();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_settings_changes_are_a_closed_set() {
        assert_eq!(Change::parse("auto_update:1"), Some(Change::AutoUpdate(true)));
        assert_eq!(Change::parse("background_guard:0"), Some(Change::BackgroundGuard(false)));
        assert_eq!(Change::parse("watch_downloads:0"), Some(Change::WatchDownloads(false)));
        assert_eq!(Change::parse("library_block:0"), Some(Change::LibraryBlock(false)));
        assert_eq!(Change::parse("collection:1"), Some(Change::Collection(true)));
        assert_eq!(Change::parse("locale:en"), Some(Change::Locale("en")));
        for forged in [
            "", "auto_update", "auto_update:", "auto_update:true", "auto_update:1:2", "collection:oui", "locale:de",
            "in_steam:0", "steam_dir:C:\\x", "library_dir:1", "watch_dirs:C:\\",
        ] {
            assert_eq!(Change::parse(forged), None, "{forged}");
        }
    }

    #[test]
    fn steam_settings_view_reads_the_config_with_its_defaults() {
        let view = view_of(&AppConfig::default(), crate::detect::Flavor::Classic, "fr");
        assert!(view.auto_update && view.background_guard && view.watch_downloads, "on unless switched off");
        assert!(!view.background_notify, "off unless asked");
        assert!(view.library_block && view.collection, "on unless switched off");
        assert_eq!(view.steamtools, "classic");
        let off = AppConfig { steam_library: Some(false), steam_collection: Some(false), auto_update: Some(false), ..Default::default() };
        let view = view_of(&off, crate::detect::Flavor::None, "en");
        assert!(!view.library_block && !view.collection && !view.auto_update);
        let call = settings_call(&view);
        assert!(call.starts_with("window.lvSettings && window.lvSettings({"), "{call}");
        assert!(!call.contains("\"error\""), "no error, no field");
    }
}

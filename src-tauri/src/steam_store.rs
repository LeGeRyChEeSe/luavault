//! The store button of « LuaVault dans Steam »: what a click on « Trouver sur le site » does.
//!
//! LuaVault never downloads a game's files itself. A click opens the source site's home page in the
//! person's browser (`commands::source_site_url`: the home page, never a game or an AppID in the
//! address) and remembers that the game was asked for (`store-requests.json`, 30 minutes). The person
//! downloads the `.lua` and the patch there; the downloads detection (`downloads.rs`) sees them arrive.
//! The panel of the store says each step.

use std::path::{Path, PathBuf};

use log::{info, warn};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::steam_client::{self, Job};
use crate::{config, notify, vdf};

/// A Steam AppID as the pages may send it: 1 to 10 digits, the first not zero. Anything else is
/// refused whole (no trimming, no list).
pub fn parse_app_id(raw: &str) -> Option<String> {
    let ok = !raw.is_empty()
        && raw.len() <= 10
        && raw.bytes().all(|b| b.is_ascii_digit())
        && !raw.starts_with('0');
    ok.then(|| raw.to_string())
}

/// How Steam holds a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamSide {
    /// Installed and complete.
    Ready,
    /// Steam knows it: downloading, updating, or queued.
    Downloading,
    /// Steam does not have it.
    Absent,
    /// Steam's folder or its manifests could not be read.
    Unknown,
}

/// `located` is `None` when the manifests could not be read at all.
pub fn steam_side(steam_resolved: bool, located: Option<&vdf::GameInstall>) -> SteamSide {
    match (steam_resolved, located) {
        (false, _) | (_, None) => SteamSide::Unknown,
        (_, Some(game)) if game.installed && game.fully_installed => SteamSide::Ready,
        (_, Some(game)) if game.known_to_steam => SteamSide::Downloading,
        _ => SteamSide::Absent,
    }
}

// ------------------------------------------------------------ what was asked for

/// How long a click on the store button stands for « this person wants this game's files ».
pub const REQUEST_TTL_SECS: i64 = 30 * 60;
/// At most this many games remembered.
pub const MAX_REQUESTS: usize = 32;
/// A second click on the same game within this time is the same click.
pub const SAME_CLICK_SECS: i64 = 3;
const REQUESTS_FILE: &str = "store-requests.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    #[serde(rename = "appId")]
    pub app_id: String,
    pub at: i64,
}

pub fn requests_path(data_dir: &Path) -> PathBuf {
    data_dir.join(REQUESTS_FILE)
}

/// Pure. What a file read back may keep: strict AppIDs, not expired, not from the future, one per
/// game (the latest), `MAX_REQUESTS` at most (the newest).
pub fn sanitize(list: Vec<Request>, now: i64) -> Vec<Request> {
    let mut kept: Vec<Request> = Vec::new();
    for r in list {
        if parse_app_id(&r.app_id).is_none() || r.at > now + 60 || now.saturating_sub(r.at) >= REQUEST_TTL_SECS {
            continue;
        }
        if let Some(old) = kept.iter_mut().find(|k| k.app_id == r.app_id) {
            old.at = old.at.max(r.at);
        } else {
            kept.push(r);
        }
    }
    kept.sort_by_key(|r| r.at);
    let extra = kept.len().saturating_sub(MAX_REQUESTS);
    kept.drain(..extra);
    kept
}

/// The live requests. An unreadable file is no request at all.
pub fn load_requests(data_dir: &Path, now: i64) -> Vec<Request> {
    let raw = std::fs::read_to_string(requests_path(data_dir)).unwrap_or_default();
    sanitize(serde_json::from_str(&raw).unwrap_or_default(), now)
}

fn save_requests(data_dir: &Path, list: &[Request]) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let path = requests_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(list).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// Remember a click. `false` when the same game was asked a moment ago (a double click).
pub fn remember(data_dir: &Path, app_id: &str, now: i64) -> bool {
    let mut list = load_requests(data_dir, now);
    if list.iter().any(|r| r.app_id == app_id && now.saturating_sub(r.at) < SAME_CLICK_SECS) {
        return false;
    }
    list.push(Request { app_id: app_id.to_string(), at: now });
    let list = sanitize(list, now);
    if let Err(e) = save_requests(data_dir, &list) {
        warn!("LuaVault dans Steam : demande non enregistrée ({e})");
    }
    true
}

/// Whether this game was asked for from Steam in the last `REQUEST_TTL_SECS`.
pub fn requested(data_dir: &Path, app_id: &str, now: i64) -> bool {
    load_requests(data_dir, now).iter().any(|r| r.app_id == app_id)
}

// ------------------------------------------------------------ the click

/// What the person goes to the site for: the game (its `.lua`, and the patch if there is one), or
/// only the patch of a game LuaVault already holds (the play bar's « Trouver »).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted {
    Game,
    PatchOnly,
}

/// Pure. The panel's line once the site is open (or not).
pub fn find_line(lang: &str, name: &str, app_id: &str, wanted: Wanted, watching: bool, opened: Result<(), String>) -> (&'static str, String) {
    let fr = lang == "fr";
    match opened {
        Err(e) => (
            "error",
            if fr { format!("Le site n'a pas pu s'ouvrir : {e}") } else { format!("The site could not open: {e}") },
        ),
        Ok(()) if watching && wanted == Wanted::PatchOnly => (
            "busy",
            if fr {
                format!("Site ouvert dans votre navigateur. Cherchez « {name} » (AppID {app_id}) et téléchargez son correctif s'il y en a un : LuaVault le reprendra dans vos Téléchargements.")
            } else {
                format!("Site opened in your browser. Look for « {name} » (AppID {app_id}) and download its patch if there is one: LuaVault will pick it up from your Downloads.")
            },
        ),
        Ok(()) if watching => (
            "busy",
            if fr {
                format!("Site ouvert dans votre navigateur. Cherchez « {name} » (AppID {app_id}), téléchargez le .lua et, s'il y en a un, le correctif : LuaVault les reprendra dans vos Téléchargements.")
            } else {
                format!("Site opened in your browser. Look for « {name} » (AppID {app_id}), download the .lua and, if there is one, the patch: LuaVault will pick them up from your Downloads.")
            },
        ),
        Ok(()) => (
            "warn",
            if fr {
                format!("Site ouvert dans votre navigateur. Cherchez « {name} » (AppID {app_id}) et téléchargez ses fichiers, puis ajoutez-les dans LuaVault : la détection des Téléchargements est coupée dans ses Réglages.")
            } else {
                format!("Site opened in your browser. Look for « {name} » (AppID {app_id}) and download its files, then add them in LuaVault: download detection is off in its Settings.")
            },
        ),
    }
}

/// The language LuaVault speaks to this person.
pub fn lang_of(cfg: &config::AppConfig) -> &'static str {
    notify::pick_language(cfg.locale.as_deref(), notify::system_locale().as_deref())
}

/// The game's name: the library's, then Steam's manifest, then the cached store lookup.
pub async fn game_name(app: &AppHandle, app_id: &str, lang: &str) -> String {
    let state = app.state::<crate::AppState>();
    let cfg = state.config.lock().unwrap().clone();
    let id = app_id.to_string();
    let local = tauri::async_runtime::spawn_blocking(move || {
        let lib = crate::library::load_index(&cfg.resolved_library_dir());
        if let Some(entry) = lib.iter().find(|e| e.app_id == id) {
            return Some(entry.name.clone());
        }
        crate::commands::resolve_steam(&cfg).and_then(|steam| vdf::locate_game(&steam, &id).steam_name)
    })
    .await
    .ok()
    .flatten();
    if let Some(name) = local.filter(|n| !n.trim().is_empty()) {
        return name;
    }
    match tokio::time::timeout(std::time::Duration::from_secs(3), crate::commands::cached_steam_details(&state, app_id, lang)).await {
        Ok(Ok(details)) if !details.name.trim().is_empty() => details.name,
        _ => format!("AppID {app_id}"),
    }
}

/// A click on « Trouver sur le site ».
pub async fn find(app: &AppHandle, app_id: &str, wanted: Wanted) {
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    let now = chrono::Utc::now().timestamp();
    if crate::kill_switch::engaged(&config::data_dir()) || !remember(&config::data_dir(), app_id, now) {
        return;
    }
    let lang = lang_of(&cfg);
    let opened = crate::commands::source_site_url()
        .ok_or_else(|| "adresse du site invalide".to_string())
        .and_then(|url| tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| e.to_string()));
    if let Err(e) = &opened {
        warn!("LuaVault dans Steam : site non ouvert ({e})");
    } else {
        info!("LuaVault dans Steam : site ouvert pour {app_id}");
    }
    let name = game_name(app, app_id, lang).await;
    let (tone, line) = find_line(lang, &name, app_id, wanted, crate::commands::downloads_watch_wanted(&cfg), opened);
    steam_client::publish(Job {
        app_id: app_id.to_string(),
        name,
        tone,
        line,
        install: false,
        library: false,
        site: tone != "error",
        at: 0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_store_app_id_rule_is_strict() {
        for ok in ["1", "264710", "9999999999"] {
            assert_eq!(parse_app_id(ok).as_deref(), Some(ok));
        }
        for bad in ["", "0", "0264710", "12345678901", "264710,848450", " 264710", "264710 ", "26a710", "-1", "+1"] {
            assert_eq!(parse_app_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn steam_store_reads_steam_as_it_stands() {
        let game = |known, installed, full| vdf::GameInstall { known_to_steam: known, installed, fully_installed: full, ..Default::default() };
        assert_eq!(steam_side(true, Some(&game(true, true, true))), SteamSide::Ready);
        assert_eq!(steam_side(true, Some(&game(true, true, false))), SteamSide::Downloading);
        assert_eq!(steam_side(true, Some(&game(true, false, false))), SteamSide::Downloading);
        assert_eq!(steam_side(true, Some(&game(false, false, false))), SteamSide::Absent);
        assert_eq!(steam_side(false, Some(&game(true, true, true))), SteamSide::Unknown);
        assert_eq!(steam_side(true, None), SteamSide::Unknown);
    }

    #[test]
    fn steam_store_requests_keep_the_live_strict_and_newest() {
        let r = |id: &str, at: i64| Request { app_id: id.into(), at };
        let now = 10_000;
        let kept = sanitize(
            vec![r("264710", now - 10), r("0264710", now), r("1", now - REQUEST_TTL_SECS), r("2", now + 3600), r("264710", now - 5)],
            now,
        );
        assert_eq!(kept, vec![r("264710", now - 5)], "strict, live, one per game at its latest");
        let many: Vec<Request> = (1..=40).map(|i| r(&i.to_string(), now - 40 + i)).collect();
        let kept = sanitize(many, now);
        assert_eq!(kept.len(), MAX_REQUESTS);
        assert_eq!(kept.last().unwrap().app_id, "40", "the newest stay");
    }

    #[test]
    fn steam_store_remembers_a_click_once_for_half_an_hour() {
        let dir = std::env::temp_dir().join(format!("lv_requests_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(remember(&dir, "264710", 1_000));
        assert!(!remember(&dir, "264710", 1_001), "a double click is one click");
        assert!(requested(&dir, "264710", 1_100));
        assert!(!requested(&dir, "264710", 1_000 + REQUEST_TTL_SECS), "30 minutes later it is no request");
        assert!(!requested(&dir, "848450", 1_100));
        std::fs::write(requests_path(&dir), "pas du json").unwrap();
        assert!(load_requests(&dir, 1_300).is_empty(), "unreadable is nothing asked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn steam_store_line_says_what_to_do_and_never_claims_a_download() {
        let (tone, line) = find_line("fr", "Subnautica", "264710", Wanted::Game, true, Ok(()));
        assert_eq!(tone, "busy");
        assert!(line.contains("Subnautica") && line.contains("264710") && line.contains("Téléchargements"), "{line}");
        let (tone, line) = find_line("en", "Subnautica", "264710", Wanted::Game, false, Ok(()));
        assert_eq!(tone, "warn");
        assert!(line.contains("download detection is off"), "{line}");
        let (tone, _) = find_line("fr", "x", "1", Wanted::PatchOnly, true, Err("refus".into()));
        assert_eq!(tone, "error");
        let (tone, line) = find_line("fr", "Stardew Valley", "413150", Wanted::PatchOnly, true, Ok(()));
        assert_eq!(tone, "busy");
        assert!(line.contains("correctif") && !line.contains(".lua"), "a game LuaVault holds is not asked its .lua again: {line}");
        for lang in ["fr", "en"] {
            for wanted in [Wanted::Game, Wanted::PatchOnly] {
                for watching in [true, false] {
                    let (_, line) = find_line(lang, "n", "1", wanted, watching, Ok(()));
                    assert!(!line.contains('\u{2014}'), "no em dash: {line}");
                }
            }
        }
    }
}

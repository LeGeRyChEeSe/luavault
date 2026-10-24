//! The resident side of the download guard: the tray icon, the notifications and the loop
//! that feeds `source_guard` with what Steam writes.
//!
//! The decisions are `source_guard`'s; this file only carries them out, so there is
//! nothing here worth a unit test that the pure module does not already have. What can
//! only be checked on a desktop (the icon, the menu, the banners) is checked by hand.
//!
//! Off in a debug build unless `AST_GUARD` is set: the bench runs debug binaries, and a
//! resident icon, a hidden window and a registered logon entry pointing at a throwaway
//! sandbox executable have no place there.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use log::{info, warn};
use tauri::menu::{MenuBuilder, MenuItem, MenuItemBuilder};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::config::{self, AppConfig};
use crate::source_guard::{note_texts, parse_event, tray_lines, Guard, Switch, Tail, View};
use crate::{detect, manifest_fallback, notify, steamtools_source, AppState};

const TRAY_ID: &str = "download-guard";
/// One tag: each notification replaces the previous one instead of piling up.
const TOAST_TAG: &str = "download-guard";

/// Is the guard running right now (icon shown, closing the window hides it).
static ON: AtomicBool = AtomicBool::new(false);
static LOOP_STARTED: AtomicBool = AtomicBool::new(false);
static HINTED: AtomicBool = AtomicBool::new(false);

struct Items {
    status: MenuItem<Wry>,
    detail: MenuItem<Wry>,
    /// The way in that Do Not Disturb cannot silence: a game file is missing, one click.
    manifest: MenuItem<Wry>,
    open: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

/// The head line of the tray as last shown, so the tooltip can carry the missing-file
/// reminder after it without the two refreshes undoing each other.
static HEAD: Mutex<String> = Mutex::new(String::new());

static ITEMS: Mutex<Option<Items>> = Mutex::new(None);

pub fn is_on() -> bool {
    ON.load(Ordering::SeqCst)
}

/// The user's choice: on unless switched off. A debug build never starts it by itself.
pub fn wanted(cfg: &AppConfig) -> bool {
    cfg.background_guard != Some(false) && (!cfg!(debug_assertions) || std::env::var_os("AST_GUARD").is_some())
}

fn lang(cfg: &AppConfig) -> &'static str {
    notify::pick_language(cfg.locale.as_deref(), notify::system_locale().as_deref())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// The tray entry for a missing game file, and the line added to the tooltip. Windows' Do Not
/// Disturb sends every toast straight to the notification centre, so this entry is the one
/// signal that cannot be muted. No provider is named here, as nowhere in the tray.
fn manifest_menu(lang: &str, pending: bool) -> (&'static str, Option<&'static str>) {
    match (lang == "fr", pending) {
        (true, true) => ("Un fichier de jeu manque : obtenir une clé", Some("Un fichier de jeu manque")),
        (false, true) => ("A game file is missing: get a key", Some("A game file is missing")),
        (true, false) => ("Aucun fichier de jeu manquant", None),
        (false, false) => ("No game file missing", None),
    }
}

fn labels(lang: &str) -> (String, &'static str) {
    let open = if lang == "fr" { format!("Ouvrir {}", notify::display_name()) } else { format!("Open {}", notify::display_name()) };
    (open, if lang == "fr" { "Quitter" } else { "Quit" })
}

fn build_tray(app: &AppHandle, cfg: &AppConfig) -> tauri::Result<()> {
    if app.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    let l = lang(cfg);
    let (head, detail) = tray_lines(l, &View { status: crate::source_guard::Status::Online, tries: 0, retry_in: None, next_round_in: None });
    let (open_label, quit_label) = labels(l);
    let status = MenuItemBuilder::with_id("status", &head).enabled(false).build(app)?;
    let detail_item = MenuItemBuilder::with_id("detail", &detail).enabled(false).build(app)?;
    let manifest = MenuItemBuilder::with_id("manifest", manifest_menu(l, false).0).enabled(false).build(app)?;
    let open = MenuItemBuilder::with_id("open", &open_label).build(app)?;
    let quit = MenuItemBuilder::with_id("quit", quit_label).build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&status)
        .item(&detail_item)
        .item(&manifest)
        .separator()
        .item(&open)
        .item(&quit)
        .build()?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip(&head)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "manifest" => open_for_manifest(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    *ITEMS.lock().unwrap() = Some(Items { status, detail: detail_item, manifest, open, quit });
    Ok(())
}

/// Turn the guard on or off: the icon, the hidden-close behaviour and the loop follow.
pub fn set_running(app: &AppHandle, on: bool) {
    if on {
        let cfg = app.state::<AppState>().config.lock().unwrap().clone();
        if let Err(e) = build_tray(app, &cfg) {
            warn!("service de téléchargement : icône impossible ({e})");
            return;
        }
        ON.store(true, Ordering::SeqCst);
        if !LOOP_STARTED.swap(true, Ordering::SeqCst) {
            let app = app.clone();
            tauri::async_runtime::spawn(run(app));
        }
    } else {
        ON.store(false, Ordering::SeqCst);
        app.remove_tray_by_id(TRAY_ID);
        *ITEMS.lock().unwrap() = None;
    }
}

/// The window's close button: hide it when the guard runs. Said once per session.
pub fn on_close_requested(app: &AppHandle) -> bool {
    if !is_on() {
        return false;
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    if !HINTED.swap(true, Ordering::SeqCst) {
        let cfg = app.state::<AppState>().config.lock().unwrap().clone();
        let fr = lang(&cfg) == "fr";
        let (title, body) = if fr {
            (
                format!("{} reste actif en arrière-plan", notify::display_name()),
                "Il surveille vos téléchargements. L'icône près de l'horloge permet de rouvrir l'application ou de la quitter.".to_string(),
            )
        } else {
            (
                format!("{} keeps running in the background", notify::display_name()),
                "It watches your downloads. The icon next to the clock reopens the application or quits it.".to_string(),
            )
        };
        let xml = notify::toast_xml(&title, &body, "", &[]);
        let _ = notify::show_toast_tagged(&xml, "download-guard-hint", false);
    }
    true
}

fn refresh_tray(app: &AppHandle, l: &str, view: &View, last: &mut (String, String)) {
    let (head, detail) = tray_lines(l, view);
    if (head.as_str(), detail.as_str()) == (last.0.as_str(), last.1.as_str()) {
        return;
    }
    if let Some(items) = ITEMS.lock().unwrap().as_ref() {
        let (open_label, quit_label) = labels(l);
        let _ = items.status.set_text(&head);
        let _ = items.detail.set_text(&detail);
        let _ = items.open.set_text(&open_label);
        let _ = items.quit.set_text(quit_label);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(&head));
    }
    *HEAD.lock().unwrap() = head.clone();
    *last = (head, detail);
}

/// Keep the missing-file entry and the tooltip in step with the pending list. Cheap: it
/// only touches the menu when what it would show changed.
fn refresh_manifest(app: &AppHandle, l: &str, last: &mut (bool, String, String)) {
    let pending = !manifest_fallback::pending().is_empty();
    let head = HEAD.lock().unwrap().clone();
    if (pending, head.as_str(), l) == (last.0, last.1.as_str(), last.2.as_str()) {
        return;
    }
    let (label, hint) = manifest_menu(l, pending);
    if let Some(items) = ITEMS.lock().unwrap().as_ref() {
        let _ = items.manifest.set_text(label);
        let _ = items.manifest.set_enabled(pending);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let tip = match hint {
            Some(h) => format!("{head}\n{h}"),
            None => head.clone(),
        };
        let _ = tray.set_tooltip(Some(&tip));
    }
    *last = (pending, head, l.to_string());
}

async fn run(app: AppHandle) {
    let t0 = Instant::now();
    let mut guard = Guard::default();
    let mut tail: Option<Tail> = None;
    // The source the incident started on: where a lost round comes back to.
    let mut first_source: Option<String> = None;
    let mut last_shown = (String::new(), String::new());
    let mut last_manifest = (false, String::new(), String::new());
    let mut n: u64 = 0;
    info!("service de téléchargement : surveillance démarrée");
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        n += 1;
        let now = t0.elapsed().as_secs() as i64;
        if !is_on() {
            guard = Guard::default();
            tail = None;
            continue;
        }
        let cfg = app.state::<AppState>().config.lock().unwrap().clone();
        let l = lang(&cfg);
        let steam = cfg.steam_dir.clone().or_else(detect::detect_steam_path);
        // Without SteamTools a failed download is an ordinary network problem, and
        // changing the source of a tool that is not there would be a lie.
        let watching = steam.as_deref().filter(|s| detect::inspect_steamtools(s).installed);
        let Some(steam) = watching else {
            tail = None;
            guard = Guard::default();
            refresh_tray(&app, l, &guard.view(now), &mut last_shown);
            continue;
        };
        let log = steam.join("logs").join("content_log.txt");
        if tail.as_ref().map(|t| t.path() != log.as_path()).unwrap_or(true) {
            tail = Some(Tail::new(log));
        }
        if n.is_multiple_of(2) {
            let lines = tail.as_mut().map(Tail::poll).unwrap_or_default();
            for line in lines {
                if let Some(missing) = manifest_fallback::parse_missing(&line) {
                    on_missing(&app, l, steam, missing).await;
                }
                let Some(event) = parse_event(&line) else { continue };
                let was_online = guard.view(now).status == crate::source_guard::Status::Online;
                if was_online {
                    first_source = Some(steamtools_source::current(steam));
                }
                let outcome = guard.on_event(event, now);
                if outcome.switch != Switch::No {
                    let current = steamtools_source::current(steam);
                    let target = match outcome.switch {
                        Switch::Next => steamtools_source::next_source(&current).to_string(),
                        _ => first_source.clone().unwrap_or_else(|| steamtools_source::DEFAULT_SOURCE.to_string()),
                    };
                    let dir = steam.to_path_buf();
                    let t = target.clone();
                    let done = tauri::async_runtime::spawn_blocking(move || steamtools_source::apply(&dir, &t)).await;
                    match done {
                        Ok(Ok(())) => info!("service de téléchargement : changement de serveur effectué"),
                        Ok(Err(e)) => warn!("service de téléchargement : changement impossible ({e})"),
                        Err(e) => warn!("service de téléchargement : changement interrompu ({e})"),
                    }
                }
                if let Some(note) = outcome.note {
                    let (title, body) = note_texts(l, note);
                    let xml = notify::toast_xml(&title, &body, "", &[]);
                    let silent = !outcome.loud;
                    let _ = tauri::async_runtime::spawn_blocking(move || notify::show_toast_tagged(&xml, TOAST_TAG, silent)).await;
                }
            }
        }
        guard.tick(now);
        refresh_tray(&app, l, &guard.view(now), &mut last_shown);
        refresh_manifest(&app, l, &mut last_manifest);
    }
}

const MANIFEST_TAG: &str = "download-guard-manifest";

fn manifest_texts(lang: &str, fetched: bool) -> (String, String) {
    let name = notify::display_name();
    match (lang == "fr", fetched) {
        (true, true) => ("Fichier de jeu récupéré".into(), "Steam va réessayer tout seul d'ici une minute.".into()),
        (false, true) => ("Game file retrieved".into(), "Steam will retry on its own within a minute.".into()),
        (true, false) => (
            "Un fichier de jeu manque".into(),
            format!("Steam n'arrive pas à l'obtenir. Cliquez ici : la page d'une clé gratuite s'ouvre, puis collez la clé dans {name}."),
        ),
        (false, false) => (
            "A game file is missing".into(),
            format!("Steam cannot get it. Click here: the page for a free key opens, then paste the key into {name}."),
        ),
    }
}

/// The click on the "a file is missing" toast, or on the tray entry: open the page where the
/// user gets a key and show the window, on the tools tab, where the key is pasted. The
/// verification on the page is theirs to pass; nothing here touches it.
fn open_for_manifest(app: &AppHandle) {
    if let Err(e) = crate::commands::open_manifest_page_in_browser("manifesthub") {
        warn!("service de téléchargement : page de clé non ouverte ({e})");
    }
    show_main(app);
    let _ = app.emit("manifest-fallback://open", ());
}

/// Steam was refused a manifest request code. With a key on hand the file is fetched at once
/// and silently; without one the user is told, once, and the click on the notification takes
/// them to the page that hands out a key.
async fn on_missing(app: &AppHandle, l: &str, steam: &std::path::Path, m: manifest_fallback::Missing) {
    // Steam writes the refusal twice per attempt: once the file is there, stay quiet.
    if manifest_fallback::depotcache_path(steam, m.depot_id, m.gid).exists() {
        manifest_fallback::forget(&m);
        return;
    }
    let first = manifest_fallback::note_missing(m);
    let keys = manifest_fallback::load_keys(&config::data_dir());
    let now = chrono::Utc::now().timestamp();
    let mut fetched = false;
    if keys.any_usable(now) {
        let state = app.state::<AppState>();
        fetched = crate::commands::fetch_pending(&state.fallback_http, steam, &keys, vec![m], now).await.is_empty();
    }
    // The page learns at once, not at its next poll: a hidden window may not poll at all.
    let _ = app.emit("manifest-fallback://changed", ());
    if fetched || first {
        let (title, body) = manifest_texts(l, fetched);
        let xml = notify::toast_xml(&title, &body, "", &[]);
        let click_app = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            if fetched {
                notify::show_toast_tagged(&xml, MANIFEST_TAG, true)
            } else {
                notify::show_toast_clickable(&xml, MANIFEST_TAG, false, move || open_for_manifest(&click_app))
            }
        })
        .await;
    }
}

/// The logon entry of the resident process, for the installed edition only. A portable
/// folder may move, a debug binary is a bench sandbox: neither is written to the registry.
pub fn sync_autostart(enabled: bool) {
    if config::is_portable() || cfg!(debug_assertions) {
        return;
    }
    if enabled {
        if let Ok(exe) = std::env::current_exe() {
            if let Err(e) = notify::set_guard_autostart(&exe, true).and_then(|_| notify::ensure_toast_identity()) {
                warn!("service de téléchargement : démarrage automatique impossible ({e})");
            }
        }
    } else {
        let _ = notify::set_guard_autostart(std::path::Path::new(""), false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tray_entry_speaks_both_languages_and_only_invites_when_a_file_is_missing() {
        for lang in ["fr", "en"] {
            let (idle, idle_hint) = manifest_menu(lang, false);
            let (busy, busy_hint) = manifest_menu(lang, true);
            assert_ne!(idle, busy);
            assert!(idle_hint.is_none(), "nothing to add to the tooltip when nothing is missing");
            assert!(busy_hint.is_some_and(|h| !h.is_empty()));
            for text in [idle, busy, busy_hint.unwrap()] {
                assert!(!text.contains('\u{2014}'), "no em dash in a user text: {text}");
                for provider in ["ManifestHub", "Hubcap", "wudrm", "steamrun"] {
                    assert!(!text.to_lowercase().contains(&provider.to_lowercase()), "{text} names {provider}");
                }
            }
        }
        assert!(manifest_menu("fr", true).0.contains("fichier de jeu"));
    }

    #[test]
    fn the_toast_texts_name_no_provider_and_no_dash() {
        for lang in ["fr", "en"] {
            for fetched in [false, true] {
                let (title, body) = manifest_texts(lang, fetched);
                for text in [title, body] {
                    assert!(!text.contains('\u{2014}'), "{text}");
                    for provider in ["manifesthub", "hubcap", "wudrm", "steamrun"] {
                        assert!(!text.to_lowercase().contains(provider), "{text} names {provider}");
                    }
                }
            }
        }
    }
}

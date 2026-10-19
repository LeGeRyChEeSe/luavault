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
use tauri::{AppHandle, Manager, Wry};

use crate::config::{self, AppConfig};
use crate::source_guard::{note_texts, parse_event, tray_lines, Guard, Switch, Tail, View};
use crate::{detect, notify, steamtools_source, AppState};

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
    open: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

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
    let open = MenuItemBuilder::with_id("open", &open_label).build(app)?;
    let quit = MenuItemBuilder::with_id("quit", quit_label).build(app)?;
    let menu = MenuBuilder::new(app).item(&status).item(&detail_item).separator().item(&open).item(&quit).build()?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip(&head)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
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
    *ITEMS.lock().unwrap() = Some(Items { status, detail: detail_item, open, quit });
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
    *last = (head, detail);
}

async fn run(app: AppHandle) {
    let t0 = Instant::now();
    let mut guard = Guard::default();
    let mut tail: Option<Tail> = None;
    // The source the incident started on: where a lost round comes back to.
    let mut first_source: Option<String> = None;
    let mut last_shown = (String::new(), String::new());
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

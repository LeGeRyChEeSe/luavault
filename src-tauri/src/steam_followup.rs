//! What follows a click on « Trouver sur le site » (LuaVault dans Steam).
//!
//! The person downloaded the game's files from the site; the downloads detection saw them arrive.
//! For a game asked for from Steam in the last 30 minutes (`steam_store::requested`) LuaVault does not
//! ask again (the click in Steam was the question):
//!
//! - a `.lua` is imported (`commands::import_lua_path`, the window's own import) and put where Steam
//!   reads it (`library::copy_to_steam`); the panel says how Steam holds the game and offers
//!   « Installer » when Steam does not have it;
//! - a patch archive is imported (`commands::import_patch_archive_inner`) and applied at once when the
//!   game is installed (`commands::install_fix_inner`); otherwise it waits in `steam-pending.json`
//!   and the watcher applies it once Steam has finished installing the game and it is not running.
//!
//! Anything that cannot be done this way goes back to the ordinary offer of the downloads popup.

use std::path::{Path, PathBuf};
use std::time::Duration;

use log::{info, warn};
use tauri::{AppHandle, Emitter, Manager};

use crate::archive::Probe;
use crate::downloads::DownloadFound;
use crate::steam_client::{self, Job};
use crate::steam_store::{self, SteamSide};
use crate::{config, vdf};

/// Told to the window: the library changed behind its back.
pub const LIBRARY_CHANGED: &str = "library://changed";
const PENDING_FILE: &str = "steam-pending.json";
/// How often the watcher looks at what waits for Steam.
pub const WATCH_SECS: u64 = 20;
/// At most this many patches wait.
pub const MAX_PENDING: usize = 32;

/// Pure. Whether a found file can be taken without asking: a `.lua`, or a patch archive that opens
/// (or could not be checked) and does not replace a stored one.
pub fn takes_silently(item: &DownloadFound) -> bool {
    match item.kind.as_str() {
        "lua" => true,
        _ => {
            !item.replaces
                && !matches!(item.archive_check, Some(Probe::NeedsPassword) | Some(Probe::WrongPassword) | Some(Probe::Unreadable))
        }
    }
}

/// Pure. The files found, split into the ones of games asked for from Steam (taken now, `.lua`
/// first so a patch finds its game) and the rest (offered as usual).
pub fn split_requested(
    found: Vec<DownloadFound>,
    wanted: bool,
    is_requested: impl Fn(&str) -> bool,
) -> (Vec<DownloadFound>, Vec<DownloadFound>) {
    if !wanted {
        return (Vec::new(), found);
    }
    let (mut taken, rest): (Vec<_>, Vec<_>) = found.into_iter().partition(|item| {
        item.app_id.as_deref().is_some_and(|id| steam_store::parse_app_id(id).is_some() && is_requested(id)) && takes_silently(item)
    });
    taken.sort_by_key(|item| item.kind != "lua");
    (taken, rest)
}

fn say(lang: &str, fr: String, en: String) -> String {
    if lang == "fr" { fr } else { en }
}

/// Pure. The panel's line once the `.lua` is in Steam's folder.
pub fn lua_line(lang: &str, side: SteamSide) -> (&'static str, String, bool) {
    match side {
        SteamSide::Ready => ("ok", say(lang, "Ajouté à LuaVault et placé dans Steam. Le jeu est installé.".into(), "Added to LuaVault and placed in Steam. The game is installed.".into()), false),
        SteamSide::Downloading => ("ok", say(lang, "Ajouté à LuaVault et placé dans Steam. Steam télécharge le jeu.".into(), "Added to LuaVault and placed in Steam. Steam is downloading the game.".into()), false),
        _ => (
            "ok",
            say(lang, "Ajouté à LuaVault et placé dans Steam : cliquez sur Installer.".into(), "Added to LuaVault and placed in Steam: click Install.".into()),
            true,
        ),
    }
}

/// Pure. How a found file is named: `lua`, `patch` or `bypass` (`commands::patch_flavor`).
pub fn noun(lang: &str, flavor: &str) -> &'static str {
    match (lang == "fr", flavor) {
        (true, "lua") => "Le .lua",
        (true, "bypass") => "Contournement DRM",
        (true, _) => "Correctif",
        (false, "lua") => "The .lua",
        (false, "bypass") => "DRM bypass",
        (false, _) => "Patch",
    }
}

/// Pure. The panel's line once a patch archive is in the library.
pub fn patch_line(lang: &str, flavor: &str, applied: Option<Result<usize, String>>) -> (&'static str, String) {
    let what = noun(lang, flavor);
    match applied {
        Some(Ok(n)) => ("ok", say(lang, format!("{what} appliqué ({n} fichier(s)). Le jeu est prêt."), format!("{what} applied ({n} file(s)). The game is ready."))),
        Some(Err(e)) => ("error", say(lang, format!("{what} ajouté mais non appliqué : {e}"), format!("{what} added but not applied: {e}"))),
        None => ("busy", say(lang, format!("{what} ajouté : il s'appliquera dès que Steam aura installé le jeu."), format!("{what} added: it will be applied as soon as Steam has installed the game."))),
    }
}

/// Pure. The title of the card shown in Steam when LuaVault detected a game's file by itself.
pub fn detected_title(lang: &str, name: &str) -> String {
    say(lang, format!("Détection automatique : {name}"), format!("Detected automatically: {name}"))
}

/// Pure. The card's line for files found in Downloads that LuaVault does not take by itself: what
/// was found, and that a click opens LuaVault to add it.
pub fn found_line(lang: &str, flavors: &[&str]) -> String {
    let kind = |f: &str| match f {
        "lua" | "bypass" => f.to_string(),
        _ => "patch".to_string(),
    };
    let mut names: Vec<&str> = Vec::new();
    for flavor in ["lua", "patch", "bypass"] {
        if flavors.iter().any(|f| kind(f) == flavor) {
            names.push(match (lang == "fr", flavor) {
                (true, "lua") => "le .lua",
                (true, "bypass") => "le contournement DRM",
                (true, _) => "le correctif",
                (false, "lua") => "the .lua",
                (false, "bypass") => "the DRM bypass",
                (false, _) => "the patch",
            });
        }
    }
    let several = names.len() > 1;
    let joined = match names.as_slice() {
        [] => say(lang, "un fichier".into(), "a file".into()),
        [one] => one.to_string(),
        [rest @ .., last] => say(lang, format!("{} et {last}", rest.join(", ")), format!("{} and {last}", rest.join(", "))),
    };
    let mut first = joined.chars();
    let joined = first.next().map(|c| c.to_uppercase().collect::<String>() + first.as_str()).unwrap_or_default();
    let (fr, en) = if several { ("les", "them") } else { ("l'", "it") };
    say(
        lang,
        format!("{joined} dans vos Téléchargements. Cliquez pour {fr}{}ajouter avec LuaVault.", if several { " " } else { "" }),
        format!("{joined} in your Downloads. Click to add {en} with LuaVault."),
    )
}

// ------------------------------------------------------------ patches waiting for Steam

pub fn pending_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PENDING_FILE)
}

/// Pure. Strict AppIDs, no twice the same, `MAX_PENDING` at most (the newest).
pub fn sanitize_pending(list: Vec<String>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for id in list {
        if steam_store::parse_app_id(&id).is_some() && !kept.contains(&id) {
            kept.push(id);
        }
    }
    let extra = kept.len().saturating_sub(MAX_PENDING);
    kept.drain(..extra);
    kept
}

pub fn load_pending(data_dir: &Path) -> Vec<String> {
    let raw = std::fs::read_to_string(pending_path(data_dir)).unwrap_or_default();
    sanitize_pending(serde_json::from_str(&raw).unwrap_or_default())
}

fn save_pending(data_dir: &Path, list: &[String]) {
    let path = pending_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    let write = std::fs::create_dir_all(data_dir)
        .and_then(|_| std::fs::write(&tmp, serde_json::to_vec(list).unwrap_or_default()))
        .and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = write {
        warn!("LuaVault dans Steam : liste des correctifs en attente non enregistrée ({e})");
    }
}

pub fn remember_pending(data_dir: &Path, app_id: &str) {
    let mut list = load_pending(data_dir);
    list.push(app_id.to_string());
    save_pending(data_dir, &sanitize_pending(list));
}

/// Take the entry out BEFORE applying it: a second watcher (another instance) never applies it twice.
fn claim_pending(data_dir: &Path, app_id: &str) -> bool {
    let list = load_pending(data_dir);
    if !list.iter().any(|id| id == app_id) {
        return false;
    }
    save_pending(data_dir, &list.into_iter().filter(|id| id != app_id).collect::<Vec<_>>());
    true
}

/// Pure. A waiting patch is applied once the game is fully installed and not running.
pub fn due(side: SteamSide, running: Option<u32>, app_id: &str) -> bool {
    side == SteamSide::Ready && running.map(|r| r.to_string()).as_deref() != Some(app_id)
}

// ------------------------------------------------------------ doing it

fn side_of(cfg: &config::AppConfig, app_id: &str) -> SteamSide {
    match crate::commands::resolve_steam(cfg) {
        Some(steam) => steam_store::steam_side(true, Some(&vdf::locate_game(&steam, app_id))),
        None => SteamSide::Unknown,
    }
}

async fn name_for(app: &AppHandle, app_id: &str, lang: &str) -> String {
    match steam_client::job_of(app_id) {
        Some(job) => job.name,
        None => steam_store::game_name(app, app_id, lang).await,
    }
}

/// The step goes to the store's panel and to a card in Steam (`title`), or to a notification of
/// Windows when no page of the client is equipped. The card is shown before the pages are equipped
/// again (`refresh`), so it reaches the pages attached now.
#[allow(clippy::too_many_arguments)] // one call per step, each argument a distinct fact of it
fn show(app: &AppHandle, app_id: &str, name: String, title: String, tone: &'static str, line: String, install: bool, library: bool) {
    let notice = steam_client::Notice {
        title: title.clone(),
        line: line.clone(),
        tone: steam_client::notice_tone(tone),
        app_id: Some(app_id.to_string()),
        open: false,
    };
    let in_steam = steam_client::notify_in_steam(&notice);
    steam_client::publish(Job { app_id: app_id.to_string(), name, tone, line: line.clone(), install, library, site: false, at: 0 });
    steam_client::refresh();
    let _ = app.emit(LIBRARY_CHANGED, ());
    if !in_steam {
        windows_toast(&title, &line);
    }
}

fn windows_toast(title: &str, line: &str) {
    let xml = crate::notify::toast_xml(title, &crate::notify::plain_text(line, 220), "", &[]);
    if let Err(e) = crate::notify::show_toast(&xml) {
        warn!("LuaVault dans Steam : notification impossible ({e})");
    }
}

/// Files found in Downloads that are not taken by themselves: one card per game in Steam, a click
/// opens LuaVault where the popup offers them. `false` when Steam could show nothing (the ordinary
/// notification of Windows then says it).
pub async fn announce_found(app: &AppHandle, items: &[DownloadFound]) -> bool {
    let cfg = app.state::<crate::AppState>().config.lock().map(|c| c.clone()).unwrap_or_default();
    if items.is_empty() || !steam_client::wanted(&cfg, config::is_portable(), config::EXPERIMENTAL) {
        return false;
    }
    if !steam_client::window_open() && !steam_client::pages_open() {
        return false;
    }
    let lang = steam_store::lang_of(&cfg);
    let mut games: Vec<(String, Vec<&str>)> = Vec::new();
    for item in items {
        let Some(id) = item.app_id.as_deref().and_then(steam_store::parse_app_id) else { continue };
        match games.iter_mut().find(|(g, _)| *g == id) {
            Some((_, flavors)) => flavors.push(item.flavor.as_str()),
            None => games.push((id, vec![item.flavor.as_str()])),
        }
    }
    let mut shown = false;
    for (app_id, flavors) in games {
        let name = name_for(app, &app_id, lang).await;
        let notice = steam_client::Notice {
            title: detected_title(lang, &name),
            line: found_line(lang, &flavors),
            tone: "info",
            app_id: Some(app_id),
            open: true,
        };
        shown |= steam_client::notify_in_steam(&notice);
    }
    shown
}

async fn apply_fix(cfg: &config::AppConfig, app_id: &str) -> Result<usize, String> {
    crate::commands::install_fix_inner(cfg, app_id, None).await.map(|report| report.file_count)
}

/// Take the files of games asked for from Steam. Returns what could not be taken, for the ordinary
/// offer of the downloads popup.
pub async fn take(app: &AppHandle, items: Vec<DownloadFound>) -> Vec<DownloadFound> {
    let mut left = Vec::new();
    for item in items {
        let Some(app_id) = item.app_id.clone() else {
            left.push(item);
            continue;
        };
        let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
        let lang = steam_store::lang_of(&cfg);
        if item.kind == "lua" {
            let imported = crate::commands::import_lua_path(&app.state::<crate::AppState>(), item.path.clone()).await;
            let placed = imported.and_then(|_| {
                let steam = crate::commands::resolve_steam(&cfg).ok_or("Steam introuvable".to_string())?;
                crate::library::copy_to_steam(&cfg.resolved_library_dir(), &app_id, &steam).map_err(|e| e.to_string())
            });
            let name = name_for(app, &app_id, lang).await;
            match placed {
                Ok(_) => {
                    info!("LuaVault dans Steam : {app_id} ajouté et placé dans Steam");
                    let id = app_id.clone();
                    let c = cfg.clone();
                    let side = tauri::async_runtime::spawn_blocking(move || side_of(&c, &id)).await.unwrap_or(SteamSide::Unknown);
                    let (tone, line, install) = lua_line(lang, side);
                    let title = detected_title(lang, &name);
                    show(app, &app_id, name, title, tone, line, install, true);
                }
                Err(e) => {
                    warn!("LuaVault dans Steam : {app_id} non ajouté ({e})");
                    let title = detected_title(lang, &name);
                    show(app, &app_id, name, title, "error", say(lang, format!("Le .lua n'a pas pu être ajouté : {e}"), format!("The .lua could not be added: {e}")), false, false);
                    left.push(item);
                }
            }
            continue;
        }
        let (path, lib, data) = (PathBuf::from(&item.path), cfg.resolved_library_dir(), config::data_dir());
        let id = app_id.clone();
        let imported = tauri::async_runtime::spawn_blocking(move || crate::commands::import_patch_archive_inner(&path, Some(&id), &lib, &data))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        if let Err(e) = imported {
            // Not this way (no game in the library yet, a name it cannot read): the popup offers it.
            info!("LuaVault dans Steam : correctif de {app_id} laissé à la fenêtre ({e})");
            left.push(item);
            continue;
        }
        let name = name_for(app, &app_id, lang).await;
        let (c, id) = (cfg.clone(), app_id.clone());
        let side = tauri::async_runtime::spawn_blocking(move || side_of(&c, &id)).await.unwrap_or(SteamSide::Unknown);
        let applied = if due(side, crate::motd_run::running_app_id(), &app_id) {
            Some(apply_fix(&cfg, &app_id).await)
        } else {
            remember_pending(&config::data_dir(), &app_id);
            None
        };
        let (tone, line) = patch_line(lang, &item.flavor, applied);
        let title = detected_title(lang, &name);
        show(app, &app_id, name, title, tone, line, false, side != SteamSide::Absent);
    }
    left
}

/// The watcher every instance runs: patches waiting for Steam are applied once their game is ready.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(WATCH_SECS)).await;
            let data_dir = config::data_dir();
            let waiting = load_pending(&data_dir);
            if waiting.is_empty() {
                continue;
            }
            let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
            let running = crate::motd_run::running_app_id();
            for app_id in waiting {
                let (c, id) = (cfg.clone(), app_id.clone());
                let side = tauri::async_runtime::spawn_blocking(move || side_of(&c, &id)).await.unwrap_or(SteamSide::Unknown);
                if !due(side, running, &app_id) || !claim_pending(&data_dir, &app_id) {
                    continue;
                }
                let lang = steam_store::lang_of(&cfg);
                let applied = apply_fix(&cfg, &app_id).await;
                info!("LuaVault dans Steam : correctif en attente de {app_id} appliqué : {}", applied.is_ok());
                let name = name_for(&app, &app_id, lang).await;
                let (tone, line) = patch_line(lang, "patch", Some(applied));
                let title = name.clone();
                show(&app, &app_id, name, title, tone, line, false, true);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(kind: &str, app_id: Option<&str>, check: Option<Probe>, replaces: bool) -> DownloadFound {
        DownloadFound {
            path: format!("C:\\Downloads\\{}.{kind}", app_id.unwrap_or("x")),
            name: "x".into(),
            size: 1,
            kind: kind.into(),
            app_id: app_id.map(str::to_string),
            flavor: kind.into(),
            replaces,
            archive_check: check,
        }
    }

    #[test]
    fn steam_followup_takes_only_what_was_asked_and_the_lua_first() {
        let items = vec![
            found("zip", Some("264710"), Some(Probe::Opens), false),
            found("lua", Some("264710"), None, false),
            found("lua", Some("848450"), None, false),
            found("lua", None, None, false),
        ];
        let (taken, rest) = split_requested(items.clone(), true, |id| id == "264710");
        assert_eq!(taken.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>(), vec!["lua", "zip"], "the .lua first");
        assert_eq!(rest.len(), 2, "another game and no AppID are offered as usual");
        let (taken, rest) = split_requested(items, false, |_| true);
        assert!(taken.is_empty() && rest.len() == 4, "the switch off takes nothing");
    }

    #[test]
    fn steam_followup_leaves_doubtful_archives_to_the_person() {
        assert!(takes_silently(&found("zip", Some("1"), Some(Probe::Opens), false)));
        assert!(takes_silently(&found("rar", Some("1"), Some(Probe::Unchecked), false)));
        assert!(takes_silently(&found("rar", Some("1"), None, false)));
        for check in [Probe::NeedsPassword, Probe::WrongPassword, Probe::Unreadable] {
            assert!(!takes_silently(&found("zip", Some("1"), Some(check), false)));
        }
        assert!(!takes_silently(&found("zip", Some("1"), Some(Probe::Opens), true)), "it would replace a stored patch");
    }

    #[test]
    fn steam_followup_waits_for_steam_and_for_the_game_to_close() {
        assert!(due(SteamSide::Ready, None, "264710"));
        assert!(due(SteamSide::Ready, Some(848450), "264710"));
        assert!(!due(SteamSide::Ready, Some(264710), "264710"), "the game is running");
        for side in [SteamSide::Downloading, SteamSide::Absent, SteamSide::Unknown] {
            assert!(!due(side, None, "264710"));
        }
    }

    #[test]
    fn steam_followup_pending_list_is_strict_and_claimed_once() {
        assert_eq!(sanitize_pending(vec!["1".into(), "0".into(), "1".into(), "x".into(), "2".into()]), vec!["1", "2"]);
        let many: Vec<String> = (1..=40).map(|i| i.to_string()).collect();
        assert_eq!(sanitize_pending(many).first().map(String::as_str), Some("9"), "the newest stay");
        let dir = std::env::temp_dir().join(format!("lv_pending_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        remember_pending(&dir, "264710");
        remember_pending(&dir, "264710");
        assert_eq!(load_pending(&dir), vec!["264710"]);
        assert!(claim_pending(&dir, "264710"));
        assert!(!claim_pending(&dir, "264710"), "claimed once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn steam_followup_cards_say_the_detection_and_what_was_found() {
        assert_eq!(detected_title("fr", "Stardew Valley"), "Détection automatique : Stardew Valley");
        assert_eq!(detected_title("en", "Stardew Valley"), "Detected automatically: Stardew Valley");
        assert_eq!(found_line("fr", &["lua"]), "Le .lua dans vos Téléchargements. Cliquez pour l'ajouter avec LuaVault.");
        assert_eq!(
            found_line("fr", &["patch", "lua", "bypass"]),
            "Le .lua, le correctif et le contournement DRM dans vos Téléchargements. Cliquez pour les ajouter avec LuaVault."
        );
        assert_eq!(found_line("en", &["bypass"]), "The DRM bypass in your Downloads. Click to add it with LuaVault.");
        assert_eq!(found_line("en", &["lua", "patch"]), "The .lua and the patch in your Downloads. Click to add them with LuaVault.");
        for lang in ["fr", "en"] {
            for flavors in [&["lua"][..], &["patch"], &["bypass"], &["lua", "patch", "bypass"]] {
                assert!(!found_line(lang, flavors).contains('\u{2014}'));
            }
        }
    }

    #[test]
    fn steam_followup_lines_offer_install_only_when_steam_lacks_the_game() {
        assert!(lua_line("fr", SteamSide::Absent).2);
        assert!(!lua_line("fr", SteamSide::Ready).2);
        assert!(!lua_line("en", SteamSide::Downloading).2);
        assert_eq!(patch_line("fr", "patch", None).0, "busy");
        assert_eq!(patch_line("en", "patch", Some(Ok(3))).0, "ok");
        assert_eq!(patch_line("fr", "patch", Some(Err("x".into()))).0, "error");
        assert!(patch_line("fr", "bypass", Some(Ok(2))).1.starts_with("Contournement DRM appliqué"));
        assert!(patch_line("en", "patch", None).1.starts_with("Patch added"));
        for lang in ["fr", "en"] {
            for side in [SteamSide::Ready, SteamSide::Downloading, SteamSide::Absent] {
                assert!(!lua_line(lang, side).1.contains('\u{2014}'));
            }
        }
    }
}

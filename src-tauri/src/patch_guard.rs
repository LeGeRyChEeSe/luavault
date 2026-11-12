//! Online fixes put back after Steam updates a game.
//!
//! A Steam update of a game rewrites its files and often takes a fix's files with it: the fix stops
//! working with no word said (`fixes::verify` then reads `Damaged`). Every instance watches the games
//! that carry a fix: when Steam's build of one changes (`buildid` of its appmanifest) and the game is
//! fully installed and not running, the fix is checked, and a damaged one is installed again from the
//! archive kept beside the library, the way the window's button does it. A notification says so.
//!
//! The builds seen are kept in `patch-builds.json` (data dir), so a change made while no instance ran
//! is caught at the next start.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use log::{info, warn};
use tauri::{AppHandle, Manager};

use crate::fixes::FixHealth;

pub const STATE_FILE: &str = "patch-builds.json";
/// A minute: Steam's updates are not a matter of seconds, and each look reads a few manifests.
const POLL_SECS: u64 = 60;
/// What a notification body may hold (`notify::plain_text` cuts there).
const TOAST_MAX_CHARS: usize = 220;

/// Pure. Whether this game's fix must be checked now: Steam's build differs from the one last seen
/// (or none was seen), the game is fully installed, and it is not running (its files are open). An
/// unknown build is not a change: nothing to compare with.
pub fn due(seen: Option<&str>, build: Option<&str>, fully_installed: bool, running: bool) -> bool {
    let Some(build) = build.filter(|b| !b.trim().is_empty()) else { return false };
    fully_installed && !running && seen != Some(build)
}

/// Pure. Whether a checked fix must be installed again: its files are missing or were replaced. A
/// game that moved is the person's business (the window offers it), not a repair to run alone.
pub fn needs_repair(health: FixHealth) -> bool {
    health == FixHealth::Damaged
}

fn load(data_dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(data_dir.join(STATE_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(data_dir: &Path, seen: &BTreeMap<String, String>) {
    let path = data_dir.join(STATE_FILE);
    let tmp = path.with_extension("json.tmp");
    let Ok(text) = serde_json::to_string_pretty(seen) else { return };
    if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// What one look did: the fixes put back, and the ones that could not be.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pass {
    pub repaired: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// One look at every game with a fix. The repair goes through the window's own install path.
pub async fn look(cfg: &crate::config::AppConfig, data_dir: &Path) -> Pass {
    let mut pass = Pass::default();
    let Some(steam) = cfg.steam_dir.clone().or_else(crate::detect::detect_steam_path) else { return pass };
    let lib = cfg.resolved_library_dir();
    let running = crate::motd_run::running_app_id();
    let mut seen = load(data_dir);
    let mut changed = false;
    for app_id in crate::fixes::installed_app_ids(&lib) {
        let game = crate::vdf::locate_game(&steam, &app_id);
        let this_one_runs = running.is_some_and(|r| r.to_string() == app_id);
        if !due(seen.get(&app_id).map(String::as_str), game.build_id.as_deref(), game.fully_installed, this_one_runs) {
            continue;
        }
        let build = game.build_id.clone().unwrap_or_default();
        let dir = game.install_dir.clone().map(std::path::PathBuf::from);
        let (lib2, id2) = (lib.clone(), app_id.clone());
        let Ok(report) = tauri::async_runtime::spawn_blocking(move || crate::fixes::verify(&lib2, &id2, dir.as_deref())).await
        else {
            continue;
        };
        if needs_repair(report.health) {
            info!("correctifs : {app_id} abîmé après la mise à jour du jeu (build {build}), remise en place");
            match crate::commands::install_fix_inner(cfg, &app_id, None).await {
                Ok(r) if r.health == FixHealth::Healthy => pass.repaired.push(app_id.clone()),
                Ok(_) => pass.failed.push((app_id.clone(), "fichiers toujours différents".to_string())),
                Err(e) => pass.failed.push((app_id.clone(), e)),
            }
        }
        // Recorded whatever the outcome: a failed repair is said once, not every minute.
        seen.insert(app_id, build);
        changed = true;
    }
    if changed {
        save(data_dir, &seen);
    }
    pass
}

fn game_name(cfg: &crate::config::AppConfig, app_id: &str) -> String {
    crate::library::load_index(&cfg.resolved_library_dir())
        .into_iter()
        .find(|e| e.app_id == app_id)
        .map(|e| e.name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| app_id.to_string())
}

/// Pure. The words of a notification: one game, repaired or not.
pub fn text(name: &str, ok: bool, lang: &str) -> (&'static str, String) {
    match (ok, lang == "fr") {
        (true, true) => ("Correctif reposé", format!("{name} : Steam a mis le jeu à jour et retiré le correctif, il est de nouveau en place.")),
        (true, false) => ("Fix put back", format!("{name}: Steam updated the game and removed the fix; it is back in place.")),
        (false, true) => ("Correctif à reprendre", format!("{name} : Steam a mis le jeu à jour et retiré le correctif, qui n'a pas pu être reposé. Ouvrez l'application pour le réparer.")),
        (false, false) => ("Fix to redo", format!("{name}: Steam updated the game and removed the fix, which could not be put back. Open the app to repair it.")),
    }
}

fn announce(cfg: &crate::config::AppConfig, app_id: &str, ok: bool) {
    let lang = crate::notify::pick_language(cfg.locale.as_deref(), crate::notify::system_locale().as_deref());
    let (title, line) = text(&game_name(cfg, app_id), ok, lang);
    let xml = crate::notify::toast_xml(title, &crate::notify::plain_text(&line, TOAST_MAX_CHARS), "", &[]);
    if let Err(e) = crate::notify::show_toast(&xml) {
        warn!("correctifs : notification impossible ({e})");
    }
}

/// The watcher every instance runs.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // A first look soon after the start: Steam may have updated a game while nothing ran.
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
            let pass = look(&cfg, &crate::config::data_dir()).await;
            for app_id in &pass.repaired {
                announce(&cfg, app_id, true);
            }
            for (app_id, why) in &pass.failed {
                warn!("correctifs : {app_id} non reposé ({why})");
                announce(&cfg, app_id, false);
            }
            tokio::time::sleep(Duration::from_secs(POLL_SECS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_guard_checks_only_after_a_steam_update_of_an_idle_installed_game() {
        assert!(due(Some("100"), Some("101"), true, false), "Steam changed the build");
        assert!(due(None, Some("101"), true, false), "never seen: looked at once");
        assert!(!due(Some("101"), Some("101"), true, false), "same build: nothing to do");
        assert!(!due(Some("100"), Some("101"), false, false), "still downloading");
        assert!(!due(Some("100"), Some("101"), true, true), "the game runs: its files are open");
        assert!(!due(Some("100"), None, true, false), "unknown build: nothing to compare");
        assert!(!due(Some("100"), Some("  "), true, false));
    }

    #[test]
    fn patch_guard_repairs_only_a_damaged_fix() {
        assert!(needs_repair(FixHealth::Damaged));
        for h in [FixHealth::Healthy, FixHealth::NotInstalled, FixHealth::GameMoved] {
            assert!(!needs_repair(h), "{h:?}");
        }
    }

    #[test]
    fn patch_guard_says_what_happened_in_both_languages() {
        let (t, l) = text("Subnautica", true, "fr");
        assert_eq!(t, "Correctif reposé");
        assert!(l.contains("Subnautica"));
        let (t, l) = text("Subnautica", false, "en");
        assert_eq!(t, "Fix to redo");
        assert!(l.contains("Open the app"));
        for lang in ["fr", "en"] {
            for ok in [true, false] {
                let (t, l) = text("X", ok, lang);
                assert!(!t.contains('—') && !l.contains('—'), "no em dash");
                assert!(l.chars().count() <= TOAST_MAX_CHARS, "{l}");
            }
        }
    }

    #[test]
    fn patch_guard_keeps_the_builds_it_saw() {
        let dir = std::env::temp_dir().join(format!("lv-patch-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load(&dir).is_empty(), "nothing seen yet");
        let mut seen = BTreeMap::new();
        seen.insert("264710".to_string(), "123".to_string());
        save(&dir, &seen);
        assert_eq!(load(&dir), seen);
        std::fs::write(dir.join(STATE_FILE), "pas du json").unwrap();
        assert!(load(&dir).is_empty(), "an unreadable file is nothing seen");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

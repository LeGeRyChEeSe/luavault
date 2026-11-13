//! « Masquer » : hide everything LuaVault put into Steam, and put it all back in one click
//! (experimental edition, LuaVault dans Steam).
//!
//! Engaging (from the panel of the « LV » pill in the Steam client, or from « À l'essai »):
//!
//! - every `.lua` of the library that sits where SteamTools reads it moves to a vault in the data
//!   dir (`coffre\<appid>.lua`, the Steam copy byte for byte, copied then removed: the data dir and
//!   Steam are often on two drives);
//! - every installed patch is uninstalled, originals put back (`fixes::uninstall`); the archives
//!   stay in the library;
//! - the patches still waiting for Steam (`steam_followup`) are set aside, so nothing is applied
//!   behind the person's back, and files arriving in Downloads are not taken by themselves;
//! - nothing of ours is drawn in Steam's pages any more (`steam_client` reads [`engaged`]).
//!
//! The game folders are left as they are. Releasing (from the window only: once engaged, Steam
//! shows nothing of ours) brings the `.lua` back and hands every removed patch to the follow-up,
//! whose watcher applies each one as soon as Steam has the game.
//!
//! The state file is written **first** on engaging, so a crash halfway leaves a stash that says what
//! was already moved, and the pages stop being equipped before anything is touched.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{detect, fixes, library, steam_followup};

/// In the data dir: present = engaged.
pub const STATE_FILE: &str = "kill-switch.json";
/// In the data dir: the `.lua` files taken out of Steam.
pub const VAULT_DIR: &str = "coffre";
/// The window listens to this to show or drop its notice.
pub const CHANGED_EVENT: &str = "kill-switch://changed";

/// What an engaged switch holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stash {
    /// When it was engaged (Unix seconds).
    pub since: i64,
    /// AppIDs whose Steam copy is in the vault.
    pub luas: Vec<String>,
    /// AppIDs whose patch was taken off (or was waiting), to apply again.
    pub patches: Vec<String>,
}

pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STATE_FILE)
}

pub fn vault_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(VAULT_DIR)
}

/// Engaged = the state file exists, readable or not: an unreadable stash must still hide
/// everything rather than let the pages be equipped again.
pub fn engaged(data_dir: &Path) -> bool {
    state_path(data_dir).is_file()
}

pub fn load(data_dir: &Path) -> Option<Stash> {
    let raw = std::fs::read_to_string(state_path(data_dir)).ok()?;
    Some(serde_json::from_str(&raw).unwrap_or_default())
}

fn save(data_dir: &Path, stash: &Stash) -> Result<()> {
    std::fs::create_dir_all(data_dir).context("dossier des données")?;
    let tmp = data_dir.join(format!("{STATE_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(stash)?).context("écriture de l'état du masquage")?;
    std::fs::rename(&tmp, state_path(data_dir)).context("remplacement de l'état du masquage")?;
    Ok(())
}

fn add(list: &mut Vec<String>, id: &str) {
    if !list.iter().any(|x| x == id) {
        list.push(id.to_string());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EngageReport {
    /// `.lua` files moved to the vault by this run.
    pub luas: usize,
    /// Patches uninstalled by this run.
    pub patches: usize,
    /// One line per thing that could not be done (the stash says what was).
    pub failed: Vec<String>,
}

/// Engage, or engage again (what was placed meanwhile is added to the same stash).
pub fn engage(lib: &Path, data_dir: &Path, steam: Option<&Path>, now: i64) -> Result<EngageReport> {
    let mut stash = load(data_dir).unwrap_or(Stash { since: now, ..Default::default() });
    save(data_dir, &stash)?;
    let mut report = EngageReport::default();
    // Strict read: a damaged index cannot say which files are ours, so none is moved (the patches,
    // which the fix states name, are still taken off).
    let entries = match library::load_index_with_data_dir(lib, data_dir) {
        Ok(entries) => entries,
        Err(e) => {
            report.failed.push(format!("bibliothèque illisible, .lua laissés dans Steam ({e:#})"));
            Vec::new()
        }
    };

    // What the follow-up still owed joins the stash: it is applied again on release.
    let owed = steam_followup::load_pending(data_dir);
    if !owed.is_empty() {
        for id in &owed {
            add(&mut stash.patches, id);
        }
        save(data_dir, &stash)?;
        steam_followup::save_pending(data_dir, &[]);
    }

    if let Some(steam) = steam {
        let vault = vault_dir(data_dir);
        std::fs::create_dir_all(&vault).context("création du coffre")?;
        let dir = detect::lua_dir(steam);
        for entry in &entries {
            let src = dir.join(library::lua_file_name(&entry.app_id));
            if !src.is_file() {
                continue;
            }
            let kept = vault.join(library::lua_file_name(&entry.app_id));
            match std::fs::copy(&src, &kept).and_then(|_| std::fs::remove_file(&src)) {
                Ok(()) => {
                    add(&mut stash.luas, &entry.app_id);
                    report.luas += 1;
                }
                Err(e) => report.failed.push(format!("{} : .lua non rangé ({e})", entry.name)),
            }
        }
        save(data_dir, &stash)?;
    }

    let mut ids = fixes::installed_app_ids(lib);
    ids.sort();
    for id in ids {
        match fixes::uninstall(lib, &id) {
            Ok(_) => {
                add(&mut stash.patches, &id);
                report.patches += 1;
            }
            Err(e) => report.failed.push(format!("{id} : correctif non retiré ({e:#})")),
        }
    }
    save(data_dir, &stash)?;
    Ok(report)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReleaseReport {
    /// `.lua` files back where SteamTools reads them.
    pub luas: usize,
    /// Games whose patch was handed to the follow-up.
    pub patches_queued: usize,
    pub failed: Vec<String>,
    /// Something could not be put back: the switch stays engaged with what remains.
    pub still_engaged: bool,
}

/// Put everything back. A `.lua` that cannot be put back keeps the switch engaged with that file
/// alone in the stash, so a second click retries it and nothing is drawn in Steam meanwhile.
pub fn release(lib: &Path, data_dir: &Path, steam: Option<&Path>) -> Result<ReleaseReport> {
    let Some(stash) = load(data_dir) else { return Ok(ReleaseReport::default()) };
    let mut report = ReleaseReport::default();
    let vault = vault_dir(data_dir);
    let mut left = Vec::new();
    for id in &stash.luas {
        let Some(steam) = steam else {
            left.push(id.clone());
            continue;
        };
        match put_back(lib, &vault, steam, id) {
            Ok(()) => report.luas += 1,
            Err(e) => {
                report.failed.push(format!("{id} : .lua non remis ({e:#})"));
                left.push(id.clone());
            }
        }
    }
    if steam.is_none() && !left.is_empty() {
        report.failed.push("Steam introuvable : les .lua restent au coffre".to_string());
    }
    for id in &stash.patches {
        steam_followup::remember_pending(data_dir, id);
        report.patches_queued += 1;
    }
    if left.is_empty() {
        std::fs::remove_file(state_path(data_dir)).context("retrait de l'état du masquage")?;
    } else {
        save(data_dir, &Stash { since: stash.since, luas: left, patches: Vec::new() })?;
        report.still_engaged = true;
    }
    Ok(report)
}

/// One `.lua` back into Steam: the vault copy when there is one (written the way the SteamTools in
/// place wants it), else the library's. A file Steam already has again is left alone.
fn put_back(lib: &Path, vault: &Path, steam: &Path, app_id: &str) -> Result<()> {
    let name = library::lua_file_name(app_id);
    let kept = vault.join(&name);
    let flavor = detect::flavor(steam);
    let dst_dir = detect::lua_dir_for(steam, flavor);
    let dst = dst_dir.join(&name);
    if dst.is_file() {
        let _ = std::fs::remove_file(&kept);
        return Ok(());
    }
    if !kept.is_file() {
        library::copy_to_steam(lib, app_id, steam)?;
        return Ok(());
    }
    let bytes = std::fs::read(&kept).context("lecture du coffre")?;
    let bytes = match flavor {
        detect::Flavor::Classic => std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| library::without_main_key(text, app_id))
            .map(String::into_bytes)
            .unwrap_or(bytes),
        _ => bytes,
    };
    std::fs::create_dir_all(&dst_dir).context("dossier des .lua de Steam")?;
    let tmp = dst.with_extension("lua.tmp");
    std::fs::write(&tmp, &bytes).context("écriture dans Steam")?;
    std::fs::rename(&tmp, &dst).context("pose dans Steam")?;
    let _ = std::fs::remove_file(&kept);
    Ok(())
}

/// What the window shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    pub engaged: bool,
    pub since: Option<i64>,
    pub luas: usize,
    pub patches: usize,
}

pub fn status(data_dir: &Path) -> Status {
    match load(data_dir) {
        Some(s) => Status { engaged: true, since: Some(s.since), luas: s.luas.len(), patches: s.patches.len() },
        None => Status { engaged: engaged(data_dir), ..Default::default() },
    }
}

/// The sentence said after engaging.
pub fn engaged_text(report: &EngageReport, lang: &str) -> (&'static str, String) {
    let fr = lang == "fr";
    let title = if fr { "LuaVault masqué dans Steam" } else { "LuaVault hidden in Steam" };
    let mut body = if fr {
        format!(
            "{} jeu(x) rangé(s) au coffre, {} correctif(s) retiré(s), originaux remis. Tout se remet depuis l'onglet « À l'essai » de LuaVault.",
            report.luas, report.patches
        )
    } else {
        format!(
            "{} game(s) put in the vault, {} patch(es) removed, originals restored. Put it all back from LuaVault's « Trials » tab.",
            report.luas, report.patches
        )
    };
    if !report.failed.is_empty() {
        body.push_str(if fr { " Certains éléments n'ont pas pu être retirés : voir le journal." } else { " Some items could not be removed: see the log." });
    }
    (title, body)
}

/// The refusal while a game runs: its files are open, and taking a patch off under it would leave
/// the game half patched.
pub fn refusal(running_app_id: Option<u32>, lang: &str) -> Option<&'static str> {
    crate::motd_run::game_running(running_app_id).then_some(if lang == "fr" {
        "Un jeu est en cours dans Steam : fermez-le d'abord, puis recommencez. Rien n'a été modifié."
    } else {
        "A game is running in Steam: close it first, then try again. Nothing was changed."
    })
}

fn lang_of(app: &tauri::AppHandle) -> &'static str {
    use tauri::Manager;
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    crate::steam_store::lang_of(&cfg)
}

/// Engage on a blocking thread with this instance's settings, then tell the window.
pub async fn engage_now(app: &tauri::AppHandle) -> Result<EngageReport, String> {
    use tauri::{Emitter, Manager};
    if let Some(why) = refusal(crate::motd_run::running_app_id(), lang_of(app)) {
        return Err(why.to_string());
    }
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    let lib = cfg.resolved_library_dir();
    let steam = crate::commands::resolve_steam(&cfg);
    let data = crate::config::data_dir();
    let now = chrono::Utc::now().timestamp();
    let report = tauri::async_runtime::spawn_blocking(move || engage(&lib, &data, steam.as_deref(), now))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
    for line in &report.failed {
        log::warn!("masquage : {line}");
    }
    log::info!("masquage : {} .lua au coffre, {} correctif(s) retiré(s)", report.luas, report.patches);
    crate::steam_client::refresh();
    let _ = app.emit(CHANGED_EVENT, status(&crate::config::data_dir()));
    let _ = app.emit(steam_followup::LIBRARY_CHANGED, ());
    Ok(report)
}

/// The panel's « Masquer » in the Steam client: Steam is about to show nothing of ours, so the
/// answer is a notification of Windows.
pub async fn engage_from_client(app: &tauri::AppHandle) {
    let lang = lang_of(app);
    let (title, body) = match engage_now(app).await {
        Ok(report) => {
            let (title, body) = engaged_text(&report, lang);
            (title.to_string(), body)
        }
        Err(why) => ((if lang == "fr" { "Rien n'a été masqué" } else { "Nothing was hidden" }).to_string(), why),
    };
    let xml = crate::notify::toast_xml(&title, &crate::notify::plain_text(&body, 220), "", &[]);
    if let Err(e) = crate::notify::show_toast(&xml) {
        log::warn!("masquage : notification impossible ({e})");
    }
}

/// Put everything back; the follow-up's watcher applies the patches as soon as Steam has the games.
pub async fn release_now(app: &tauri::AppHandle) -> Result<ReleaseReport, String> {
    use tauri::{Emitter, Manager};
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    let lib = cfg.resolved_library_dir();
    let steam = crate::commands::resolve_steam(&cfg);
    let data = crate::config::data_dir();
    let report = tauri::async_runtime::spawn_blocking(move || release(&lib, &data, steam.as_deref()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
    for line in &report.failed {
        log::warn!("remise en place : {line}");
    }
    log::info!("remise en place : {} .lua remis, {} correctif(s) à reposer", report.luas, report.patches_queued);
    crate::steam_client::refresh();
    let _ = app.emit(CHANGED_EVENT, status(&crate::config::data_dir()));
    let _ = app.emit(steam_followup::LIBRARY_CHANGED, ());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lv_kill_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const LUA: &str = "addappid(2881650, 1, \"aaaa\")\naddappid(2881651, 1, \"bbbb\")\n";

    /// A library with one game copied into an OpenSteamTool Steam and its patch installed.
    fn world(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = scratch(tag);
        let (lib, data, steam, game) = (root.join("lib"), root.join("data"), root.join("steam"), root.join("game"));
        std::fs::create_dir_all(&steam).unwrap();
        std::fs::write(steam.join("OpenSteamTool.dll"), b"x").unwrap();
        std::fs::write(steam.join("xinput1_4.dll"), b"x").unwrap();
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("Game.exe"), b"original").unwrap();
        library::upsert_with_data_dir(&lib, &data, "2881650", "Content Warning", None, LUA.as_bytes()).unwrap();
        library::copy_to_steam(&lib, "2881650", &steam).unwrap();
        fixes::fake_fix_archive(&fixes::archive_path(&lib, "2881650"), &[("Game.exe", b"patched"), ("OnlineFix.ini", b"ini")]);
        fixes::install(&lib, "2881650", &game, Some("testpass")).unwrap();
        (root, lib, data, steam, game)
    }

    #[test]
    fn kill_switch_vaults_the_lua_takes_the_patch_off_and_keeps_the_game() {
        let _lock = library::cache_test_lock();
        let (root, lib, data, steam, game) = world("engage");
        let in_steam = detect::lua_dir(&steam).join("2881650.lua");
        assert!(in_steam.is_file());
        assert_eq!(std::fs::read(game.join("Game.exe")).unwrap(), b"patched");
        steam_followup::remember_pending(&data, "264710");

        let report = engage(&lib, &data, Some(&steam), 100).unwrap();
        assert_eq!((report.luas, report.patches), (1, 1), "{report:?}");
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(engaged(&data));
        assert!(!in_steam.exists(), "Steam no longer sees the game");
        assert_eq!(std::fs::read_to_string(vault_dir(&data).join("2881650.lua")).unwrap(), LUA, "the vault keeps the Steam copy");
        assert_eq!(std::fs::read(game.join("Game.exe")).unwrap(), b"original", "the original is back");
        assert!(!game.join("OnlineFix.ini").exists(), "the patch's own files are gone");
        assert!(game.is_dir(), "the game folder stays");
        assert!(fixes::archive_path(&lib, "2881650").is_file(), "the archive stays in the library");
        assert!(steam_followup::load_pending(&data).is_empty(), "nothing is applied behind the person's back");
        let s = status(&data);
        assert_eq!((s.engaged, s.luas, s.patches), (true, 1, 2));

        let again = engage(&lib, &data, Some(&steam), 200).unwrap();
        assert_eq!((again.luas, again.patches), (0, 0), "engaging again adds nothing twice");
        assert_eq!(load(&data).unwrap().since, 100, "the first engagement's date stays");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn kill_switch_release_puts_the_lua_back_and_queues_every_patch() {
        let _lock = library::cache_test_lock();
        let (root, lib, data, steam, _game) = world("release");
        steam_followup::remember_pending(&data, "264710");
        engage(&lib, &data, Some(&steam), 100).unwrap();

        let report = release(&lib, &data, Some(&steam)).unwrap();
        assert_eq!((report.luas, report.patches_queued, report.still_engaged), (1, 2, false), "{report:?}");
        assert!(!engaged(&data));
        assert_eq!(std::fs::read_to_string(detect::lua_dir(&steam).join("2881650.lua")).unwrap(), LUA);
        assert!(!vault_dir(&data).join("2881650.lua").exists(), "the vault is emptied");
        let mut owed = steam_followup::load_pending(&data);
        owed.sort();
        assert_eq!(owed, vec!["264710".to_string(), "2881650".to_string()], "the removed patch and what was owed before come back");
        assert_eq!(release(&lib, &data, Some(&steam)).unwrap(), ReleaseReport::default(), "nothing engaged, nothing done");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn kill_switch_release_without_steam_keeps_the_switch_on() {
        let _lock = library::cache_test_lock();
        let (root, lib, data, steam, _game) = world("nosteam");
        engage(&lib, &data, Some(&steam), 100).unwrap();
        let report = release(&lib, &data, None).unwrap();
        assert!(report.still_engaged && engaged(&data), "{report:?}");
        assert_eq!(load(&data).unwrap().luas, vec!["2881650".to_string()]);
        assert!(load(&data).unwrap().patches.is_empty(), "the patches were already handed over");
        let report = release(&lib, &data, Some(&steam)).unwrap();
        assert!(!report.still_engaged && !engaged(&data), "a second click finishes");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn kill_switch_release_into_the_classic_steamtools_without_the_main_key() {
        let _lock = library::cache_test_lock();
        let (root, lib, data, steam, _game) = world("classic");
        engage(&lib, &data, Some(&steam), 100).unwrap();
        std::fs::remove_file(steam.join("OpenSteamTool.dll")).unwrap();
        std::fs::write(steam.join("dwmapi.dll"), b"x").unwrap();
        release(&lib, &data, Some(&steam)).unwrap();
        let placed = std::fs::read_to_string(steam.join("config").join("stplug-in").join("2881650.lua")).unwrap();
        assert_eq!(placed, "addappid(2881650)\naddappid(2881651, 1, \"bbbb\")\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn kill_switch_unreadable_state_still_hides() {
        let data = scratch("unreadable");
        std::fs::write(state_path(&data), b"{not json").unwrap();
        assert!(engaged(&data) && status(&data).engaged);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn kill_switch_refuses_under_a_running_game_and_its_texts_fit() {
        assert!(refusal(Some(2881650), "fr").is_some());
        assert!(refusal(None, "en").is_none());
        let long = EngageReport { luas: 999, patches: 999, failed: vec!["x".into()] };
        for lang in ["fr", "en"] {
            let (_, body) = engaged_text(&long, lang);
            assert!(body.chars().count() <= 220, "{body}");
            assert!(!body.contains('\u{2014}'));
        }
    }
}

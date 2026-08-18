//! Granular teardown. Every switch is opt-in and previewable, and nothing here
//! ever touches `steamapps`, `userdata`, or Steam's login/config files —
//! games and user data always survive.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{backup, detect, fixes, i18n_log, library};

/// SteamTools' own payload inside the Steam folder.
const STEAMTOOLS_FILES: [&str; 2] = ["OpenSteamTool.dll", "xinput1_4.dll"];

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WipePlan {
    /// Remove only the `.lua` files this app put into `{Steam}\config\lua`.
    pub remove_managed_lua_from_steam: bool,
    /// Empty `{Steam}\config\lua` entirely, including files from other tools.
    pub remove_all_lua_from_steam: bool,
    /// Roll every installed online fix back to its pre-patch state.
    pub uninstall_online_fixes: bool,
    /// Delete the downloaded fix archives from the library.
    pub delete_fix_archives: bool,
    /// Delete the pre-patch backups (makes fix rollback impossible).
    pub delete_fix_backups: bool,
    /// Delete the `.lua` files and the index from the library.
    pub delete_library_lua: bool,
    /// Delete every app snapshot in `<data>\backups`.
    pub delete_app_backups: bool,
    /// Reset `config.json` (Steam/library folders, onboarding, sitekey).
    pub reset_app_config: bool,
    /// Remove the SteamTools DLLs from the Steam folder.
    pub remove_steamtools: bool,
    /// Remove the conflicting SteamProof/MFX artifacts.
    pub remove_steamtools_conflicts: bool,
    /// Remove the legacy `config\stplug-in` folder.
    pub remove_legacy_plugin_dir: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WipeLevel {
    /// Reversible, or only touches data this app can re-download.
    Safe,
    /// Changes the state of Steam or the games, but recoverable.
    Moderate,
    /// Loses data that cannot be recovered from within the app.
    Destructive,
}

#[derive(Debug, Clone, Serialize)]
pub struct WipeAction {
    pub id: String,
    pub level: WipeLevel,
    /// How many items this action would affect (0 = nothing to do).
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum WipeStepDetail {
    UninstallOnlineFixesOk { count: usize },
    UninstallOnlineFixesPartial { ok_count: usize, ignored: usize, problems: String },
    DeletedWithFailures { done: usize, failed: usize },
    DeletedFolder,
    DeletedFolderFailed { e: String },
    RemoveSteamtoolsLocked { done: usize, failed: usize },
    DeletedFiles { done: usize },
    SteamMissing,
    DeletedArchives { done: usize },
    DeletedBackups { done: usize },
    DeletedLuaAndIndex { done: usize },
    DeletedSnapshots { done: usize },
    ConfigReset,
    ConfigMissing,
}

#[derive(Debug, Clone, Serialize)]
pub struct WipeStep {
    pub id: String,
    pub ok: bool,
    pub detail: WipeStepDetail,
}

#[derive(Debug, Clone, Serialize)]
pub struct WipeReport {
    pub steps: Vec<WipeStep>,
    /// Set when files could not be deleted without administrator rights.
    pub needs_elevation: bool,
}

pub struct WipeContext<'a> {
    pub library_dir: &'a Path,
    pub data_dir: &'a Path,
    pub steam_dir: Option<&'a Path>,
}

/// The index seen by a wipe: best-effort like `library::load_index`, but
/// verified against the context's own `data_dir` — in production the two
/// are the same folder, in tests the context carries a scratch dir so the
/// real application data is never reached.
fn load_index_for_wipe(ctx: &WipeContext) -> Vec<library::LibraryEntry> {
    match library::load_index_with_data_dir(ctx.library_dir, ctx.data_dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("{}", i18n_log::i18n_log(format!("wipe: index de la bibliothèque ignoré: {e:#}"), "logs.wipe.library-index-ignored", &[("error", serde_json::json!(format!("{e:#}")))]));
            Vec::new()
        }
    }
}

fn count_files(dir: &Path, keep: impl Fn(&Path) -> bool) -> usize {
    std::fs::read_dir(dir)
        .map(|read| {
            read.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file() && keep(p))
                .count()
        })
        .unwrap_or(0)
}

fn is_lua(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("lua")) == Some(true)
}

/// Describe exactly what `plan` would do, without doing any of it.
pub fn preview(plan: &WipePlan, ctx: &WipeContext) -> Vec<WipeAction> {
    let mut actions = Vec::new();
    let entries = load_index_for_wipe(ctx);
    let steam_lua = ctx.steam_dir.map(detect::lua_dir);
    let fixes_dir = fixes::fixes_dir(ctx.library_dir);

    let mut push = |id: &str, level: WipeLevel, count: usize| {
        if count > 0 {
            actions.push(WipeAction {
                id: id.to_string(),
                level,
                count,
            });
        }
    };

    if plan.remove_managed_lua_from_steam {
        let count = steam_lua
            .as_ref()
            .map(|dir| {
                entries
                    .iter()
                    .filter(|e| dir.join(library::lua_file_name(&e.app_id)).is_file())
                    .count()
            })
            .unwrap_or(0);
        push("remove_managed_lua_from_steam", WipeLevel::Safe, count);
    }
    if plan.remove_all_lua_from_steam {
        let count = steam_lua.as_ref().map(|d| count_files(d, is_lua)).unwrap_or(0);
        push("remove_all_lua_from_steam", WipeLevel::Moderate, count);
    }
    if plan.uninstall_online_fixes {
        let count = fixes::installed_app_ids(ctx.library_dir).len();
        push("uninstall_online_fixes", WipeLevel::Moderate, count);
    }
    if plan.delete_fix_archives {
        let count = count_files(&fixes_dir, |p| !p.to_string_lossy().ends_with(".state.json"));
        push("delete_fix_archives", WipeLevel::Safe, count);
    }
    if plan.delete_fix_backups {
        let count = count_files(&fixes_dir.join("backups"), |_| true);
        push("delete_fix_backups", WipeLevel::Destructive, count);
    }
    if plan.delete_library_lua {
        push("delete_library_lua", WipeLevel::Moderate, entries.len());
    }
    if plan.delete_app_backups {
        let count = backup::list_snapshots(ctx.data_dir).len();
        push("delete_app_backups", WipeLevel::Destructive, count);
    }
    if plan.reset_app_config {
        push("reset_app_config", WipeLevel::Safe, 1);
    }
    if plan.remove_steamtools {
        let count = ctx
            .steam_dir
            .map(|s| STEAMTOOLS_FILES.iter().filter(|f| s.join(f).is_file()).count())
            .unwrap_or(0);
        push("remove_steamtools", WipeLevel::Moderate, count);
    }
    if plan.remove_steamtools_conflicts {
        let count = ctx
            .steam_dir
            .map(|s| detect::inspect_steamtools(s).conflicts.len())
            .unwrap_or(0);
        push("remove_steamtools_conflicts", WipeLevel::Safe, count);
    }
    if plan.remove_legacy_plugin_dir {
        let count = ctx
            .steam_dir
            .map(|s| usize::from(s.join("config").join("stplug-in").is_dir()))
            .unwrap_or(0);
        push("remove_legacy_plugin_dir", WipeLevel::Safe, count);
    }
    actions
}

fn delete_dir_contents(dir: &Path, keep: impl Fn(&Path) -> bool) -> (usize, usize) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    let (mut done, mut failed) = (0, 0);
    for path in read.filter_map(|e| e.ok()).map(|e| e.path()) {
        if !path.is_file() || !keep(&path) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => done += 1,
            Err(_) => failed += 1,
        }
    }
    (done, failed)
}

/// Run `plan`. Steps are ordered so that rollbacks happen before the data they
/// depend on is deleted.
pub fn execute(plan: &WipePlan, ctx: &WipeContext) -> WipeReport {
    let mut steps = Vec::new();
    let mut needs_elevation = false;
    let fixes_dir = fixes::fixes_dir(ctx.library_dir);

    let mut step = |id: &str, ok: bool, detail: WipeStepDetail| {
        steps.push(WipeStep {
            id: id.to_string(),
            ok,
            detail,
        });
    };

    // 1. Roll fixes back while their backups and states still exist.
    if plan.uninstall_online_fixes {
        let ids = fixes::installed_app_ids(ctx.library_dir);
        let mut ok_count = 0;
        let mut problems = Vec::new();
        for app_id in &ids {
            match fixes::uninstall(ctx.library_dir, app_id) {
                Ok(_) => ok_count += 1,
                Err(e) => {
                    // The game folder is gone — drop the record rather than block.
                    fixes::forget(ctx.library_dir, app_id);
                    problems.push(format!("{app_id}: {e}"));
                }
            }
        }
        let detail = if problems.is_empty() {
            WipeStepDetail::UninstallOnlineFixesOk { count: ok_count }
        } else {
            WipeStepDetail::UninstallOnlineFixesPartial {
                ok_count,
                ignored: problems.len(),
                problems: problems.join(" ; "),
            }
        };
        step("uninstall_online_fixes", true, detail);
    }

    // 2. Steam-side cleanup.
    if let Some(steam) = ctx.steam_dir {
        let lua_dir = detect::lua_dir(steam);

        if plan.remove_managed_lua_from_steam && !plan.remove_all_lua_from_steam {
            let entries = load_index_for_wipe(ctx);
            let (mut done, mut failed) = (0, 0);
            for entry in &entries {
                let path = lua_dir.join(library::lua_file_name(&entry.app_id));
                if !path.is_file() {
                    continue;
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => done += 1,
                    Err(_) => failed += 1,
                }
            }
            needs_elevation |= failed > 0;
            step(
                "remove_managed_lua_from_steam",
                failed == 0,
                WipeStepDetail::DeletedWithFailures { done, failed },
            );
        }
        if plan.remove_all_lua_from_steam {
            let (done, failed) = delete_dir_contents(&lua_dir, is_lua);
            needs_elevation |= failed > 0;
            step(
                "remove_all_lua_from_steam",
                failed == 0,
                WipeStepDetail::DeletedWithFailures { done, failed },
            );
        }
        if plan.remove_steamtools_conflicts {
            let conflicts = detect::inspect_steamtools(steam).conflicts;
            let (mut done, mut failed) = (0, 0);
            for rel in &conflicts {
                let path = steam.join(rel);
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                match result {
                    Ok(()) => done += 1,
                    Err(_) => failed += 1,
                }
            }
            needs_elevation |= failed > 0;
            step(
                "remove_steamtools_conflicts",
                failed == 0,
                WipeStepDetail::DeletedWithFailures { done, failed },
            );
        }
        if plan.remove_legacy_plugin_dir {
            let path = steam.join("config").join("stplug-in");
            let result = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                Ok(())
            };
            needs_elevation |= result.is_err();
            step(
                "remove_legacy_plugin_dir",
                result.is_ok(),
                match &result {
                    Ok(()) => WipeStepDetail::DeletedFolder,
                    Err(e) => WipeStepDetail::DeletedFolderFailed { e: e.to_string() },
                },
            );
        }
        if plan.remove_steamtools {
            let (mut done, mut failed) = (0, 0);
            for name in STEAMTOOLS_FILES {
                let path = steam.join(name);
                if !path.is_file() {
                    continue;
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => done += 1,
                    Err(_) => failed += 1,
                }
            }
            needs_elevation |= failed > 0;
            step(
                "remove_steamtools",
                failed == 0,
                if failed > 0 {
                    WipeStepDetail::RemoveSteamtoolsLocked { done, failed }
                } else {
                    WipeStepDetail::DeletedFiles { done }
                },
            );
        }
    } else if plan.remove_managed_lua_from_steam
        || plan.remove_all_lua_from_steam
        || plan.remove_steamtools
        || plan.remove_steamtools_conflicts
        || plan.remove_legacy_plugin_dir
    {
        step("steam_missing", false, WipeStepDetail::SteamMissing);
    }

    // 3. Local data.
    if plan.delete_fix_archives {
        let (done, failed) =
            delete_dir_contents(&fixes_dir, |p| !p.to_string_lossy().ends_with(".state.json"));
        step(
            "delete_fix_archives",
            failed == 0,
            WipeStepDetail::DeletedArchives { done },
        );
    }
    if plan.delete_fix_backups {
        let (done, failed) = delete_dir_contents(&fixes_dir.join("backups"), |_| true);
        step(
            "delete_fix_backups",
            failed == 0,
            WipeStepDetail::DeletedBackups { done },
        );
    }
    if plan.delete_library_lua {
        let (done, _) = delete_dir_contents(ctx.library_dir, is_lua);
        let _ = std::fs::remove_file(ctx.library_dir.join("index.json"));
        // Invalidate the in-memory index cache — the file is gone.
        library::clear_index_cache();
        step(
            "delete_library_lua",
            true,
            WipeStepDetail::DeletedLuaAndIndex { done },
        );
    }
    if plan.delete_app_backups {
        let (done, failed) = delete_dir_contents(&backup::backups_dir(ctx.data_dir), |_| true);
        step(
            "delete_app_backups",
            failed == 0,
            WipeStepDetail::DeletedSnapshots { done },
        );
    }
    if plan.reset_app_config {
        let result = std::fs::remove_file(ctx.data_dir.join("config.json"));
        step(
            "reset_app_config",
            true,
            match result {
                Ok(()) => WipeStepDetail::ConfigReset,
                Err(_) => WipeStepDetail::ConfigMissing,
            },
        );
    }

    WipeReport {
        steps,
        needs_elevation,
    }
}

/// Absolute paths a plan would never touch — surfaced in the UI as a guarantee.
pub fn protected_paths(steam: Option<&Path>) -> Vec<String> {
    let Some(steam) = steam else {
        return Vec::new();
    };
    ["steamapps", "userdata", "config\\loginusers.vdf", "config\\config.vdf"]
        .iter()
        .map(|rel| steam.join(rel).display().to_string())
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ast_wipe_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn seed(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let data = root.join("data");
        let lib = data.join("library");
        let steam = root.join("steam");
        std::fs::create_dir_all(detect::lua_dir(&steam)).unwrap();
        std::fs::create_dir_all(steam.join("steamapps").join("common").join("Game")).unwrap();
        std::fs::create_dir_all(steam.join("userdata")).unwrap();
        std::fs::create_dir_all(fixes::fixes_dir(&lib).join("backups")).unwrap();

        library::upsert_with_data_dir(&lib, &data, "42", "Test Game", None, b"-- lua").unwrap();
        std::fs::write(detect::lua_dir(&steam).join("42.lua"), b"-- lua").unwrap();
        std::fs::write(detect::lua_dir(&steam).join("999.lua"), b"-- foreign").unwrap();
        std::fs::write(fixes::fixes_dir(&lib).join("42_online_fix.rar"), b"x").unwrap();
        std::fs::write(steam.join("OpenSteamTool.dll"), b"x").unwrap();
        std::fs::write(steam.join("xinput1_4.dll"), b"x").unwrap();
        std::fs::write(steam.join("wtsapi32.dll"), b"conflict").unwrap();
        (lib, data, steam)
    }

    #[test]
    fn preview_counts_without_touching_anything() {
        let _lock = library::cache_test_lock();
        let root = scratch("preview");
        let (lib, data, steam) = seed(&root);
        let ctx = WipeContext {
            library_dir: &lib,
            data_dir: &data,
            steam_dir: Some(&steam),
        };
        let plan = WipePlan {
            remove_managed_lua_from_steam: true,
            delete_fix_archives: true,
            remove_steamtools: true,
            remove_steamtools_conflicts: true,
            ..Default::default()
        };
        let actions = preview(&plan, &ctx);
        let by_id = |id: &str| actions.iter().find(|a| a.id == id).unwrap().count;
        assert_eq!(by_id("remove_managed_lua_from_steam"), 1);
        assert_eq!(by_id("delete_fix_archives"), 1);
        assert_eq!(by_id("remove_steamtools"), 2);
        assert_eq!(by_id("remove_steamtools_conflicts"), 1);
        // Preview is read-only.
        assert!(detect::lua_dir(&steam).join("42.lua").is_file());
        assert!(steam.join("OpenSteamTool.dll").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn managed_wipe_spares_foreign_lua_games_and_user_data() {
        let _lock = library::cache_test_lock();
        let root = scratch("managed");
        let (lib, data, steam) = seed(&root);
        let ctx = WipeContext {
            library_dir: &lib,
            data_dir: &data,
            steam_dir: Some(&steam),
        };
        let report = execute(
            &WipePlan {
                remove_managed_lua_from_steam: true,
                ..Default::default()
            },
            &ctx,
        );
        assert!(report.steps.iter().all(|s| s.ok));
        assert!(!detect::lua_dir(&steam).join("42.lua").exists());
        assert!(detect::lua_dir(&steam).join("999.lua").is_file());
        assert!(steam.join("steamapps").join("common").join("Game").is_dir());
        assert!(steam.join("userdata").is_dir());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn full_wipe_clears_app_data_but_keeps_games() {
        let _lock = library::cache_test_lock();
        let root = scratch("full");
        let (lib, data, steam) = seed(&root);
        let ctx = WipeContext {
            library_dir: &lib,
            data_dir: &data,
            steam_dir: Some(&steam),
        };
        let plan = WipePlan {
            remove_all_lua_from_steam: true,
            uninstall_online_fixes: true,
            delete_fix_archives: true,
            delete_fix_backups: true,
            delete_library_lua: true,
            delete_app_backups: true,
            reset_app_config: true,
            remove_steamtools: true,
            remove_steamtools_conflicts: true,
            remove_legacy_plugin_dir: true,
            ..Default::default()
        };
        let report = execute(&plan, &ctx);
        assert!(report.steps.iter().all(|s| s.ok), "{:?}", report.steps);
        assert_eq!(count_files(&detect::lua_dir(&steam), is_lua), 0);
        assert_eq!(count_files(&lib, is_lua), 0);
        assert!(!steam.join("OpenSteamTool.dll").exists());
        assert!(!steam.join("wtsapi32.dll").exists());
        assert!(library::load_index_with_data_dir(&lib, &data).unwrap().is_empty());
        // The games themselves are untouched.
        assert!(steam.join("steamapps").join("common").join("Game").is_dir());
        assert!(steam.join("userdata").is_dir());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn steam_actions_report_clearly_when_steam_is_unknown() {
        let _lock = library::cache_test_lock();
        let root = scratch("nosteam");
        let (lib, data, _steam) = seed(&root);
        let ctx = WipeContext {
            library_dir: &lib,
            data_dir: &data,
            steam_dir: None,
        };
        let report = execute(
            &WipePlan {
                remove_steamtools: true,
                ..Default::default()
            },
            &ctx,
        );
        assert_eq!(report.steps.len(), 1);
        assert!(!report.steps[0].ok);
        assert!(matches!(
            report.steps[0].detail,
            WipeStepDetail::SteamMissing
        ));

        let _ = std::fs::remove_dir_all(&root);
    }
}





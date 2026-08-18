//! Library statistics — stage distribution, fix counts, and on-disk footprint.

use serde::Serialize;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::commands::GameStatus;
use crate::fixes;

/// Count of games in a given stage.
#[derive(Debug, Clone, Serialize)]
pub struct StageCount {
    pub stage: String,
    pub count: usize,
}

/// The most-played game of the library, among those with recorded data.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MostPlayedGame {
    pub app_id: String,
    pub name: String,
    pub minutes: u64,
}

/// Aggregate statistics for the whole library.
#[derive(Debug, Clone, Serialize)]
pub struct LibraryStats {
    pub total: usize,
    /// Entries hidden from the library view (still counted in `total`).
    pub hidden: usize,
    pub by_stage: Vec<StageCount>,
    pub fixes_installed: usize,
    pub fixes_downloaded: usize,
    /// Bytes taken by the `.lua` files and `index.json`.
    pub lua_bytes: u64,
    /// Bytes taken by the downloaded fix archives and their state files.
    pub fix_archive_bytes: u64,
    /// Bytes taken by the `.luabak` snapshots.
    pub backup_bytes: u64,
    /// Sum of `SizeOnDisk` for the games Steam reports as installed.
    pub games_on_disk_bytes: u64,
    /// LOT-13 — sum of every KNOWN playtime, in minutes. Games without data
    /// are excluded from the total, never counted as zero.
    pub playtime_total_minutes: u64,
    /// The most-played game among those with minutes > 0, if any.
    pub most_played: Option<MostPlayedGame>,
    /// Games with no readable playtime data — "on ne sait pas", reported as
    /// a count rather than folded into the total as zero minutes.
    pub playtime_unknown: usize,
}

/// Total size of regular files directly inside `dir`, non-recursively when
/// `recursive` is false, or recursively when true.
///
/// Returns 0 when the directory is absent or unreadable — never panics.
/// Symbolic links are ignored (`follow_links(false)`).
fn dir_bytes(dir: &Path, recursive: bool) -> u64 {
    let mut total = 0u64;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_symlink() {
            continue;
        }
        if meta.is_file() {
            total += meta.len();
        } else if recursive && meta.is_dir() {
            total += dir_recursive(dir.join(entry.file_name()));
        }
    }
    total
}

/// Recursively sum file sizes under `dir`, skipping symlinks.
fn dir_recursive(dir: PathBuf) -> u64 {
    let mut total = 0u64;
    for entry in WalkDir::new(&dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                if !meta.is_symlink() {
                    total += meta.len();
                }
            }
        }
    }
    total
}

/// Aggregate the already-computed game statuses plus the on-disk footprint.
pub fn compute(statuses: &[GameStatus], lib: &Path, data_dir: &Path) -> LibraryStats {
    // Stage histogram — only stages that actually appear.
    let mut stage_map: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    let mut hidden = 0usize;
    let mut fixes_installed = 0usize;
    let mut fixes_downloaded = 0usize;
    let mut games_on_disk = 0u64;
    let mut playtime_total = 0u64;
    let mut playtime_unknown = 0usize;
    let mut most_played: Option<MostPlayedGame> = None;

    for s in statuses {
        *stage_map.entry(s.stage).or_default() += 1;
        if s.hidden {
            hidden += 1;
        }
        if s.fix.health == fixes::FixHealth::Healthy {
            fixes_installed += 1;
        }
        if s.fix_downloaded {
            fixes_downloaded += 1;
        }
        if s.game.installed && s.game.fully_installed {
            games_on_disk += s.game.size_on_disk;
        }
        match s.playtime_minutes {
            // Only what is known enters the total; zero minutes is a
            // legitimate "jamais joué" and counts as known.
            Some(minutes) => {
                playtime_total += minutes;
                if minutes > 0 {
                    let candidate = MostPlayedGame {
                        app_id: s.app_id.clone(),
                        name: s.name.clone(),
                        minutes,
                    };
                    let wins = match most_played.as_ref() {
                        Some(current) => beats_most_played(&candidate, current),
                        None => true,
                    };
                    if wins {
                        most_played = Some(candidate);
                    }
                }
            }
            None => playtime_unknown += 1,
        }
    }

    // Sort by count descending, then by stage name ascending.
    let mut by_stage: Vec<StageCount> = stage_map
        .into_iter()
        .map(|(stage, count)| StageCount {
            stage: stage.to_string(),
            count,
        })
        .collect();
    by_stage.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.stage.cmp(&b.stage)));

    let lua_bytes = dir_bytes(lib, false);
    let fix_archive_bytes = if let Some(refs) = fix_refs_dir(lib) {
        dir_bytes(&refs, true)
    } else {
        0
    };
    let backup_bytes = dir_bytes(&data_dir.join("backups"), true);

    LibraryStats {
        total: statuses.len(),
        hidden,
        by_stage,
        fixes_installed,
        fixes_downloaded,
        lua_bytes,
        fix_archive_bytes,
        backup_bytes,
        games_on_disk_bytes: games_on_disk,
        playtime_total_minutes: playtime_total,
        most_played,
        playtime_unknown,
    }
}

/// Return the `fixes` sub-directory inside the library, if it exists.
fn fix_refs_dir(lib: &Path) -> Option<PathBuf> {
    let p = lib.join("fixes");
    p.is_dir().then_some(p)
}

/// Strict ordering for the "most played" title: more minutes wins; on a tie,
/// alphabetical name (case-insensitive) then app_id, so the answer never
/// flips between two refreshes of the same library.
fn beats_most_played(candidate: &MostPlayedGame, current: &MostPlayedGame) -> bool {
    candidate.minutes > current.minutes
        || (candidate.minutes == current.minutes
            && (candidate.name.to_lowercase(), candidate.app_id.as_str())
                < (current.name.to_lowercase(), current.app_id.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixes::FixHealth;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ast_stats_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ---------------------------------------------------------------- dir_bytes

    #[test]
    fn dir_bytes_on_absent_dir_returns_zero() {
        let absent = PathBuf::from(r"\this\path\does\not\exist\ast_stats_absent");
        assert_eq!(dir_bytes(&absent, false), 0);
        assert_eq!(dir_bytes(&absent, true), 0);
    }

    #[test]
    fn dir_bytes_recursive_vs_non_recursive() {
        let base = scratch("dir_bytes_tree");
        // Create a two-level tree:
        //   base/a.txt  (10 bytes)
        //   base/sub/b.txt (20 bytes)
        std::fs::write(base.join("a.txt"), b"0123456789").unwrap();
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("sub").join("b.txt"), b"01234567890123456789").unwrap();

        // Non-recursive: only a.txt.
        assert_eq!(dir_bytes(&base, false), 10);

        // Recursive: a.txt + sub/b.txt.
        assert_eq!(dir_bytes(&base, true), 30);

        let _ = std::fs::remove_dir_all(&base);
    }

    // ---------------------------------------------------------------- compute

    fn fake_status(app_id: &str, stage: &'static str, hidden: bool, fix_downloaded: bool, size: u64) -> GameStatus {
        // Determine health from the stage name for realistic test data.
        let health = match stage {
            "fix_damaged" | "fix_game_moved" => FixHealth::Damaged,
            "fix_installed" => FixHealth::Healthy,
            _ => FixHealth::NotInstalled,
        };
        GameStatus {
            app_id: app_id.to_string(),
            name: format!("Game {app_id}"),
            icon: None,
            updated_at: None,
            added_at: None,
            in_library: true,
            lua_in_steam: true,
            fix_downloaded,
            hidden,
            tags: Vec::new(),
            game: crate::vdf::GameInstall {
                app_id: app_id.to_string(),
                known_to_steam: true,
                installed: true,
                fully_installed: true,
                size_on_disk: size,
                ..Default::default()
            },
            playtime_minutes: None,
            last_played: None,
            fix: fixes::FixReport {
                app_id: app_id.to_string(),
                health,
                installed_at: None,
                game_dir: None,
                file_count: 0,
                missing: Vec::new(),
                modified: Vec::new(),
                has_backup: false,
                foreign: Vec::new(),
            },
            stage,
        }
    }

    fn fake_status_not_installed(app_id: &str, stage: &'static str, hidden: bool) -> GameStatus {
        GameStatus {
            app_id: app_id.to_string(),
            name: format!("Game {app_id}"),
            icon: None,
            updated_at: None,
            added_at: None,
            in_library: true,
            lua_in_steam: true,
            fix_downloaded: false,
            hidden,
            tags: Vec::new(),
            game: crate::vdf::GameInstall {
                app_id: app_id.to_string(),
                known_to_steam: true,
                installed: false,
                fully_installed: false,
                ..Default::default()
            },
            playtime_minutes: None,
            last_played: None,
            fix: fixes::FixReport {
                app_id: app_id.to_string(),
                health: fixes::FixHealth::NotInstalled,
                installed_at: None,
                game_dir: None,
                file_count: 0,
                missing: Vec::new(),
                modified: Vec::new(),
                has_backup: false,
                foreign: Vec::new(),
            },
            stage,
        }
    }

    #[test]
    fn compute_counts_stages_and_fixes() {
        let lib = scratch("compute_lib");
        let data = scratch("compute_data");

        let statuses = vec![
            fake_status("1", "fix_installed", false, false, 10_000),
            fake_status("2", "fix_installed", false, false, 20_000),
            fake_status("3", "fix_damaged", false, true, 30_000),
            fake_status_not_installed("4", "ready", true),
            fake_status_not_installed("5", "needs_steam_install", false),
        ];

        // Write a .lua file so lua_bytes > 0.
        std::fs::write(lib.join("1.lua"), b"-- lua").unwrap();
        std::fs::write(lib.join("index.json"), b"[]").unwrap();

        let stats = compute(&statuses, &lib, &data);

        assert_eq!(stats.total, 5);
        assert_eq!(stats.hidden, 1);
        assert_eq!(stats.fixes_installed, 2); // only stages 1 and 2 have Healthy
        assert_eq!(stats.fixes_downloaded, 1); // stage 3

        // by_stage: only present stages, sorted by count desc then name asc.
        assert_eq!(stats.by_stage.len(), 4); // fix_installed(2) + 3 unique stages
        assert_eq!(stats.by_stage[0].count, 2);
        assert_eq!(stats.by_stage[0].stage, "fix_installed");
        assert_eq!(stats.by_stage[1].count, 1);
        assert_eq!(stats.by_stage[1].stage, "fix_damaged");
        assert_eq!(stats.by_stage[2].count, 1);
        assert_eq!(stats.by_stage[2].stage, "needs_steam_install");
        assert_eq!(stats.by_stage[3].count, 1);
        assert_eq!(stats.by_stage[3].stage, "ready");

        // games_on_disk: only fully installed games count (10k + 20k + 30k = 60k;
        // jeu 5 is not installed, jeu 4 is hidden but still counted).
        assert_eq!(stats.games_on_disk_bytes, 60_000);
        assert!(stats.lua_bytes > 0);

        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn compute_empty_library() {
        let lib = scratch("compute_empty");
        let data = scratch("compute_empty_data");

        let stats = compute(&[], &lib, &data);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.hidden, 0);
        assert!(stats.by_stage.is_empty());
        assert_eq!(stats.fixes_installed, 0);
        assert_eq!(stats.fixes_downloaded, 0);
        assert_eq!(stats.lua_bytes, 0);
        assert_eq!(stats.fix_archive_bytes, 0);
        assert_eq!(stats.backup_bytes, 0);
        assert_eq!(stats.games_on_disk_bytes, 0);
        assert_eq!(stats.playtime_total_minutes, 0);
        assert!(stats.most_played.is_none());
        assert_eq!(stats.playtime_unknown, 0);

        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn compute_no_zero_entries_in_by_stage() {
        let lib = scratch("compute_no_zeros");
        let data = scratch("compute_no_zeros_data");

        let statuses = vec![
            fake_status("1", "ready", false, false, 0),
            fake_status("2", "ready", false, false, 0),
        ];

        let stats = compute(&statuses, &lib, &data);
        // Only "ready" should appear, with count 2.
        assert_eq!(stats.by_stage.len(), 1);
        assert_eq!(stats.by_stage[0].stage, "ready");
        assert_eq!(stats.by_stage[0].count, 2);

        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&data);
    }

    // ---------------------------------------------------------------- byte fields

    #[test]
    fn compute_byte_fields_with_realistic_tree() {
        let lib = scratch("compute_bytes_tree");
        let data = scratch("compute_bytes_data");

        // Create a realistic library tree:
        //   lib/index.json  (10 bytes)
        //   lib/1.lua       (20 bytes)
        //   lib/fixes/1_online_fix.rar       (100 bytes)
        //   lib/fixes/1.state.json           (30 bytes)
        //   lib/fixes/backups/1_pre-fix.zip  (200 bytes)
        //   data/backups/auto-x.luabak       (150 bytes)
        std::fs::write(lib.join("index.json"), b"0123456789").unwrap();
        std::fs::write(lib.join("1.lua"), b"01234567890123456789").unwrap();
        std::fs::create_dir_all(lib.join("fixes")).unwrap();
        std::fs::write(lib.join("fixes").join("1_online_fix.rar"), vec![0u8; 100]).unwrap();
        std::fs::write(lib.join("fixes").join("1.state.json"), vec![0u8; 30]).unwrap();
        std::fs::create_dir_all(lib.join("fixes").join("backups")).unwrap();
        std::fs::write(lib.join("fixes").join("backups").join("1_pre-fix.zip"), vec![0u8; 200]).unwrap();
        std::fs::create_dir_all(data.join("backups")).unwrap();
        std::fs::create_dir_all(data.join("backups").join("rollup")).unwrap();
        std::fs::write(data.join("backups").join("auto-x.luabak"), vec![0u8; 150]).unwrap();
        std::fs::write(data.join("backups").join("rollup").join("00001.luabak"), vec![0u8; 200]).unwrap();

        let statuses = vec![
            fake_status("1", "fix_installed", false, false, 0),
        ];

        let stats = compute(&statuses, &lib, &data);

        // lua_bytes = index.json (10) + 1.lua (20) = 30
        assert_eq!(stats.lua_bytes, 30);
        // fix_archive_bytes = rar (100) + state.json (30) + backups zip (200) = 330
        assert_eq!(stats.fix_archive_bytes, 330);
        // backup_bytes = auto-x.luabak (150) + rollup/00001.luabak (200) = 350
        assert_eq!(stats.backup_bytes, 350);

        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn compute_excludes_partial_install_from_games_on_disk() {
        let lib = scratch("compute_partial_disk");
        let data = scratch("compute_partial_data");

        let statuses = vec![
            // Fully installed — should be counted
            fake_status("1", "fix_installed", false, false, 50_000),
            // Not fully installed — must NOT be counted
            {
                let mut s = fake_status("2", "installing", false, false, 99_000);
                s.game.fully_installed = false;
                s
            },
        ];

        let stats = compute(&statuses, &lib, &data);
        // Only game 1 counts: 50 000
        assert_eq!(stats.games_on_disk_bytes, 50_000);

        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&data);
    }

    // ---------------------------------------------------------------- playtime

    /// LOT-13: the total counts only KNOWN playtimes, the most-played game
    /// comes out of the same set, and games without data are reported as a
    /// count — never folded into the total as zero minutes.
    #[test]
    fn compute_aggregates_playtime_and_counts_unknowns() {
        let lib = scratch("pt_stats");
        let data = scratch("pt_stats_data");

        let mut statuses = vec![
            fake_status("1", "ready", false, false, 0),
            fake_status("2", "ready", false, false, 0),
            fake_status("3", "ready", false, false, 0),
            fake_status("4", "ready", false, false, 0),
        ];
        statuses[0].name = "Alpha".to_string();
        statuses[0].playtime_minutes = Some(217);
        statuses[1].name = "Beta".to_string();
        statuses[1].playtime_minutes = Some(43);
        statuses[2].name = "Gamma".to_string();
        statuses[2].playtime_minutes = Some(0); // jamais joué — known zero
        statuses[3].name = "Delta".to_string();
        statuses[3].playtime_minutes = None; // sans donnée

        let stats = compute(&statuses, &lib, &data);
        assert_eq!(stats.playtime_total_minutes, 260, "217 + 43 + 0, jamais 3 ni 4");
        assert_eq!(stats.playtime_unknown, 1, "un seul jeu sans donnée");
        let most = stats.most_played.expect("des minutes sont connues");
        assert_eq!(most.app_id, "1");
        assert_eq!(most.name, "Alpha");
        assert_eq!(most.minutes, 217);

        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn compute_most_played_tiebreak_is_stable() {
        let lib = scratch("pt_tie");
        let data = scratch("pt_tie_data");

        let mut zeta = fake_status("10", "ready", false, false, 0);
        zeta.name = "Zeta".to_string();
        zeta.playtime_minutes = Some(100);
        let mut alpha = fake_status("9", "ready", false, false, 0);
        alpha.name = "Alpha".to_string();
        alpha.playtime_minutes = Some(100);

        // Whatever the input order, the same game holds the title.
        for statuses in [vec![zeta.clone(), alpha.clone()], vec![alpha.clone(), zeta.clone()]] {
            let stats = compute(&statuses, &lib, &data);
            let most = stats.most_played.expect("100 minutes partout");
            assert_eq!(most.name, "Alpha", "à égalité, le nom alphabétique gagne");
        }

        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn compute_most_played_none_without_positive_minutes() {
        let lib = scratch("pt_none");
        let data = scratch("pt_none_data");

        let mut never = fake_status("1", "ready", false, false, 0);
        never.playtime_minutes = Some(0); // jamais joué
        let mut unknown = fake_status("2", "ready", false, false, 0);
        unknown.playtime_minutes = None;

        let stats = compute(&[never, unknown], &lib, &data);
        assert_eq!(stats.playtime_total_minutes, 0);
        assert!(
            stats.most_played.is_none(),
            "personne n'a joué : pas de « jeu le plus joué »"
        );
        assert_eq!(stats.playtime_unknown, 1);

        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&data);
    }
}

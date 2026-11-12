//! The last step of leaving OpenSteamTool: Steam back on its latest build, done by the app.
//!
//! Once the classic SteamTools is in place, Steam must leave the old build (1788652215) our
//! `steam_downgrade` installed: the classic SteamTools does not work on it (the games show
//! « Acheter », the author's machine, 2026-11-11). Left alone, Steam downloads its update silently
//! while it runs and installs it only at its next start: nothing tells the person.
//!
//! So the app does it, once per session, when the classic SteamTools sits on that old build and no
//! game runs: Steam is closed, `steam.exe -forcesteamupdate -forcepackagedownload -exitsteam`
//! downloads and installs the latest build, and the bootstrapper relaunches Steam (Windows Sandbox,
//! 1788652215 → 1794424036 in 14 s, 2026-11-11). Its progress is read in `logs\bootstrap_log.txt`
//! and published to the window (`PROGRESS_EVENT`), which shows it in the banner.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use log::{info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::steam_downgrade;

pub const PROGRESS_EVENT: &str = "steam-update://progress";
/// The bootstrapper downloaded ≈ 250 MB in 5 s on the bench; a slow line gets ten minutes.
const TIMEOUT: Duration = Duration::from_secs(600);
/// How often the watcher looks (it waits for the classic SteamTools after a repair).
const POLL_SECS: u64 = 10;

/// Where the update stands, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    /// Steam is being closed, or the bootstrapper looks for the update.
    Starting,
    Downloading { percent: u8 },
    Installing,
    Done { version: String },
    Failed { reason: String },
}

impl Phase {
    pub fn running(&self) -> bool {
        matches!(self, Phase::Starting | Phase::Downloading { .. } | Phase::Installing)
    }
}

static CURRENT: Mutex<Option<Phase>> = Mutex::new(None);
static TRIED: AtomicBool = AtomicBool::new(false);

/// The last phase published this session (the window asks it when it opens).
pub fn current() -> Option<Phase> {
    CURRENT.lock().unwrap().clone()
}

fn digits(s: &str) -> Option<u64> {
    let d: String = s.chars().filter(char::is_ascii_digit).collect();
    d.parse().ok()
}

/// Pure. The phase the bootstrapper's own lines describe, read from the lines it wrote since our
/// run began. `None` while it has said nothing that counts yet.
///
/// The lines, in the language of the client (French here) or English: `Téléchargement : (52,838 sur
/// 251,515 Ko)` / `Downloading update (52,838 of 251,515 KB)`; the language-free `Found pending update`
/// once downloaded. The two numbers are read digits only: the ratio does not care which separator.
pub fn phase_of(log: &str) -> Option<Phase> {
    let mut phase = None;
    for line in log.lines() {
        if line.contains("Found pending update") || line.contains("Extraction du package") || line.contains("Extracting package") {
            phase = Some(Phase::Installing);
            continue;
        }
        let Some(open) = line.rfind('(') else { continue };
        let Some(close) = line[open..].find(')') else { continue };
        let inner = &line[open + 1..open + close];
        let Some((done, rest)) = inner.split_once(" sur ").or_else(|| inner.split_once(" of ")) else { continue };
        let (total, unit) = rest.rsplit_once(' ').unwrap_or((rest, ""));
        if !matches!(unit, "Ko" | "KB") {
            continue;
        }
        if let (Some(done), Some(total)) = (digits(done), digits(total)) {
            if total > 0 && !matches!(phase, Some(Phase::Installing)) {
                phase = Some(Phase::Downloading { percent: (done.min(total) * 100 / total) as u8 });
            }
        }
    }
    phase
}

/// Pure. Whether the app should bring Steam up to date now: the classic SteamTools in place, Steam
/// not pinned any more (the pin of ours is lifted by `steam_keys`' watcher), on our old build or
/// older, no game running, and not tried yet this session (a failed try is not repeated in a loop).
pub fn wanted(classic: bool, pinned: bool, installed: Option<&str>, game_running: bool, tried: bool) -> bool {
    let old = steam_downgrade::OUR_PINNED_BUILD.parse::<u64>().unwrap_or(0);
    classic
        && !pinned
        && !game_running
        && !tried
        && installed.and_then(|v| v.trim().parse::<u64>().ok()).is_some_and(|v| v <= old)
}

fn publish(app: &AppHandle, phase: Phase) {
    *CURRENT.lock().unwrap() = Some(phase.clone());
    let _ = app.emit(PROGRESS_EVENT, phase);
}

fn log_path(steam: &Path) -> PathBuf {
    steam.join("logs").join("bootstrap_log.txt")
}

fn read_from(path: &Path, offset: u64) -> String {
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    if f.seek(SeekFrom::Start(offset)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    let _ = f.take(1 << 20).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Close Steam, run the forced update, follow it, and make sure Steam runs afterwards.
/// Blocking: called on a blocking thread.
pub fn update_to_latest(app: &AppHandle, steam: &Path) -> anyhow::Result<String> {
    let exe = steam_downgrade::steam_exe(steam)?;
    publish(app, Phase::Starting);
    steam_downgrade::shutdown_steam(&exe)?;
    steam_downgrade::remove_pin(steam)?;
    let log = log_path(steam);
    let offset = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
    let mut child = steam_downgrade::hidden(
        std::process::Command::new(&exe).args(["-forcesteamupdate", "-forcepackagedownload", "-exitsteam"]),
    )
    .spawn()?;
    let start = Instant::now();
    let mut last = Phase::Starting;
    loop {
        if let Some(phase) = phase_of(&read_from(&log, offset)) {
            if phase != last {
                publish(app, phase.clone());
                last = phase;
            }
        }
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            anyhow::bail!("la mise à jour de Steam n'a pas abouti en dix minutes");
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    crate::vdf::clear_caches();
    let version = steam_downgrade::installed_version(steam).unwrap_or_default();
    let old = steam_downgrade::OUR_PINNED_BUILD.parse::<u64>().unwrap_or(0);
    if version.trim().parse::<u64>().map_or(true, |v| v <= old) {
        anyhow::bail!("Steam est resté sur la version {}", if version.is_empty() { "inconnue" } else { &version });
    }
    // The bootstrapper relaunches Steam after an update; if it did not, the app does.
    std::thread::sleep(Duration::from_secs(10));
    if !steam_downgrade::steam_running() {
        let _ = std::process::Command::new(&exe).spawn();
    }
    Ok(version)
}

/// The watcher every instance runs: when `wanted`, the update, announced step by step.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(POLL_SECS));
        if current().is_some_and(|p| p.running()) {
            continue;
        }
        let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
        let Some(steam) = cfg.steam_dir.clone().or_else(crate::detect::detect_steam_path) else { continue };
        let classic = crate::detect::flavor(&steam) == crate::detect::Flavor::Classic;
        let installed = steam_downgrade::installed_version(&steam);
        let game = crate::motd_run::game_running(crate::motd_run::running_app_id());
        if !wanted(classic, steam_downgrade::is_pinned(&steam), installed.as_deref(), game, TRIED.load(Ordering::SeqCst)) {
            continue;
        }
        TRIED.store(true, Ordering::SeqCst);
        info!("mise à jour de Steam : SteamTools classique sur l'ancienne version {}, mise à jour lancée", installed.unwrap_or_default());
        match update_to_latest(&app, &steam) {
            Ok(version) => {
                info!("mise à jour de Steam : terminée, version {version}");
                publish(&app, Phase::Done { version });
            }
            Err(e) => {
                warn!("mise à jour de Steam : échec ({e})");
                publish(&app, Phase::Failed { reason: e.to_string() });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bootstrapper's lines of a forced update, as written in Windows Sandbox on 2026-11-11 (French
    /// client, 1788652215 → 1794424036), the noise lines (pending downloads, requests) shortened.
    const REAL_LOG: &str = "[2026-11-11 23:39:16] Startup - Steam Client launched with: \"C:\\Program Files (x86)\\Steam\\steam.exe\" -forcesteamupdate -forcepackagedownload -exitsteam \n\
[2026-11-11 23:39:16] Checking for update on startup\n\
[2026-11-11 23:39:16] Recherche de nouvelles mises à jour...\n\
[2026-11-11 23:39:16] Downloaded new manifest: /steam_client_win64 version 1794424036, installed version 1788652215, existing pending version 0\n\
[2026-11-11 23:39:16] Package file bins_win64.zip.vz.8314703354cc39622eb2b6ab226c43778f0e5a81_35740846 missing or incorrect size\n\
[2026-11-11 23:39:17] Téléchargement : (27,625 sur 251,515 Ko)\n\
[2026-11-11 23:39:17] Téléchargement : (52,838 sur 251,515 Ko)\n";
    const REAL_END: &str = "[2026-11-11 23:39:21] Téléchargement : (251,515 sur 251,515 Ko)\n\
[2026-11-11 23:39:21] Téléchargement terminé.\n\
[2026-11-11 23:39:21] uninstalled manifest found in C:\\Program Files (x86)\\Steam\\package\\steam_client_win64 (1).\n\
[2026-11-11 23:39:21] Found pending update\n\
[2026-11-11 23:39:21] Installation de la mise à jour...\n\
[2026-11-11 23:39:22] Extraction du package...\n\
[2026-11-11 23:39:30] Mise à jour terminée. Lancement de Steam...\n";

    #[test]
    fn steam_update_reads_the_bootstrappers_real_progress() {
        assert_eq!(phase_of(REAL_LOG), Some(Phase::Downloading { percent: 21 }));
        assert_eq!(phase_of(&format!("{REAL_LOG}{REAL_END}")), Some(Phase::Installing));
        assert_eq!(phase_of(""), None);
        assert_eq!(phase_of("[x] Checking for update on startup\n"), None, "nothing that counts yet");
        // The English client, same shape.
        assert_eq!(phase_of("[x] Downloading update (125,758 of 251,515 KB)...\n"), Some(Phase::Downloading { percent: 50 }));
        // Parentheses that are not a download are not read as one.
        assert_eq!(phase_of("[x] uninstalled manifest found in C:\\Steam\\package\\steam_client_win64 (1).\n"), None);
        assert_eq!(phase_of("[x] Windows 10.0.26100.0, 0, 2, 256, 1, 4 (2 sur 0 Ko)\n"), None, "no total, no ratio");
    }

    #[test]
    fn steam_update_runs_only_for_the_classic_steamtools_on_the_old_build() {
        assert!(wanted(true, false, Some("1788652215"), false, false));
        assert!(wanted(true, false, Some("1780000000"), false, false), "older still");
        assert!(!wanted(false, false, Some("1788652215"), false, false), "OpenSteamTool needs the old build");
        assert!(!wanted(true, true, Some("1788652215"), false, false), "the pin is lifted first, by the keys watcher");
        assert!(!wanted(true, false, Some("1794424036"), false, false), "already up to date");
        assert!(!wanted(true, false, None, false, false), "unknown build: left alone");
        assert!(!wanted(true, false, Some("1788652215"), true, false), "never while a game runs");
        assert!(!wanted(true, false, Some("1788652215"), false, true), "once per session");
    }

    #[test]
    fn steam_update_phases_say_whether_it_still_runs() {
        assert!(Phase::Starting.running() && Phase::Installing.running() && Phase::Downloading { percent: 3 }.running());
        assert!(!Phase::Done { version: "1".into() }.running() && !Phase::Failed { reason: "x".into() }.running());
        let json = serde_json::to_value(Phase::Downloading { percent: 42 }).unwrap();
        assert_eq!(json, serde_json::json!({ "phase": "downloading", "percent": 42 }));
    }
}

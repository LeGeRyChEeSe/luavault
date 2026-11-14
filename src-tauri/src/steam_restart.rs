//! LuaVault inside Steam needs Steam started AFTER the file `.cef-enable-remote-debugging`
//! exists (`steam_client::MARKER_FILE`): the client opens its debugging port only at its start.
//!
//! Before this module, the person who installed the experimental had to restart Steam by hand,
//! and someone installing it for the first time could not know (field report, 2026-11-13). Now
//! every instance watches: while the setting wants the file, the file is written, and a Steam that
//! was started before it (process creation time older than the file) is closed and started again,
//! once per session, never while a game runs, and never while the app brings Steam up to date
//! (`steam_update::due`): that update restarts Steam anyway, after the file is in place.

use std::path::Path;
use std::time::{Duration, SystemTime};

use log::{info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{commands, config, steam_client, steam_downgrade, steam_update};

/// What the window hears (a toast): Steam is restarting, then whether it came back.
pub const EVENT: &str = "steam-client://restart";
const POLL_SECS: u64 = 10;
/// Checks in a row that must ask for the restart before it happens. A Steam the bootstrapper just
/// relaunched, a pin `steam_keys` is about to lift (then the update comes first): a few seconds
/// settle them, and a restart is not a gesture to make on one reading.
const SETTLE_TICKS: u32 = 3;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Phase {
    Restarting,
    Done,
    Failed { reason: String },
}

/// What one look at this machine found.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts {
    /// The setting wants LuaVault inside Steam (`steam_client::wanted`).
    pub wanted: bool,
    /// The kill switch is engaged: nothing is drawn in Steam, nothing to restart for.
    pub hidden: bool,
    /// When the running Steam started; `None` when Steam does not run (it will read the file).
    pub steam_started: Option<SystemTime>,
    /// When the file was written; `None` when it is not there.
    pub marker_written: Option<SystemTime>,
    /// Something answers on the debugging port already.
    pub port_open: bool,
    /// The app is updating Steam, or is about to.
    pub update_due: bool,
    pub game_running: bool,
    /// This session already restarted Steam for it (one try, never a loop).
    pub tried: bool,
}

/// Pure. Whether Steam must be restarted now for LuaVault to appear in it.
pub fn restart_wanted(f: &Facts) -> bool {
    f.wanted
        && !f.hidden
        && !f.port_open
        && !f.update_due
        && !f.game_running
        && !f.tried
        && matches!((f.steam_started, f.marker_written), (Some(s), Some(m)) if s < m)
}

/// When the Steam process named in `HKCU\Software\Valve\Steam\ActiveProcess\pid` started.
#[cfg(windows)]
fn steam_started() -> Option<SystemTime> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use winreg::enums::HKEY_CURRENT_USER;

    let pid = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam\ActiveProcess")
        .ok()?
        .get_value::<u32, _>("pid")
        .ok()
        .filter(|p| *p != 0)?;
    // SAFETY: a handle opened here, read once, closed before returning.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) =
            (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let read = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
        let _ = CloseHandle(handle);
        read.ok()?;
        filetime_to_system(((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64)
    }
}

#[cfg(not(windows))]
fn steam_started() -> Option<SystemTime> {
    None
}

/// Pure. A Windows FILETIME (100 ns ticks since 1601-01-01) as a `SystemTime`.
pub fn filetime_to_system(ticks: u64) -> Option<SystemTime> {
    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
    let since_unix = ticks.checked_sub(UNIX_EPOCH_TICKS)?;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_nanos(since_unix.checked_mul(100)?))
}

fn port_open() -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], steam_client::DEBUG_PORT));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

fn publish(app: &AppHandle, phase: Phase) {
    let _ = app.emit(EVENT, phase);
}

fn restart(steam: &Path) -> anyhow::Result<()> {
    let exe = steam_downgrade::steam_exe(steam)?;
    steam_downgrade::shutdown_steam(&exe)?;
    steam_downgrade::hidden(&mut std::process::Command::new(&exe)).spawn()?;
    Ok(())
}

/// Debug builds (the bench, `tauri dev`) never close the developer's Steam unless asked.
fn active() -> bool {
    !cfg!(debug_assertions) || std::env::var("LV_STEAM_RESTART").as_deref() == Ok("1")
}

/// The watcher every instance runs.
pub fn spawn(app: AppHandle) {
    if !active() {
        return;
    }
    std::thread::spawn(move || {
        let mut tried = false;
        let mut streak = 0u32;
        loop {
            std::thread::sleep(Duration::from_secs(POLL_SECS));
            let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
            let wanted = steam_client::wanted(&cfg, config::is_portable(), config::EXPERIMENTAL);
            let Some(steam) = commands::resolve_steam(&cfg) else { continue };
            let marker = steam_client::marker_path(&steam);
            if wanted && !marker.exists() {
                match steam_client::set_marker(&steam, true) {
                    Ok(()) => info!("LuaVault dans Steam : fichier de débogage écrit"),
                    Err(e) => warn!("LuaVault dans Steam : fichier de débogage non écrit ({e})"),
                }
            }
            let running = steam_downgrade::steam_running();
            let game_running = crate::motd_run::game_running(crate::motd_run::running_app_id());
            let facts = Facts {
                wanted,
                hidden: crate::kill_switch::engaged(&config::data_dir()),
                steam_started: if running { steam_started() } else { None },
                marker_written: std::fs::metadata(&marker).and_then(|m| m.modified()).ok(),
                port_open: wanted && running && port_open(),
                update_due: steam_update::due(&steam, game_running),
                game_running,
                tried,
            };
            if !restart_wanted(&facts) {
                streak = 0;
                continue;
            }
            streak += 1;
            if streak < SETTLE_TICKS {
                continue;
            }
            tried = true;
            info!("LuaVault dans Steam : Steam a démarré avant le fichier, redémarrage");
            publish(&app, Phase::Restarting);
            match restart(&steam) {
                Ok(()) => publish(&app, Phase::Done),
                Err(e) => {
                    warn!("LuaVault dans Steam : redémarrage de Steam impossible ({e})");
                    publish(&app, Phase::Failed { reason: e.to_string() });
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> Option<SystemTime> {
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs))
    }

    fn due() -> Facts {
        Facts { wanted: true, steam_started: at(100), marker_written: at(200), ..Default::default() }
    }

    #[test]
    fn steam_restart_only_a_steam_older_than_the_file() {
        assert!(restart_wanted(&due()), "Steam started before the file: restarted");
        assert!(!restart_wanted(&Facts { steam_started: at(300), ..due() }), "started after: it read the file");
        assert!(!restart_wanted(&Facts { steam_started: None, ..due() }), "not running: it will read it");
        assert!(!restart_wanted(&Facts { marker_written: None, ..due() }), "no file: a restart changes nothing");
    }

    #[test]
    fn steam_restart_never_against_a_reason_to_wait() {
        assert!(!restart_wanted(&Facts { wanted: false, ..due() }), "setting off");
        assert!(!restart_wanted(&Facts { hidden: true, ..due() }), "kill switch engaged");
        assert!(!restart_wanted(&Facts { port_open: true, ..due() }), "already open");
        assert!(!restart_wanted(&Facts { update_due: true, ..due() }), "the update restarts Steam itself");
        assert!(!restart_wanted(&Facts { game_running: true, ..due() }), "a game runs");
        assert!(!restart_wanted(&Facts { tried: true, ..due() }), "once per session");
    }

    #[test]
    fn steam_restart_reads_a_windows_filetime() {
        assert_eq!(filetime_to_system(116_444_736_000_000_000), Some(SystemTime::UNIX_EPOCH));
        assert_eq!(filetime_to_system(116_444_736_000_000_000 + 10_000_000), at(1));
        assert_eq!(filetime_to_system(0), None, "before 1970: unreadable, never a restart");
    }
}

//! SteamTools installed by the app itself, silently (2026-11-14, the author's decision).
//!
//! The openlua script (`fix-st.ps1`) removes OpenSteamTool and installs the classic SteamTools; run
//! by hand it waits for keys and its installer for three clicks (Next, Install, Finish). The app
//! already runs it hidden behind `relaunch::repair_steamtools_then_relaunch`, with its key waits
//! turned into pauses; `relaunch::repair_command_line` now also gives the installer `/S` (it is an
//! NSIS installer) and the elevated window is hidden. What is left is ONE Windows elevation prompt:
//! the installer writes to Program Files.
//!
//! This module decides when the app starts it by itself: Steam is there, the classic SteamTools is
//! not (absent, or still OpenSteamTool), no game runs, and the last automatic try is a day old at
//! least, so a « No » to the prompt is not asked again at every start.

use crate::detect::Flavor;

/// A day between two automatic tries.
pub const RETRY_SECS: u64 = 24 * 60 * 60;

/// Pure. Whether the app should install SteamTools by itself now.
pub fn due(steam_found: bool, flavor: Flavor, game_running: bool, last_try: Option<u64>, now: u64) -> bool {
    steam_found
        && flavor != Flavor::Classic
        && !game_running
        && last_try.is_none_or(|at| now.saturating_sub(at) >= RETRY_SECS || at > now + 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steamtools_auto_installs_when_the_classic_one_is_missing() {
        assert!(due(true, Flavor::None, false, None, 1_000), "absent: installed");
        assert!(due(true, Flavor::OpenSteamTool, false, None, 1_000), "OpenSteamTool: replaced");
        assert!(!due(true, Flavor::Classic, false, None, 1_000), "already there");
        assert!(!due(false, Flavor::None, false, None, 1_000), "no Steam, nothing to put it in");
        assert!(!due(true, Flavor::None, true, None, 1_000), "never while a game runs (Steam is closed)");
    }

    #[test]
    fn steamtools_auto_tries_once_a_day() {
        assert!(!due(true, Flavor::None, false, Some(1_000), 1_000 + RETRY_SECS - 1), "refused or failed today: not again");
        assert!(due(true, Flavor::None, false, Some(1_000), 1_000 + RETRY_SECS), "a day later");
        assert!(due(true, Flavor::None, false, Some(10 * RETRY_SECS), 1_000), "a date in the future is not believed");
    }
}

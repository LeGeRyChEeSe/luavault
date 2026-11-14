//! What a window shows while an automatic update runs in ANOTHER process.
//!
//! The background check (`--notify-check`, `headless::install_offer`) downloads and installs with
//! no window. Before this file existed, a person who opened the app during that download saw it
//! close a moment later, with nothing said, and come back (field report, 2026-11-14): the helper
//! stops every instance before the silent installer runs. Now the background path writes
//! `update-running.json` in the data folder while it works, any open window reads it every few
//! seconds and says what is happening, and the helper removes the file once the installer is done.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "update-running.json";
/// A file older than this is a leftover (a helper that died, a machine turned off mid-way): the
/// download is capped at a few minutes and the installer at three attempts.
pub const FRESH_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Running {
    pub version: String,
    pub started_at: u64,
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Written when the background update starts working (before the download).
pub fn mark(data_dir: &Path, version: &str) {
    let running = Running { version: version.to_string(), started_at: now() };
    if let Ok(json) = serde_json::to_string(&running) {
        let _ = std::fs::write(path(data_dir), json);
    }
}

/// Removed on every way out that does not end in the installer (the helper removes it after).
pub fn clear(data_dir: &Path) {
    let _ = std::fs::remove_file(path(data_dir));
}

/// Pure. The update this file announces, if it is recent; a version that is not a plain version
/// string is not shown.
pub fn fresh(json: &str, now: u64) -> Option<String> {
    let r: Running = serde_json::from_str(json).ok()?;
    let plain = !r.version.is_empty()
        && r.version.len() <= 32
        && r.version.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    (plain && r.started_at <= now + 60 && now.saturating_sub(r.started_at) < FRESH_SECS).then_some(r.version)
}

/// What a window asks every few seconds.
pub fn current(data_dir: &Path) -> Option<String> {
    let json = std::fs::read_to_string(path(data_dir)).ok()?;
    fresh(&json, now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_notice_shows_a_recent_update_only() {
        let at = |v: &str, t: u64| serde_json::to_string(&Running { version: v.into(), started_at: t }).unwrap();
        assert_eq!(fresh(&at("1.5.3", 1000), 1100), Some("1.5.3".into()));
        assert_eq!(fresh(&at("1.5.3-exp.8", 1000), 1000 + FRESH_SECS - 1), Some("1.5.3-exp.8".into()));
        assert_eq!(fresh(&at("1.5.3", 1000), 1000 + FRESH_SECS), None, "a leftover is not an update");
        assert_eq!(fresh(&at("1.5.3", 5000), 1000), None, "a date far ahead is not believed");
        assert_eq!(fresh(&at("<b>1</b>", 1000), 1000), None, "only a plain version string");
        assert_eq!(fresh(&at("", 1000), 1000), None);
        assert_eq!(fresh("{ pas du json", 1000), None);
    }

    #[test]
    fn update_notice_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("lv-update-notice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        mark(&dir, "1.5.4");
        assert_eq!(current(&dir), Some("1.5.4".into()));
        clear(&dir);
        assert_eq!(current(&dir), None);
        clear(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

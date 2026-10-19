use crate::{archive, commands, library, AppState};
use log::debug;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const MAX_LUA_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 2_000;
pub const STABLE_POLLS: u8 = 2;
pub const MAX_WINDOW_MINUTES: u32 = 30;
pub const DOWNLOADS_FOUND_EVENT: &str = "downloads://found";
pub const DOWNLOADS_STATE_EVENT: &str = "downloads://state";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Incomplete,
    Lua { app_id: String },
    Archive { app_id: Option<String> },
    Ignore,
}

fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
}

pub fn classify(
    name: &str,
    size: u64,
    read_head: impl FnOnce(usize) -> Option<Vec<u8>>,
) -> Verdict {
    let extension = extension(name);
    if matches!(
        extension.as_deref(),
        Some("crdownload" | "part" | "tmp" | "download" | "opdownload" | "partial")
    ) || size == 0
    {
        return Verdict::Incomplete;
    }
    match extension.as_deref() {
        Some("lua") if size <= MAX_LUA_BYTES => {
            let Some(bytes) = read_head(MAX_LUA_BYTES as usize) else {
                return Verdict::Ignore;
            };
            let Ok(text) = String::from_utf8(bytes) else {
                return Verdict::Ignore;
            };
            library::parse_lua(&text)
                .app_ids
                .into_iter()
                .next()
                .map(|app_id| Verdict::Lua { app_id })
                .unwrap_or(Verdict::Ignore)
        }
        Some("zip" | "rar") => {
            let Some(head) = read_head(8) else {
                return Verdict::Ignore;
            };
            match archive::kind_from_head(&head) {
                archive::ArchiveKind::Zip | archive::ArchiveKind::Rar => Verdict::Archive {
                    app_id: commands::app_id_from_patch_filename(Path::new(name)),
                },
                _ => Verdict::Ignore,
            }
        }
        _ => Verdict::Ignore,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileKey(String);

impl FileKey {
    fn from_name(name: &str) -> Self {
        Self(name.to_string())
    }
}

impl From<&str> for FileKey {
    fn from(name: &str) -> Self {
        Self::from_name(name)
    }
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub name: String,
    pub size: u64,
    pub age_at_start: Option<Duration>,
}

#[derive(Debug, Clone, Default)]
struct Observation {
    size: Option<u64>,
    equal_polls: u8,
    present: bool,
    proposed: bool,
    ignored: bool,
}

pub struct Tracker {
    _started_at: Instant,
    baseline: HashSet<FileKey>,
    recent_baseline: HashSet<FileKey>,
    observations: HashMap<FileKey, Observation>,
}

impl Tracker {
    pub fn new(started_at: Instant, baseline: Vec<Seen>) -> Self {
        let mut names = HashSet::new();
        let mut recent = HashSet::new();
        for seen in baseline {
            let key = FileKey::from_name(&seen.name);
            if seen
                .age_at_start
                .is_some_and(|age| age <= Duration::from_secs(30))
            {
                recent.insert(key.clone());
            }
            names.insert(key);
        }
        Self {
            _started_at: started_at,
            baseline: names,
            recent_baseline: recent,
            observations: HashMap::new(),
        }
    }

    pub fn observe(&mut self, snapshot: Vec<Seen>) -> Vec<Seen> {
        let mut ready = Vec::new();
        let mut present = HashSet::new();
        for seen in snapshot {
            let key = FileKey::from_name(&seen.name);
            if self.baseline.contains(&key) && !self.recent_baseline.contains(&key) {
                continue;
            }
            present.insert(key.clone());
            let observation = self.observations.entry(key).or_default();
            if !observation.present && observation.size.is_some() {
                observation.equal_polls = 1;
            } else if observation.size == Some(seen.size) {
                observation.equal_polls = observation.equal_polls.saturating_add(1);
            } else {
                observation.size = Some(seen.size);
                observation.equal_polls = 1;
                observation.proposed = false;
                observation.ignored = false;
            }
            observation.present = true;
            if observation.equal_polls >= STABLE_POLLS
                && !observation.proposed
                && !observation.ignored
            {
                observation.proposed = true;
                ready.push(seen);
            }
        }
        for (key, observation) in &mut self.observations {
            if !present.contains(key) {
                observation.present = false;
            }
        }
        ready
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn ignore(&mut self, key: &FileKey) {
        if let Some(observation) = self.observations.get_mut(key) {
            observation.ignored = true;
        }
    }
}

#[derive(Debug, Clone)]
pub struct Window {
    ends_at: Option<Instant>,
}

impl Window {
    pub fn timed(start: Instant, minutes: u32) -> Self {
        let minutes = minutes.clamp(1, MAX_WINDOW_MINUTES);
        Self {
            ends_at: Some(start + Duration::from_secs(u64::from(minutes) * 60)),
        }
    }

    pub fn permanent() -> Self {
        Self { ends_at: None }
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.ends_at.is_some_and(|ends_at| now >= ends_at)
    }

    fn ends_in_secs(&self, now: Instant) -> Option<u64> {
        self.ends_at
            .map(|ends_at| ends_at.saturating_duration_since(now).as_secs())
    }
}

#[derive(Debug, Clone)]
pub struct WatchState {
    active: bool,
    window: Window,
}

impl Default for WatchState {
    fn default() -> Self {
        Self {
            active: false,
            window: Window::permanent(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WatchStatus {
    pub active: bool,
    pub ends_in_secs: Option<u64>,
}

impl WatchState {
    pub fn status(&self, now: Instant) -> WatchStatus {
        WatchStatus {
            active: self.active && !self.window.expired(now),
            ends_in_secs: self.window.ends_in_secs(now),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadFound {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub kind: String,
    pub app_id: Option<String>,
}

pub fn scan(dir: &Path) -> Vec<Seen> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            debug!("downloads scan unavailable: {error}");
            return Vec::new();
        }
    };
    entries
        .take(MAX_ENTRIES)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() || file_type.is_symlink() {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            let name = entry.file_name().to_str()?.to_string();
            let age_at_start = metadata
                .created()
                .ok()
                .and_then(|created| created.elapsed().ok());
            Some(Seen {
                name,
                size: metadata.len(),
                age_at_start,
            })
        })
        .collect()
}

fn read_head(path: &Path, max: usize) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(max as u64).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn emit_state(app: &AppHandle, status: WatchStatus) {
    if let Err(error) = app.emit(DOWNLOADS_STATE_EVENT, status) {
        debug!("downloads state event failed: {error}");
    }
}

pub fn status(state: &AppState) -> WatchStatus {
    state
        .downloads_watch
        .lock()
        .map(|watch| watch.status(Instant::now()))
        .unwrap_or(WatchStatus {
            active: false,
            ends_in_secs: None,
        })
}

pub fn stop(app: &AppHandle, state: &AppState) -> WatchStatus {
    state.downloads_generation.fetch_add(1, Ordering::SeqCst);
    let status = match state.downloads_watch.lock() {
        Ok(mut watch) => {
            watch.active = false;
            watch.status(Instant::now())
        }
        Err(_) => WatchStatus {
            active: false,
            ends_in_secs: None,
        },
    };
    emit_state(app, status.clone());
    status
}

pub fn start(app: &AppHandle, state: &AppState, dir: PathBuf, window: Window) -> WatchStatus {
    let generation = state.downloads_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let initial_status = {
        let mut watch = state
            .downloads_watch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *watch = WatchState {
            active: true,
            window: window.clone(),
        };
        watch.status(Instant::now())
    };
    emit_state(app, initial_status.clone());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let started_at = Instant::now();
        let baseline = match tauri::async_runtime::spawn_blocking({
            let dir = dir.clone();
            move || scan(&dir)
        })
        .await
        {
            Ok(baseline) => baseline,
            Err(error) => {
                debug!("downloads baseline task failed: {error}");
                return;
            }
        };
        let mut tracker = Tracker::new(started_at, baseline);
        loop {
            let current = {
                let state = app.state::<AppState>();
                if state.downloads_generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                status(&state)
            };
            if !current.active {
                let state = app.state::<AppState>();
                if state.downloads_generation.load(Ordering::SeqCst) == generation {
                    emit_state(&app, current);
                }
                return;
            }
            let directory = dir.clone();
            let ready = match tauri::async_runtime::spawn_blocking(move || {
                let snapshot = scan(&directory);
                let files: Vec<(Seen, PathBuf)> = snapshot
                    .iter()
                    .cloned()
                    .map(|seen| {
                        let path = directory.join(&seen.name);
                        (seen, path)
                    })
                    .collect();
                (snapshot, files)
            })
            .await
            {
                Ok((snapshot, files)) => (snapshot, files),
                Err(error) => {
                    debug!("downloads scan task failed: {error}");
                    return;
                }
            };
            let ready_seen = tracker.observe(ready.0);
            let found: Vec<DownloadFound> = tauri::async_runtime::spawn_blocking(move || {
                ready_seen
                    .into_iter()
                    .filter_map(|seen| {
                        let path = ready
                            .1
                            .iter()
                            .find(|(candidate, _)| candidate.name == seen.name)
                            .map(|(_, path)| path)?;
                        match classify(&seen.name, seen.size, |max| read_head(path, max)) {
                            Verdict::Lua { app_id } => Some(DownloadFound {
                                path: path.display().to_string(),
                                name: seen.name,
                                size: seen.size,
                                kind: "lua".to_string(),
                                app_id: Some(app_id),
                            }),
                            Verdict::Archive { app_id } => Some(DownloadFound {
                                path: path.display().to_string(),
                                name: seen.name,
                                size: seen.size,
                                kind: "archive".to_string(),
                                app_id,
                            }),
                            Verdict::Incomplete | Verdict::Ignore => None,
                        }
                    })
                    .collect()
            })
            .await
            .unwrap_or_default();
            if !found.is_empty() {
                if let Err(error) = app.emit(DOWNLOADS_FOUND_EVENT, found) {
                    debug!("downloads found event failed: {error}");
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    initial_status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua_fixture() -> Vec<u8> {
        b"-- AppID 1593030 | Generated on 2026-01-01 05:55 UTC | LuaVault\n-- Main Application\nsetManifestid(1593030, \"123\")\naddappid(1593030, 1, \"340ae2fcb31a92507c1e0a9139f7cac3ab50f45a8c701f6d9f06b1793aca9981\")\n-- DLCs with Content (1)\naddappid(2321930, 1, \"02757f56a474735a503289ae2b52ffb6aeda1c41eb2eec9b5e04bd596eca88d3\")\n".to_vec()
    }

    fn seen(name: &str, size: u64) -> Seen {
        Seen {
            name: name.to_string(),
            size,
            age_at_start: None,
        }
    }

    #[test]
    fn classify_respects_candidates_and_content() {
        for name in [
            "notes.txt",
            "photo.jpg",
            "cv.pdf",
            "x.7z",
            "x.lua.crdownload",
        ] {
            assert!(matches!(
                classify(name, 4, |_| panic!(
                    "non-candidate content must not be read"
                )),
                Verdict::Ignore | Verdict::Incomplete
            ));
        }
        assert_eq!(
            classify("x.lua.crdownload", 4, |_| panic!()),
            Verdict::Incomplete
        );
        assert_eq!(
            classify("x.lua", 10, |_| Some(b"print('no declaration')".to_vec())),
            Verdict::Ignore
        );
        assert_eq!(
            classify("x.zip", 10, |_| Some(b"plain text".to_vec())),
            Verdict::Ignore
        );
        assert_eq!(
            classify("Portal 2 (440).ZIP", 10, |_| Some(b"PK\x03\x04".to_vec())),
            Verdict::Archive {
                app_id: Some("440".to_string())
            }
        );
        assert_eq!(
            classify("real.LUA", lua_fixture().len() as u64, |_| Some(
                lua_fixture()
            )),
            Verdict::Lua {
                app_id: "1593030".to_string()
            }
        );
    }

    #[test]
    fn tracker_requires_stability_and_only_proposes_once() {
        let start = Instant::now();
        let mut tracker = Tracker::new(start, Vec::new());
        assert!(
            tracker.observe(vec![seen("new.lua", 10)]).is_empty(),
            "a single observation is not stable"
        );
        assert_eq!(tracker.observe(vec![seen("new.lua", 10)]).len(), 1);
        assert!(
            tracker.observe(vec![seen("new.lua", 10)]).is_empty(),
            "one file is proposed once"
        );
        assert!(
            tracker.observe(vec![seen("new.lua", 11)]).is_empty(),
            "growth restarts stability"
        );
        assert_eq!(tracker.observe(vec![seen("new.lua", 11)]).len(), 1);
    }

    #[test]
    fn tracker_excludes_old_baseline_but_keeps_recent_downloads() {
        let start = Instant::now();
        let old = Seen {
            name: "old.lua".to_string(),
            size: 10,
            age_at_start: Some(Duration::from_secs(31)),
        };
        let recent = Seen {
            name: "recent.lua".to_string(),
            size: 10,
            age_at_start: Some(Duration::from_secs(29)),
        };
        let mut tracker = Tracker::new(start, vec![old, recent]);
        assert!(
            tracker.observe(vec![seen("old.lua", 99)]).is_empty(),
            "an old baseline file is never proposed"
        );
        assert!(tracker.observe(vec![seen("recent.lua", 10)]).is_empty());
        assert_eq!(
            tracker.observe(vec![seen("recent.lua", 10)]).len(),
            1,
            "a download completed just before start is eligible despite an old modification date"
        );
    }

    #[test]
    fn tracker_ignores_until_the_file_changes() {
        let mut tracker = Tracker::new(Instant::now(), Vec::new());
        let key = FileKey::from("new.lua");
        assert!(tracker.observe(vec![seen("new.lua", 10)]).is_empty());
        assert_eq!(tracker.observe(vec![seen("new.lua", 10)]).len(), 1);
        tracker.ignore(&key);
        assert!(tracker.observe(vec![seen("new.lua", 10)]).is_empty());
        assert!(tracker.observe(vec![seen("new.lua", 11)]).is_empty());
        assert_eq!(tracker.observe(vec![seen("new.lua", 11)]).len(), 1);
    }

    #[test]
    fn window_clamps_to_the_documented_range() {
        let start = Instant::now();
        assert!(Window::timed(start, 0).expired(start + Duration::from_secs(60)));
        assert!(!Window::timed(start, 99_999).expired(start + Duration::from_secs(30 * 60 - 1)));
        assert!(Window::timed(start, 99_999).expired(start + Duration::from_secs(30 * 60)));
        assert!(!Window::permanent().expired(start + Duration::from_secs(31 * 60)));
    }

    #[test]
    fn scan_is_single_level() {
        let root =
            std::env::temp_dir().join(format!("luavault-downloads-scan-{}", std::process::id()));
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("inside.lua"), lua_fixture()).unwrap();
        assert!(
            scan(&root).is_empty(),
            "a nested file is outside the downloads directory"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

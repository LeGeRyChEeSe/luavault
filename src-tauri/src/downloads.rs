use crate::{archive, commands, config, library, notify, AppState};
use log::{debug, info, warn};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const MAX_LUA_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 2_000;
pub const STABLE_POLLS: u8 = 2;
pub const MAX_WINDOW_MINUTES: u32 = 30;
pub const DOWNLOADS_FOUND_EVENT: &str = "downloads://found";
pub const DOWNLOADS_STATE_EVENT: &str = "downloads://state";
/// One tag: each new detection replaces the previous toast instead of piling up.
pub const TOAST_TAG: &str = "downloads-found";
/// A click on the toast asks the running app to show itself; older than this, it is stale.
const WAKE_TTL_SECS: i64 = 60;
const TOAST_MAX_NAMES: usize = 4;

/// What was found and not yet answered, for the toast to list. The window holds the same
/// list in its own memory; this copy exists so a toast can be raised while the window is
/// hidden, and re-raised with every file that arrives afterwards.
static PENDING: Mutex<Vec<DownloadFound>> = Mutex::new(Vec::new());
/// Set once the `luavault://` handler is written: only then does a toast get a click.
static HANDLER_READY: AtomicBool = AtomicBool::new(false);

pub fn set_handler_ready(ready: bool) {
    HANDLER_READY.store(ready, Ordering::SeqCst);
}

/// A debug build is the bench or a dev run: no toast on the developer's desktop unless asked.
fn toasts_enabled() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("LV_DOWNLOADS_TOAST").is_some()
}

fn wake_path() -> PathBuf {
    config::data_dir().join("downloads_wake")
}

/// The toast was clicked (headless process): leave a dated note for the running app.
pub fn request_wake(now: i64) -> std::io::Result<()> {
    std::fs::write(wake_path(), now.to_string())
}

/// Is there a fresh note? The note is always removed, fresh or not.
fn take_wake(now: i64) -> bool {
    let path = wake_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return false;
    };
    let _ = std::fs::remove_file(&path);
    wake_is_fresh(&raw, now)
}

fn wake_is_fresh(raw: &str, now: i64) -> bool {
    raw.trim().parse::<i64>().is_ok_and(|at| (0..WAKE_TTL_SECS).contains(&(now - at)))
}

/// Add what was just found to what is waiting, each file once.
fn merge_pending(pending: &mut Vec<DownloadFound>, found: &[DownloadFound]) {
    for item in found {
        match pending.iter_mut().find(|known| known.path == item.path) {
            Some(known) => *known = item.clone(),
            None => pending.push(item.clone()),
        }
    }
}

/// The toast's words: how many files, and which games (the first few, then a count).
pub fn toast_texts(lang: &str, names: &[String]) -> (String, String) {
    let fr = lang == "fr";
    let count = names.len();
    let title = match (fr, count) {
        (true, 1) => "Un fichier détecté dans Téléchargements".to_string(),
        (true, n) => format!("{n} fichiers détectés dans Téléchargements"),
        (false, 1) => "A file was found in Downloads".to_string(),
        (false, n) => format!("{n} files were found in Downloads"),
    };
    let shown = names.iter().take(TOAST_MAX_NAMES).cloned().collect::<Vec<_>>().join(", ");
    let more = count.saturating_sub(TOAST_MAX_NAMES);
    let list = match (fr, more) {
        (_, 0) => shown,
        (true, n) => format!("{shown} et {n} autre{}", if n > 1 { "s" } else { "" }),
        (false, n) => format!("{shown} and {n} more"),
    };
    let call = if fr { "Cliquez pour les ajouter à LuaVault." } else { "Click to add them to LuaVault." };
    (title, format!("{list}\n{call}"))
}

async fn game_names(app: &AppHandle, items: &[DownloadFound]) -> Vec<String> {
    let state = app.state::<AppState>();
    let library_dir = state.config.lock().map(|cfg| cfg.resolved_library_dir()).ok();
    let known: HashMap<String, String> = library_dir
        .map(|dir| library::load_index(&dir))
        .unwrap_or_default()
        .into_iter()
        .map(|entry| (entry.app_id, entry.name))
        .collect();
    let mut names = Vec::new();
    for item in items {
        let Some(app_id) = item.app_id.as_deref() else {
            names.push(item.name.clone());
            continue;
        };
        let name = match known.get(app_id) {
            Some(name) => name.clone(),
            None => {
                let looked_up = tokio::time::timeout(
                    Duration::from_secs(3),
                    commands::cached_steam_details(&state, app_id, "english"),
                )
                .await;
                match looked_up {
                    Ok(Ok(details)) if !details.name.trim().is_empty() => details.name,
                    _ => format!("AppID {app_id}"),
                }
            }
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Is the user looking at the window right now? Then the popup is the announcement.
fn window_in_front(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|window| {
        window.is_visible().unwrap_or(false)
            && !window.is_minimized().unwrap_or(false)
            && window.is_focused().unwrap_or(false)
    })
}

/// Raise (or refresh) the toast that lists every file waiting for an answer.
async fn announce(app: &AppHandle, found: &[DownloadFound]) {
    let waiting = {
        let mut pending = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        merge_pending(&mut pending, found);
        pending.retain(|item| Path::new(&item.path).is_file());
        pending.clone()
    };
    if waiting.is_empty() || !toasts_enabled() || window_in_front(app) {
        info!("downloads: {} file(s) waiting, no notification (window in front or toasts off)", waiting.len());
        return;
    }
    let names = game_names(app, &waiting).await;
    let lang = {
        let state = app.state::<AppState>();
        let locale = state.config.lock().ok().and_then(|cfg| cfg.locale.clone());
        notify::pick_language(locale.as_deref(), notify::system_locale().as_deref())
    };
    let (title, body) = toast_texts(lang, &names);
    let (launch, buttons) = if HANDLER_READY.load(Ordering::SeqCst) {
        let mut state = notify::load_state();
        let now = chrono::Utc::now().timestamp();
        state.prune(now);
        let body_nonce = state.issue_downloads(now);
        let button_nonce = state.issue_downloads(now);
        // Persisted BEFORE the toast: a button whose nonce was never saved is dead.
        if notify::save_state(&state).is_err() {
            (String::new(), Vec::new())
        } else {
            let label = if lang == "fr" { "Ajouter" } else { "Add" };
            (
                notify::activation_url("open", &body_nonce),
                vec![(label.to_string(), notify::activation_url("run", &button_nonce))],
            )
        }
    } else {
        (String::new(), Vec::new())
    };
    let xml = notify::toast_xml(&title, &body, &launch, &buttons);
    let shown = tauri::async_runtime::spawn_blocking(move || notify::show_toast_tagged(&xml, TOAST_TAG, false)).await;
    match shown {
        Ok(Ok(())) => info!("downloads: notification raised for {} file(s)", waiting.len()),
        Ok(Err(error)) => warn!("downloads: notification impossible ({error})"),
        Err(error) => warn!("downloads: notification interrupted ({error})"),
    }
}

/// The window answered (added or ignored): nothing waits any more, take the toast back.
pub fn clear_pending() {
    PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    if toasts_enabled() {
        notify::remove_toast(TOAST_TAG);
    }
}

fn reveal_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Incomplete,
    Lua { app_id: String },
    Archive { app_id: Option<String> },
    Ignore,
}

impl Verdict {
    /// What the user is shown, if anything. An archive whose name carries no
    /// AppID is not offered: any `.zip` fetched during the window would
    /// otherwise raise a popup, and the import could not place it anyway.
    pub fn offer(self) -> Option<(&'static str, String)> {
        match self {
            Verdict::Lua { app_id } => Some(("lua", app_id)),
            Verdict::Archive { app_id: Some(app_id) } => Some(("archive", app_id)),
            Verdict::Archive { app_id: None } | Verdict::Incomplete | Verdict::Ignore => None,
        }
    }
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

#[derive(Debug, Clone)]
pub struct Seen {
    pub name: String,
    pub size: u64,
    /// How long before the listening started the file was created. Only the
    /// baseline uses it; a modification date is never consulted, because some
    /// browsers stamp a download with the server's date.
    pub age_at_start: Option<Duration>,
}

/// What `observe` knows about one file that is currently in the folder.
#[derive(Debug, Clone)]
struct Observation {
    size: u64,
    equal_polls: u8,
    proposed: bool,
}

pub struct Tracker {
    /// Names present at launch and older than `RECENT_SECS`: never proposed,
    /// until the file leaves the folder and comes back.
    old_baseline: HashSet<String>,
    observations: HashMap<String, Observation>,
}

/// A file finished just before the user pressed the button is still theirs.
const RECENT_SECS: u64 = 30;

impl Tracker {
    pub fn new(baseline: Vec<Seen>) -> Self {
        let old_baseline = baseline
            .into_iter()
            .filter(|seen| {
                !seen
                    .age_at_start
                    .is_some_and(|age| age <= Duration::from_secs(RECENT_SECS))
            })
            .map(|seen| seen.name)
            .collect();
        Self {
            old_baseline,
            observations: HashMap::new(),
        }
    }

    /// One poll. Returns the files that are new and whose size has been the
    /// same for `STABLE_POLLS` consecutive polls, each of them once.
    pub fn observe(&mut self, snapshot: Vec<Seen>) -> Vec<Seen> {
        let present: HashSet<&str> = snapshot.iter().map(|seen| seen.name.as_str()).collect();
        // A file that left the folder is forgotten: the same name coming back is a
        // new download, whether it was in the baseline or was proposed earlier.
        self.old_baseline.retain(|name| present.contains(name.as_str()));
        self.observations.retain(|name, _| present.contains(name.as_str()));

        let mut ready = Vec::new();
        for seen in snapshot {
            if self.old_baseline.contains(&seen.name) {
                continue;
            }
            let observation = self
                .observations
                .entry(seen.name.clone())
                .or_insert(Observation {
                    size: seen.size,
                    equal_polls: 0,
                    proposed: false,
                });
            if observation.size == seen.size {
                observation.equal_polls = observation.equal_polls.saturating_add(1);
            } else {
                *observation = Observation {
                    size: seen.size,
                    equal_polls: 1,
                    proposed: false,
                };
            }
            if observation.equal_polls >= STABLE_POLLS && !observation.proposed {
                observation.proposed = true;
                ready.push(seen);
            }
        }
        ready
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
        let mut tracker = Tracker::new(baseline);
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
            let polled = tauri::async_runtime::spawn_blocking(move || (scan(&directory), directory))
                .await;
            let Ok((snapshot, directory)) = polled else {
                debug!("downloads scan task failed");
                return;
            };
            let ready = tracker.observe(snapshot);
            let found: Vec<DownloadFound> = tauri::async_runtime::spawn_blocking(move || {
                ready
                    .into_iter()
                    .filter_map(|seen| {
                        let path = directory.join(&seen.name);
                        let (kind, app_id) =
                            classify(&seen.name, seen.size, |max| read_head(&path, max)).offer()?;
                        Some(DownloadFound {
                            path: path.display().to_string(),
                            name: seen.name,
                            size: seen.size,
                            kind: kind.to_string(),
                            app_id: Some(app_id),
                        })
                    })
                    .collect()
            })
            .await
            .unwrap_or_default();
            if !found.is_empty() {
                if let Err(error) = app.emit(DOWNLOADS_FOUND_EVENT, found.clone()) {
                    debug!("downloads found event failed: {error}");
                }
                announce(&app, &found).await;
            }
            if take_wake(chrono::Utc::now().timestamp()) {
                reveal_main(&app);
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

    fn found(path: &str, app_id: &str) -> DownloadFound {
        DownloadFound {
            path: path.to_string(),
            name: path.to_string(),
            size: 1,
            kind: "lua".to_string(),
            app_id: Some(app_id.to_string()),
        }
    }

    #[test]
    fn pending_lists_each_file_once_and_keeps_what_arrived_earlier() {
        let mut pending = Vec::new();
        merge_pending(&mut pending, &[found("a.lua", "1")]);
        merge_pending(&mut pending, &[found("b.lua", "2"), found("a.lua", "1")]);
        let paths: Vec<_> = pending.iter().map(|item| item.path.as_str()).collect();
        assert_eq!(paths, ["a.lua", "b.lua"]);
    }

    #[test]
    fn the_toast_counts_the_files_and_names_the_first_games() {
        let names: Vec<String> = ["A", "B"].iter().map(|n| n.to_string()).collect();
        let (title, body) = toast_texts("fr", &names[..1]);
        assert_eq!(title, "Un fichier détecté dans Téléchargements");
        assert!(body.starts_with("A\n"), "{body}");
        let (title, body) = toast_texts("fr", &names);
        assert_eq!(title, "2 fichiers détectés dans Téléchargements");
        assert!(body.starts_with("A, B\n"));
        let (title, _) = toast_texts("en", &names);
        assert_eq!(title, "2 files were found in Downloads");
        let many: Vec<String> = (1..=7).map(|n| format!("G{n}")).collect();
        let (_, body) = toast_texts("fr", &many);
        assert!(body.starts_with("G1, G2, G3, G4 et 3 autres\n"), "{body}");
        let (_, body) = toast_texts("en", &many);
        assert!(body.starts_with("G1, G2, G3, G4 and 3 more\n"), "{body}");
        let five: Vec<String> = (1..=5).map(|n| format!("G{n}")).collect();
        let (_, body) = toast_texts("fr", &five);
        assert!(body.starts_with("G1, G2, G3, G4 et 1 autre\n"), "singular: {body}");
    }

    #[test]
    fn a_wake_note_is_good_for_a_minute_and_never_from_the_future() {
        assert!(wake_is_fresh("1000", 1000));
        assert!(wake_is_fresh("1000", 1059));
        assert!(!wake_is_fresh("1000", 1060), "stale");
        assert!(!wake_is_fresh("1000", 999), "from the future");
        assert!(!wake_is_fresh("not a number", 1000));
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
    fn classify_refuses_an_oversized_lua_without_reading_it() {
        assert_eq!(
            classify("big.lua", MAX_LUA_BYTES + 1, |_| panic!("too large to read")),
            Verdict::Ignore
        );
        assert_eq!(classify("empty.lua", 0, |_| panic!("empty")), Verdict::Incomplete);
        assert_eq!(
            classify("x.rar", 10, |_| Some(b"Rar! ".to_vec())),
            Verdict::Archive { app_id: None }
        );
        assert_eq!(
            classify("x.7z", 10, |_| panic!("7z is not read")),
            Verdict::Ignore
        );
    }

    #[test]
    fn only_recognisable_files_are_offered() {
        assert_eq!(
            Verdict::Lua { app_id: "7".into() }.offer(),
            Some(("lua", "7".to_string()))
        );
        assert_eq!(
            Verdict::Archive { app_id: Some("440".into()) }.offer(),
            Some(("archive", "440".to_string()))
        );
        assert_eq!(Verdict::Archive { app_id: None }.offer(), None, "a random zip is not offered");
        assert_eq!(Verdict::Incomplete.offer(), None);
        assert_eq!(Verdict::Ignore.offer(), None);
    }

    fn old(name: &str, size: u64) -> Seen {
        Seen {
            name: name.to_string(),
            size,
            age_at_start: Some(Duration::from_secs(RECENT_SECS + 1)),
        }
    }

    #[test]
    fn tracker_requires_stability_and_only_proposes_once() {
        let mut tracker = Tracker::new(Vec::new());
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
    fn tracker_never_proposes_an_old_baseline_file_but_keeps_a_recent_one() {
        let recent = Seen {
            name: "recent.lua".to_string(),
            size: 10,
            age_at_start: Some(Duration::from_secs(RECENT_SECS - 1)),
        };
        let mut tracker = Tracker::new(vec![old("old.lua", 10), recent]);
        for _ in 0..3 {
            assert!(
                tracker.observe(vec![seen("old.lua", 99)]).is_empty(),
                "an old baseline file is never proposed, whatever happens to its size"
            );
        }
        assert!(tracker.observe(vec![seen("recent.lua", 10)]).is_empty());
        assert_eq!(
            tracker.observe(vec![seen("recent.lua", 10)]).len(),
            1,
            "a download finished just before the start is eligible"
        );
    }

    #[test]
    fn tracker_treats_a_file_that_comes_back_as_a_new_download() {
        let mut tracker = Tracker::new(vec![old("123.lua", 10)]);
        assert!(tracker.observe(vec![seen("123.lua", 10)]).is_empty());
        // The user deletes it, then downloads the same name again.
        assert!(tracker.observe(vec![]).is_empty());
        assert!(tracker.observe(vec![seen("123.lua", 12)]).is_empty());
        assert_eq!(
            tracker.observe(vec![seen("123.lua", 12)]).len(),
            1,
            "a baseline name that left the folder is a new file when it returns"
        );
        // Proposed, deleted, downloaded again with other content: proposed again.
        assert!(tracker.observe(vec![]).is_empty());
        assert!(tracker.observe(vec![seen("123.lua", 20)]).is_empty());
        assert_eq!(tracker.observe(vec![seen("123.lua", 20)]).len(), 1);
        // Deleted and downloaded again byte for byte (same size): still a new download.
        assert!(tracker.observe(vec![]).is_empty());
        assert!(tracker.observe(vec![seen("123.lua", 20)]).is_empty());
        assert_eq!(
            tracker.observe(vec![seen("123.lua", 20)]).len(),
            1,
            "an identical re-download is offered again"
        );
    }

    #[test]
    fn tracker_does_not_propose_two_files_of_the_same_size_as_one() {
        let mut tracker = Tracker::new(Vec::new());
        assert!(tracker
            .observe(vec![seen("a.lua", 10), seen("a (1).lua", 10)])
            .is_empty());
        let ready = tracker.observe(vec![seen("a.lua", 10), seen("a (1).lua", 10)]);
        assert_eq!(ready.len(), 2, "a duplicate name is a separate download");
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

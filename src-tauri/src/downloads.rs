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
        (true, 1) => "Un fichier détecté".to_string(),
        (true, n) => format!("{n} fichiers détectés"),
        (false, 1) => "A file was detected".to_string(),
        (false, n) => format!("{n} files were detected"),
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
    /// What the file is called on screen: `lua`, `patch` (online patch) or `bypass` (DRM).
    pub flavor: String,
    /// An archive for a game whose patch is already stored, with other content: adding it
    /// would replace that one.
    pub replaces: bool,
    /// For an archive: what opening it with the default password gives (`None` for a `.lua`).
    pub archive_check: Option<archive::Probe>,
}

/// What the library already holds, to leave out what the user has: nothing is offered twice.
pub struct LibraryFacts {
    library_dir: PathBuf,
    ids: HashSet<String>,
    /// The default archive password of Settings, the one an install would use.
    password: Option<String>,
}

type ProbeKey = (String, u64, u64, u64);

/// A check opens an archive: remembered by (path, size, modification, password), so the list
/// refreshing on every focus does not reopen what did not change. Bounded.
static PROBES: std::sync::LazyLock<Mutex<HashMap<ProbeKey, archive::Probe>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn probe_cached(path: &Path, size: u64, password: Option<&str>) -> archive::Probe {
    use std::hash::{Hash, Hasher};
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_secs());
    // The password is hashed, never stored: changing it must re-check, nothing more.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    password.hash(&mut hasher);
    let key = (path.display().to_string(), size, modified, hasher.finish());
    if let Some(known) = PROBES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(&key) {
        return *known;
    }
    let verdict = archive::probe(path, password);
    let mut cache = PROBES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(key, verdict);
    verdict
}

impl LibraryFacts {
    pub fn load(library_dir: PathBuf, password: Option<String>) -> Self {
        let ids = library::load_index(&library_dir).into_iter().map(|entry| entry.app_id).collect();
        Self { library_dir, ids, password }
    }

    #[cfg(test)]
    fn with_ids(library_dir: PathBuf, ids: &[&str]) -> Self {
        Self { library_dir, ids: ids.iter().map(|id| id.to_string()).collect(), password: None }
    }

    #[cfg(test)]
    fn with_password(mut self, password: &str) -> Self {
        self.password = Some(password.to_string());
        self
    }

    /// Is this file still worth offering? A `.lua` whose game is in the library is not. An
    /// archive is not either when the stored patch of that game is the very same file; when it
    /// is another file, it is offered and flagged `replaces`.
    pub fn judge(&self, item: &mut DownloadFound) -> bool {
        let Some(app_id) = item.app_id.as_deref() else {
            return true;
        };
        if item.kind == "lua" {
            return !self.ids.contains(app_id);
        }
        let stored = crate::fixes::archive_path(&self.library_dir, app_id);
        if stored.is_file() {
            if same_content(&stored, Path::new(&item.path)) {
                return false;
            }
            item.replaces = true;
        }
        // Still offered: say whether the default password opens it, so a protected archive does
        // not surprise the user at install time.
        item.archive_check = Some(probe_cached(Path::new(&item.path), item.size, self.password.as_deref()));
        true
    }
}

/// Same length first (free), then the SHA-256: two multi-hundred-MiB archives are not hashed
/// when their sizes already differ.
fn same_content(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    matches!((archive::sha256_file(a), archive::sha256_file(b)), (Ok(x), Ok(y)) if x == y)
}

fn flavor_of(kind: &str, name: &str) -> &'static str {
    if kind == "lua" {
        "lua"
    } else {
        commands::patch_flavor(name)
    }
}

/// One file of a watched folder, as the popup shows it, or `None` when it is not something
/// the app can take (noise, an unfinished download, an archive with no AppID in its name).
fn describe(dir: &Path, name: &str, size: u64) -> Option<DownloadFound> {
    let path = dir.join(name);
    let (kind, app_id) = classify(name, size, |max| read_head(&path, max)).offer()?;
    Some(DownloadFound {
        path: path.display().to_string(),
        name: name.to_string(),
        size,
        kind: kind.to_string(),
        app_id: Some(app_id),
        flavor: flavor_of(kind, name).to_string(),
        replaces: false,
        archive_check: None,
    })
}

/// Everything in the watched folders the app could take and does not have yet, old files
/// included: the list behind « Fichiers détectés », so a file the user ignored or forgot is one
/// click away.
pub fn inventory(dirs: &[PathBuf], facts: &LibraryFacts) -> Vec<DownloadFound> {
    let mut items = Vec::new();
    for dir in dirs {
        for seen in scan(dir) {
            if let Some(mut item) = describe(dir, &seen.name, seen.size) {
                if facts.judge(&mut item) {
                    items.push(item);
                }
            }
        }
    }
    items.sort_by_key(|item| item.name.to_lowercase());
    items.dedup_by(|a, b| a.path == b.path);
    items
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

pub fn start(app: &AppHandle, state: &AppState, dirs: Vec<PathBuf>, window: Window) -> WatchStatus {
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
        let baselines = match tauri::async_runtime::spawn_blocking({
            let dirs = dirs.clone();
            move || dirs.iter().map(|dir| scan(dir)).collect::<Vec<_>>()
        })
        .await
        {
            Ok(baselines) => baselines,
            Err(error) => {
                debug!("downloads baseline task failed: {error}");
                return;
            }
        };
        // One tracker per folder: a name is only unique inside its own folder.
        let mut trackers: Vec<Tracker> = baselines.into_iter().map(Tracker::new).collect();
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
            let polled = tauri::async_runtime::spawn_blocking({
                let dirs = dirs.clone();
                move || dirs.iter().map(|dir| scan(dir)).collect::<Vec<_>>()
            })
            .await;
            let Ok(snapshots) = polled else {
                debug!("downloads scan task failed");
                return;
            };
            let mut ready: Vec<(PathBuf, Seen)> = Vec::new();
            for ((dir, snapshot), tracker) in dirs.iter().zip(snapshots).zip(trackers.iter_mut()) {
                for seen in tracker.observe(snapshot) {
                    ready.push((dir.clone(), seen));
                }
            }
            let library_dir = app
                .state::<AppState>()
                .config
                .lock()
                .map(|cfg| (cfg.resolved_library_dir(), cfg.default_archive_password.clone()))
                .ok();
            let found: Vec<DownloadFound> = tauri::async_runtime::spawn_blocking(move || {
                // What the library already holds is not offered again.
                let facts = if ready.is_empty() {
                    None
                } else {
                    library_dir.map(|(dir, password)| LibraryFacts::load(dir, password))
                };
                ready
                    .into_iter()
                    .filter_map(|(dir, seen)| {
                        let mut item = describe(&dir, &seen.name, seen.size)?;
                        match facts.as_ref() {
                            Some(facts) if !facts.judge(&mut item) => None,
                            _ => Some(item),
                        }
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
            flavor: "lua".to_string(),
            replaces: false,
            archive_check: None,
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
        assert_eq!(title, "Un fichier détecté");
        assert!(body.starts_with("A\n"), "{body}");
        let (title, body) = toast_texts("fr", &names);
        assert_eq!(title, "2 fichiers détectés");
        assert!(body.starts_with("A, B\n"));
        let (title, _) = toast_texts("en", &names);
        assert_eq!(title, "2 files were detected");
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

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("luavault-downloads-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rar(extra: u8) -> Vec<u8> {
        let mut bytes = b"Rar!\x1a\x07\x01\x00".to_vec();
        bytes.extend_from_slice(&[extra; 64]);
        bytes
    }

    #[test]
    fn a_bypass_archive_is_recognised_and_labelled() {
        let dir = scratch("bypass");
        std::fs::write(dir.join("271590_bypass.rar"), rar(1)).unwrap();
        std::fs::write(dir.join("1790600_online_fix.zip"), b"PK\x03\x04and the rest").unwrap();
        let bypass = describe(&dir, "271590_bypass.rar", std::fs::metadata(dir.join("271590_bypass.rar")).unwrap().len())
            .expect("a bypass named with its AppID is offered");
        assert_eq!((bypass.kind.as_str(), bypass.flavor.as_str(), bypass.app_id.as_deref()), ("archive", "bypass", Some("271590")));
        let patch = describe(&dir, "1790600_online_fix.zip", 20).expect("an online patch is offered");
        assert_eq!((patch.flavor.as_str(), patch.app_id.as_deref()), ("patch", Some("1790600")));
        // The browser's duplicate counter is not a game.
        std::fs::write(dir.join("271590_bypass (1).rar"), rar(1)).unwrap();
        let again = describe(&dir, "271590_bypass (1).rar", 72).unwrap();
        assert_eq!(again.app_id.as_deref(), Some("271590"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_the_library_has_is_not_offered_again() {
        let library = scratch("facts-lib");
        let dl = scratch("facts-dl");
        let stored = crate::fixes::archive_path(&library, "271590");
        std::fs::create_dir_all(stored.parent().unwrap()).unwrap();
        std::fs::write(&stored, rar(1)).unwrap();
        std::fs::write(dl.join("271590_bypass.rar"), rar(1)).unwrap(); // the very same file
        std::fs::write(dl.join("440_bypass.rar"), rar(2)).unwrap(); // no patch stored for 440
        std::fs::write(dl.join("999_bypass.rar"), rar(3)).unwrap();
        let other = crate::fixes::archive_path(&library, "999");
        std::fs::write(&other, rar(4)).unwrap(); // same length, other content
        std::fs::write(dl.join("1174180.lua"), lua_fixture_for("1174180")).unwrap(); // in the library
        std::fs::write(dl.join("1912410.lua"), lua_fixture_for("1912410")).unwrap(); // new
        std::fs::write(dl.join("notes.txt"), "rien").unwrap();
        let facts = LibraryFacts::with_ids(library.clone(), &["1174180", "271590", "999"]);
        let items = inventory(std::slice::from_ref(&dl), &facts);
        let names: Vec<_> = items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["1912410.lua", "440_bypass.rar", "999_bypass.rar"], "{names:?}");
        let by = |n: &str| items.iter().find(|item| item.name == n).unwrap();
        assert!(!by("440_bypass.rar").replaces);
        assert!(by("999_bypass.rar").replaces, "another file for a game that has a patch replaces it");
        let _ = std::fs::remove_dir_all(library);
        let _ = std::fs::remove_dir_all(dl);
    }

    fn real_zip(path: &Path, password: Option<&str>) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let mut options = zip::write::SimpleFileOptions::default();
        if let Some(password) = password {
            options = options.with_aes_encryption(zip::AesMode::Aes256, password);
        }
        zip.start_file("OnlineFix64.dll", options).unwrap();
        zip.write_all(b"payload payload payload").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn an_offered_archive_says_whether_the_default_password_opens_it() {
        let dl = scratch("probe-dl");
        real_zip(&dl.join("500_bypass.zip"), Some("pw"));
        real_zip(&dl.join("600_bypass.zip"), None);
        std::fs::write(dl.join("700_bypass.zip"), b"PK truncated, not a zip anyone can read").unwrap();
        std::fs::write(dl.join("800.lua"), lua_fixture_for("800")).unwrap();
        let check = |facts: &LibraryFacts| {
            let items = inventory(std::slice::from_ref(&dl), facts);
            let of = |name: &str| items.iter().find(|item| item.name == name).unwrap().archive_check;
            (of("500_bypass.zip"), of("600_bypass.zip"), of("700_bypass.zip"), of("800.lua"))
        };
        let none = LibraryFacts::with_ids(scratch("probe-lib"), &[]);
        assert_eq!(
            check(&none),
            (
                Some(archive::Probe::NeedsPassword),
                Some(archive::Probe::Opens),
                Some(archive::Probe::Unreadable),
                None
            ),
            "no default password: a protected archive says so; a .lua is never probed"
        );
        let right = LibraryFacts::with_ids(scratch("probe-lib"), &[]).with_password("pw");
        assert_eq!(check(&right).0, Some(archive::Probe::Opens), "the default password opens it");
        let wrong = LibraryFacts::with_ids(scratch("probe-lib"), &[]).with_password("nope");
        assert_eq!(check(&wrong).0, Some(archive::Probe::WrongPassword), "another password does not");
        let _ = std::fs::remove_dir_all(dl);
    }

    #[test]
    fn the_inventory_reads_every_watched_folder_and_lists_a_path_once() {
        let a = scratch("inv-a");
        let b = scratch("inv-b");
        std::fs::write(a.join("111.lua"), lua_fixture_for("111")).unwrap();
        std::fs::write(b.join("222.lua"), lua_fixture_for("222")).unwrap();
        let facts = LibraryFacts::with_ids(scratch("inv-lib"), &[]);
        let items = inventory(&[a.clone(), b.clone(), a.clone()], &facts);
        let names: Vec<_> = items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["111.lua", "222.lua"]);
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }

    fn lua_fixture_for(app_id: &str) -> Vec<u8> {
        format!("-- AppID {app_id} | Generated on 2026-01-01 05:55 UTC | LuaVault\n-- Main Application\naddappid({app_id}, 1, \"340ae2fcb31a92507c1e0a9139f7cac3ab50f45a8c701f6d9f06b1793aca9981\")\n").into_bytes()
    }

    /// What the app would list for the real Downloads folder against the real library. Read-only.
    ///
    /// `cargo test live_inventory_of_the_real_downloads -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the user's real Downloads folder and library"]
    fn live_inventory_of_the_real_downloads() {
        let cfg = crate::config::AppConfig::load().unwrap_or_default();
        let dirs = commands::watch_dirs(&cfg);
        let facts = LibraryFacts::load(cfg.resolved_library_dir(), cfg.default_archive_password.clone());
        println!("watched: {dirs:?}");
        for dir in &dirs {
            for seen in scan(dir) {
                let verdict = classify(&seen.name, seen.size, |max| read_head(&dir.join(&seen.name), max));
                println!("  {:<40} {:>9} B  -> {:?}", seen.name, seen.size, verdict);
            }
        }
        for item in inventory(&dirs, &facts) {
            println!("OFFERED {:<36} {:<6} app={:?} replaces={}", item.name, item.flavor, item.app_id, item.replaces);
        }
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

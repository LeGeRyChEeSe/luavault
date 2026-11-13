//! LuaVault inside the Steam client (experimental edition).
//!
//! The Steam client's store is a web page in an embedded Chromium. When the empty file
//! `.cef-enable-remote-debugging` sits in Steam's folder, the client opens Chromium's debugging
//! protocol on `127.0.0.1:8080` at its next start (the file alone is enough, no launch option). This
//! module is the other end:
//!
//! - every second it lists the client's pages and keeps the web view showing one of Valve's sites
//!   and the main window (`client_targets`, pure);
//! - to each it gives a script (`scripts/steam-store-button.js` for the store,
//!   `scripts/steam-client-pill.js` for the window) and one function, `lvStore`, the scripts call
//!   with a short word;
//! - the function and the script are given again at every page load (`Page.domContentEventFired`):
//!   the client forgets a `Runtime.addBinding` when the view navigates;
//! - a call is believed only after the page's own host was read back through the protocol
//!   (`location.hostname`, which page code cannot forge): the function outlives a navigation, so a
//!   page of another site shown in the same view could call it.
//!
//! What a call can do is small and closed (`ClientCall`): open the source site's home page for a
//! game (the person downloads its files there, LuaVault picks them up from Downloads), take a line
//! out of the panel, show LuaVault's window. Nothing is ever downloaded from here.
//!
//! The price, said in « À l'essai »: while that file exists, any program running on this computer
//! can drive the Steam client through that port. It is opt-in, and switching it off removes the file.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use log::{debug, info, warn};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

use crate::config::{self, AppConfig};
use crate::steam_store;

/// Steam's own choice of port for `.cef-enable-remote-debugging`.
pub const DEBUG_PORT: u16 = 8080;
/// The file Steam looks for in its folder at start.
pub const MARKER_FILE: &str = ".cef-enable-remote-debugging";
/// The only host whose pages get the store button.
pub const STORE_HOST: &str = "store.steampowered.com";
/// The function handed to the pages.
pub const BINDING: &str = "lvStore";
/// How often the client's pages are listed.
pub const POLL_SECS: u64 = 1;
/// The page list is a few kilobytes; anything larger is not Steam answering.
const LIST_CAP: u64 = 512 * 1024;

/// The store page's script.
pub const STORE_SCRIPT: &str = include_str!("../../scripts/steam-store-button.js");
/// The main window's script: the « LV » pill of the title bar.
pub const WINDOW_SCRIPT: &str = include_str!("../../scripts/steam-client-pill.js");
/// The announcements' card, given to the window and to the web view alike.
pub const NOTICE_SCRIPT: &str = include_str!("../../scripts/steam-notice.js");
/// The host of the client's own interface (the main window).
pub const WINDOW_HOST: &str = "steamloopback.host";
/// The main window's title in the page list. Its listed URL is `about:blank?createflags=…`; the
/// page itself then reads `https://steamloopback.host/index.html`.
pub const WINDOW_TITLE: &str = "Steam";

/// Which page of the client: its web view (`Store`), or the main window (`Window`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Store,
    Window,
}

/// One page of the Steam client this module equips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: String,
    pub ws: String,
    pub kind: Kind,
}

/// The domains of Valve's sites the client's web view shows: the store, the community, support. It
/// is ONE view that moves from one to the other in the same page, laid over the main window.
pub const WEB_DOMAINS: [&str; 2] = ["steampowered.com", "steamcommunity.com"];

/// One of `WEB_DOMAINS` or a name under it, and nothing that merely ends like it.
pub fn is_valve_host(host: &str) -> bool {
    WEB_DOMAINS.iter().any(|domain| {
        host == *domain
            || host.strip_suffix(domain).and_then(|p| p.strip_suffix('.')).is_some_and(|labels| {
                labels.split('.').all(|l| {
                    !l.is_empty() && l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                })
            })
    })
}

/// `https://<one of Valve's sites>/…`: the client's web view, whatever site it shows now.
pub fn is_web_url(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .is_some_and(is_valve_host)
}

/// The web view and the main window out of the client's page list. The socket address must be the
/// loopback one this module asked: the list is data, and it does not get to send us anywhere else.
/// The main window is told by its title and its `about:blank?` address; what it really shows is
/// checked again in the page before anything it asks is believed (`host_answer`).
pub fn client_targets(list: &str) -> Vec<Target> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(list) else {
        return Vec::new();
    };
    let prefix = format!("ws://127.0.0.1:{DEBUG_PORT}/devtools/page/");
    items
        .iter()
        .filter_map(|item| {
            let field = |key: &str| item.get(key).and_then(Value::as_str);
            if field("type") != Some("page") {
                return None;
            }
            let url = field("url")?;
            let kind = if is_web_url(url) {
                Kind::Store
            } else if field("title") == Some(WINDOW_TITLE) && url.starts_with("about:blank?") {
                Kind::Window
            } else {
                return None;
            };
            let ws = field("webSocketDebuggerUrl")?;
            let id = ws.strip_prefix(&prefix)?;
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
                return None;
            }
            Some(Target { id: id.to_string(), ws: ws.to_string(), kind })
        })
        .collect()
}

/// What a page may ask, through the one function it is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientCall {
    /// Find this game on the source site: its home page opens in the browser, the person downloads
    /// the files there, and the panel follows the game.
    Find(String),
    /// Take this game's line out of the panel.
    Dismiss(String),
    /// Show LuaVault's window (the pill).
    OpenApp,
}

/// The call carried by a `Runtime.bindingCalled` event for our function, under the strict rule.
pub fn binding_call(message: &Value) -> Option<ClientCall> {
    if message.get("method")?.as_str()? != "Runtime.bindingCalled" {
        return None;
    }
    let params = message.get("params")?;
    if params.get("name")?.as_str()? != BINDING {
        return None;
    }
    let payload = params.get("payload")?.as_str()?;
    if payload == "open" {
        return Some(ClientCall::OpenApp);
    }
    if let Some(id) = payload.strip_prefix("dismiss:") {
        return steam_store::parse_app_id(id).map(ClientCall::Dismiss);
    }
    if let Some(id) = payload.strip_prefix("find:") {
        return steam_store::parse_app_id(id).map(ClientCall::Find);
    }
    None
}

/// Where the page must be for this call to be believed: the main window on the client's own
/// interface; a store call on the store itself; showing the window from any of Valve's sites.
pub fn host_fits(kind: Kind, call: &ClientCall, host: &str) -> bool {
    match kind {
        // The window only ever shows the application: each check holds on its own, `allowed` too.
        Kind::Window => host == WINDOW_HOST && *call == ClientCall::OpenApp,
        Kind::Store => match call {
            ClientCall::Find(_) | ClientCall::Dismiss(_) => host == STORE_HOST,
            ClientCall::OpenApp => is_valve_host(host),
        },
    }
}

/// What a page of this kind may ask. A store page finds games and dismisses its panel's lines; the
/// main window shows the application. Neither asks what belongs to the other.
pub fn allowed(kind: Kind, call: &ClientCall) -> bool {
    match call {
        ClientCall::OpenApp => true,
        ClientCall::Find(_) | ClientCall::Dismiss(_) => kind == Kind::Store,
    }
}

/// The answer to the host question (`id` is the one the question was sent with), judged for the
/// call it was asked for (`host_fits`).
pub fn host_answer(message: &Value, id: u64, kind: Kind, call: &ClientCall) -> Option<bool> {
    if message.get("id")?.as_u64()? != id {
        return None;
    }
    let value = message.pointer("/result/result/value").and_then(Value::as_str);
    Some(value.is_some_and(|host| host_fits(kind, call, host)))
}

// ------------------------------------------------------------ what the panel shows

/// One game in the panel of the store (`lvStoreJob` in the store script).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Job {
    #[serde(rename = "appId")]
    pub app_id: String,
    pub name: String,
    /// `busy`, `ok`, `warn`, `error` or `muted`: the dot, and a spinner for `busy`.
    pub tone: &'static str,
    /// Already in the person's language, set as text by the page.
    pub line: String,
    /// « Installer »: the `.lua` is where Steam reads it and Steam does not have the game.
    pub install: bool,
    /// « Bibliothèque »: the game is in Steam's library.
    pub library: bool,
    /// « Ouvrir le site »: open the source site again.
    pub site: bool,
    #[serde(skip)]
    pub at: i64,
}

/// Lines kept in the panel, and how long a finished one stays.
pub const MAX_JOBS: usize = 8;
pub const JOB_TTL_SECS: i64 = 30 * 60;

static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());
static PROGRESS: LazyLock<broadcast::Sender<String>> = LazyLock::new(|| broadcast::channel(64).0);

/// Store pages of the client this instance is attached to right now.
static PAGES: AtomicUsize = AtomicUsize::new(0);
/// Expressions for the main window only.
static WINDOW_CALLS: LazyLock<broadcast::Sender<String>> = LazyLock::new(|| broadcast::channel(64).0);
/// The client's main window this instance is attached to right now (0 or 1).
static WINDOWS: AtomicUsize = AtomicUsize::new(0);

/// An announcement shown inside Steam (`lvNotice`, `scripts/steam-notice.js`), in the style of
/// Steam's own notifications, instead of a notification of Windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub title: String,
    pub line: String,
    /// `ok`, `warn`, `error` or `info`: the colour of the card's edge.
    pub tone: &'static str,
    /// The game, for its picture; `None` for no picture.
    #[serde(rename = "appId")]
    pub app_id: Option<String>,
    /// A click on the card shows LuaVault's window.
    pub open: bool,
}

/// Pure. A panel tone as a notice tone.
pub fn notice_tone(job_tone: &str) -> &'static str {
    match job_tone {
        "ok" => "ok",
        "warn" => "warn",
        "error" => "error",
        _ => "info",
    }
}

/// What a page runs to show it. JSON from serde: no text can leave its string.
pub fn notice_call(notice: &Notice) -> String {
    let json = serde_json::to_string(notice).unwrap_or_else(|_| "null".to_string());
    format!("typeof lvNotice === \"function\" && lvNotice({json});")
}

/// Whether the client's main window is equipped.
pub fn window_open() -> bool {
    WINDOWS.load(Ordering::SeqCst) > 0
}

/// Show it in Steam: in the main window and in the web view (the view lies over the window, only
/// the one in front is seen). `false`: nowhere to show it, the caller falls back on Windows.
pub fn notify_in_steam(notice: &Notice) -> bool {
    if !window_open() && !pages_open() {
        return false;
    }
    let call = notice_call(notice);
    let _ = PROGRESS.send(call.clone());
    let _ = WINDOW_CALLS.send(call);
    true
}

struct WindowGuard;

impl WindowGuard {
    fn enter() -> WindowGuard {
        WINDOWS.fetch_add(1, Ordering::SeqCst);
        WindowGuard
    }
}

impl Drop for WindowGuard {
    fn drop(&mut self) {
        WINDOWS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One line per game, the newest last, `MAX_JOBS` at most.
pub fn merge_job(list: &mut Vec<Job>, job: Job) {
    list.retain(|j| j.app_id != job.app_id);
    list.push(job);
    if list.len() > MAX_JOBS {
        let extra = list.len() - MAX_JOBS;
        list.drain(..extra);
    }
}

/// The lines still worth showing.
pub fn live_jobs(list: &[Job], now: i64) -> Vec<Job> {
    list.iter().filter(|j| now.saturating_sub(j.at) < JOB_TTL_SECS).cloned().collect()
}

/// What a page runs to show one new step. JSON from serde: no text can leave its string.
pub fn job_call(job: &Job) -> String {
    let json = serde_json::to_string(job).unwrap_or_else(|_| "null".to_string());
    format!("typeof lvStoreJob === \"function\" && lvStoreJob({json});")
}

/// Show a step in every store page of the client, now and on the pages opened later.
pub fn publish(mut job: Job) {
    job.at = chrono::Utc::now().timestamp();
    let call = job_call(&job);
    merge_job(&mut JOBS.lock().unwrap(), job);
    let _ = PROGRESS.send(call);
}

/// The panel's line for this game, if it still shows one.
pub fn job_of(app_id: &str) -> Option<Job> {
    let now = chrono::Utc::now().timestamp();
    live_jobs(&JOBS.lock().unwrap(), now).into_iter().find(|j| j.app_id == app_id)
}

pub fn dismiss(app_id: &str) {
    JOBS.lock().unwrap().retain(|j| j.app_id != app_id);
}

/// Whether a store page of the client is there to show what happens.
pub fn pages_open() -> bool {
    PAGES.load(Ordering::SeqCst) > 0
}

struct PageGuard;

impl PageGuard {
    fn enter() -> PageGuard {
        PAGES.fetch_add(1, Ordering::SeqCst);
        PageGuard
    }
}

impl Drop for PageGuard {
    fn drop(&mut self) {
        PAGES.fetch_sub(1, Ordering::SeqCst);
    }
}

// ------------------------------------------------------ what LuaVault already has

/// How a game of the library stands in Steam, as the page names it. `None` when Steam's state
/// cannot be read: the page then keeps its ordinary button.
pub fn known_state(side: steam_store::SteamSide) -> Option<&'static str> {
    match side {
        steam_store::SteamSide::Ready => Some("installed"),
        steam_store::SteamSide::Downloading => Some("downloading"),
        steam_store::SteamSide::Absent => Some("absent"),
        steam_store::SteamSide::Unknown => None,
    }
}

/// The games of the library whose `.lua` is where Steam reads it, and how Steam holds each.
/// Blocking file reads: called on a blocking thread.
pub fn known_games(cfg: &AppConfig) -> BTreeMap<String, &'static str> {
    let mut out = BTreeMap::new();
    let Some(steam) = crate::commands::resolve_steam(cfg) else { return out };
    let lua_dir = crate::detect::lua_dir(&steam);
    for entry in crate::library::load_index(&cfg.resolved_library_dir()) {
        if steam_store::parse_app_id(&entry.app_id).is_none() {
            continue;
        }
        if !lua_dir.join(format!("{}.lua", entry.app_id)).is_file() {
            continue;
        }
        let game = crate::vdf::locate_game(&steam, &entry.app_id);
        if let Some(state) = known_state(steam_store::steam_side(true, Some(&game))) {
            out.insert(entry.app_id.clone(), state);
        }
    }
    out
}

/// Bumped whenever what the pages show must be rebuilt (a game arrived, the switch moved): every
/// page lets go and is equipped again within `POLL_SECS`.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// A game was added or installed: the pages are equipped again with what LuaVault now has.
pub fn refresh() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// The store script preceded by what it needs from the application: the games already there and
/// the panel's current lines, as JSON built by serde (no text can end the statement it sits in).
pub fn store_script_with(known: &BTreeMap<String, &'static str>, jobs: &[Job]) -> String {
    let known = serde_json::to_string(known).unwrap_or_else(|_| "{}".to_string());
    let jobs = serde_json::to_string(jobs).unwrap_or_else(|_| "[]".to_string());
    format!("var lvStoreKnown = {known};\nvar lvStoreJobs = {jobs};\n{STORE_SCRIPT}")
}

/// What is sent to a page when it is first seen: page events on (they tell us the view loaded
/// something else), then the function and the script.
pub fn opening_commands(script: &str) -> Vec<Value> {
    let mut commands = vec![json!({ "id": 1, "method": "Page.enable" })];
    commands.extend(arming_commands(script));
    commands
}

/// The function, then the script that calls it: sent at first sight and again at every page load.
pub fn arming_commands(script: &str) -> Vec<Value> {
    vec![
        json!({ "id": 2, "method": "Runtime.addBinding", "params": { "name": BINDING } }),
        json!({ "id": 3, "method": "Runtime.evaluate", "params": { "expression": script } }),
    ]
}

async fn script_for(app: &AppHandle, kind: Kind) -> String {
    match kind {
        Kind::Store => {
            let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
            let known = tauri::async_runtime::spawn_blocking(move || known_games(&cfg)).await.unwrap_or_default();
            let jobs = live_jobs(&JOBS.lock().unwrap(), chrono::Utc::now().timestamp());
            format!("{}\n{NOTICE_SCRIPT}", store_script_with(&known, &jobs))
        }
        Kind::Window => format!("{WINDOW_SCRIPT}\n{NOTICE_SCRIPT}"),
    }
}

/// The view finished parsing a new page: time to arm it again.
pub fn page_loaded(message: &Value) -> bool {
    message.get("method").and_then(Value::as_str) == Some("Page.domContentEventFired")
}

fn host_question(id: u64) -> Value {
    json!({ "id": id, "method": "Runtime.evaluate", "params": { "expression": "location.hostname", "returnByValue": true } })
}

/// Whether this build and these settings want LuaVault in the client.
pub fn wanted(cfg: &AppConfig, portable: bool, experimental: bool) -> bool {
    experimental && !portable && cfg.in_steam == Some(true)
}

/// Pure. Whether the file in Steam's folder is this app's to remove when it is uninstalled: only
/// when the person ticked « LuaVault dans Steam » here. Other tools write the same file (Steam
/// documents it for skin and plugin loaders): one LuaVault did not write stays.
pub fn marker_is_ours(cfg: &AppConfig) -> bool {
    cfg.in_steam == Some(true)
}

pub fn marker_path(steam: &Path) -> PathBuf {
    steam.join(MARKER_FILE)
}

/// Create or remove the file Steam reads at start. Removing a file that is not there is a success.
pub fn set_marker(steam: &Path, on: bool) -> std::io::Result<()> {
    let path = marker_path(steam);
    if on {
        if !path.exists() {
            std::fs::write(&path, b"")?;
        }
        return Ok(());
    }
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

async fn list_pages(http: &reqwest::Client) -> Option<String> {
    let resp = http
        .get(format!("http://127.0.0.1:{DEBUG_PORT}/json"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    let bytes = crate::update::read_capped(resp, LIST_CAP).await.ok()?;
    String::from_utf8(bytes).ok()
}

async fn send(socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, command: &Value) -> bool {
    socket.send(Message::Text(command.to_string().into())).await.is_ok()
}

/// Stay on one page until it closes: give it the script, then listen for the calls.
async fn attach(app: AppHandle, target: Target) {
    let Ok((mut socket, _)) = tokio_tungstenite::connect_async(target.ws.as_str()).await else {
        return;
    };
    let generation = GENERATION.load(Ordering::SeqCst);
    let _page = (target.kind == Kind::Store).then(PageGuard::enter);
    let _window = (target.kind == Kind::Window).then(WindowGuard::enter);
    let mut progress = PROGRESS.subscribe();
    let mut window_calls = WINDOW_CALLS.subscribe();
    for command in opening_commands(&script_for(&app, target.kind).await) {
        if !send(&mut socket, &command).await {
            return;
        }
    }
    debug!("LuaVault dans Steam : page {:?} {} équipée", target.kind, target.id);
    let mut next_id: u64 = 100;
    // The call waiting for the page's host to be confirmed: (question id, call).
    let mut asked: Option<(u64, ClientCall)> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        let frame = tokio::select! {
            frame = socket.next() => frame,
            step = progress.recv() => {
                if let (Ok(call), Kind::Store) = (step, target.kind) {
                    next_id += 1;
                    let command = json!({ "id": next_id, "method": "Runtime.evaluate", "params": { "expression": call } });
                    if !send(&mut socket, &command).await {
                        return;
                    }
                }
                continue;
            }
            call = window_calls.recv() => {
                if let (Ok(call), Kind::Window) = (call, target.kind) {
                    next_id += 1;
                    let command = json!({ "id": next_id, "method": "Runtime.evaluate", "params": { "expression": call } });
                    if !send(&mut socket, &command).await {
                        return;
                    }
                }
                continue;
            }
            _ = tick.tick() => {
                let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
                if GENERATION.load(Ordering::SeqCst) != generation || !wanted(&cfg, config::is_portable(), config::EXPERIMENTAL) {
                    return;
                }
                continue;
            }
        };
        let Some(Ok(frame)) = frame else { return };
        let Message::Text(text) = frame else { continue };
        let Ok(message) = serde_json::from_str::<Value>(text.as_str()) else { continue };
        if page_loaded(&message) {
            for command in arming_commands(&script_for(&app, target.kind).await) {
                if !send(&mut socket, &command).await {
                    return;
                }
            }
            continue;
        }
        if let Some(call) = binding_call(&message).filter(|c| allowed(target.kind, c)) {
            next_id += 1;
            asked = Some((next_id, call));
            if !send(&mut socket, &host_question(next_id)).await {
                return;
            }
            continue;
        }
        let Some((id, call)) = asked.clone() else { continue };
        match host_answer(&message, id, target.kind, &call) {
            Some(true) => {
                asked = None;
                match call {
                    ClientCall::Find(app_id) => {
                        info!("LuaVault dans Steam : recherche de {app_id} sur le site");
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            steam_store::find(&handle, &app_id).await;
                        });
                    }
                    ClientCall::Dismiss(app_id) => dismiss(&app_id),
                    ClientCall::OpenApp => {
                        info!("LuaVault dans Steam : fenêtre demandée depuis Steam");
                        crate::guard_service::show_main(&app);
                    }
                }
            }
            Some(false) => {
                asked = None;
                warn!("LuaVault dans Steam : appel ignoré, la page n'est pas celle attendue");
            }
            None => {}
        }
    }
}

/// The loop of a running instance: while the switch is on, equip the client's pages.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Ok(http) = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .no_gzip()
            .no_brotli()
            .no_zstd()
            .build()
        else {
            return;
        };
        let attached: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        loop {
            let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
            if wanted(&cfg, config::is_portable(), config::EXPERIMENTAL) {
                if let Some(list) = list_pages(&http).await {
                    for target in client_targets(&list) {
                        if !attached.lock().unwrap().insert(target.id.clone()) {
                            continue;
                        }
                        let (app, attached) = (app.clone(), attached.clone());
                        tauri::async_runtime::spawn(async move {
                            let id = target.id.clone();
                            attach(app, target).await;
                            attached.lock().unwrap().remove(&id);
                        });
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(POLL_SECS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // Copied from the real client's answer (ids shortened), not written from the code.
    const REAL_LIST: &str = r#"[ {
   "description": "",
   "devtoolsFrontendUrl": "/devtools/inspector.html?ws=127.0.0.1:8080/devtools/page/6A2F9C0D11E34B7A9C55D2E81F0A7B3C",
   "id": "6A2F9C0D11E34B7A9C55D2E81F0A7B3C",
   "title": "Subnautica sur Steam",
   "type": "page",
   "url": "https://store.steampowered.com/app/264710/Subnautica/",
   "webSocketDebuggerUrl": "ws://127.0.0.1:8080/devtools/page/6A2F9C0D11E34B7A9C55D2E81F0A7B3C"
}, {
   "description": "",
   "id": "0B1C2D3E4F5061728394A5B6C7D8E9F0",
   "title": "SharedJSContext",
   "type": "page",
   "url": "https://steamloopback.host/index.html?LOCAL_HOSTNAME=PC&LANGUAGE=french&IN_CLIENT=true",
   "webSocketDebuggerUrl": "ws://127.0.0.1:8080/devtools/page/0B1C2D3E4F5061728394A5B6C7D8E9F0"
}, {
   "description": "",
   "id": "11112222333344445555666677778888",
   "title": "Steam",
   "type": "page",
   "url": "about:blank?createflags=274&minwidth=1010&minheight=600&pid=0&browser=-1&browserType=4",
   "webSocketDebuggerUrl": "ws://127.0.0.1:8080/devtools/page/11112222333344445555666677778888"
} ]"#;

    #[test]
    fn steam_client_notices_travel_as_json_and_need_an_equipped_page() {
        let notice = Notice {
            title: "Détection automatique : \"x\";</script>".into(),
            line: "l".into(),
            tone: notice_tone("busy"),
            app_id: Some("413150".into()),
            open: true,
        };
        let call = notice_call(&notice);
        assert!(call.starts_with("typeof lvNotice === \"function\" && lvNotice({"));
        let json: Value = serde_json::from_str(call.trim_start_matches("typeof lvNotice === \"function\" && lvNotice(").trim_end_matches(");")).unwrap();
        assert_eq!(json["title"], "Détection automatique : \"x\";</script>");
        assert_eq!(json["tone"], "info");
        assert_eq!(json["appId"], "413150");
        assert_eq!(json["open"], true);
        assert_eq!(notice_tone("ok"), "ok");
        assert_eq!(notice_tone("error"), "error");
        assert!(!notify_in_steam(&notice), "nothing equipped in a test: Windows says it");
        assert!(NOTICE_SCRIPT.contains("window.lvNotice = show"));
    }

    fn called(payload: &str) -> Value {
        json!({ "method": "Runtime.bindingCalled", "params": { "name": BINDING, "payload": payload } })
    }

    #[test]
    fn steam_client_keeps_the_web_view_and_the_window_only() {
        let found = client_targets(REAL_LIST);
        let kinds: Vec<_> = found.iter().map(|t| (t.id.as_str(), t.kind)).collect();
        assert_eq!(
            kinds,
            vec![("6A2F9C0D11E34B7A9C55D2E81F0A7B3C", Kind::Store), ("11112222333344445555666677778888", Kind::Window)],
            "the shared context is left alone"
        );
        assert!(client_targets("pas du json").is_empty());
        assert!(client_targets("{}").is_empty());
    }

    #[test]
    fn steam_client_refuses_a_socket_the_list_points_elsewhere() {
        for ws in [
            "ws://evil.example/devtools/page/ABC",
            "ws://127.0.0.1:9999/devtools/page/ABC",
            "ws://127.0.0.1:8080/devtools/page/AB/../../x",
            "ws://127.0.0.1:8080/devtools/page/",
        ] {
            let list = format!(r#"[{{"type":"page","url":"https://store.steampowered.com/app/1/","webSocketDebuggerUrl":"{ws}"}}]"#);
            assert!(client_targets(&list).is_empty(), "{ws}");
        }
        let fake_window = r#"[{"type":"page","title":"Steam","url":"https://evil.example/","webSocketDebuggerUrl":"ws://127.0.0.1:8080/devtools/page/ABC"}]"#;
        assert!(client_targets(fake_window).is_empty(), "the title alone is not enough");
    }

    #[test]
    fn steam_client_valve_hosts_are_exact() {
        for host in ["store.steampowered.com", "steamcommunity.com", "help.steampowered.com"] {
            assert!(is_valve_host(host), "{host}");
        }
        for host in ["evilsteampowered.com", "steampowered.com.evil.example", "store.steampowered.com.", "STORE.steampowered.com", ".steampowered.com"] {
            assert!(!is_valve_host(host), "{host}");
        }
    }

    #[test]
    fn steam_client_reads_only_its_own_closed_set_of_calls() {
        assert_eq!(binding_call(&called("find:264710")), Some(ClientCall::Find("264710".into())));
        assert_eq!(binding_call(&called("dismiss:264710")), Some(ClientCall::Dismiss("264710".into())));
        assert_eq!(binding_call(&called("open")), Some(ClientCall::OpenApp));
        for bad in ["find:0264710", "find:", "find:12345678901", "find:264710,848450", "find:1&x=2", "264710", "hide", "open ", "OPEN"] {
            assert_eq!(binding_call(&called(bad)), None, "{bad}");
        }
        let other = json!({ "method": "Runtime.bindingCalled", "params": { "name": "autre", "payload": "open" } });
        assert_eq!(binding_call(&other), None, "another function's call");
    }

    #[test]
    fn steam_client_believes_a_call_only_from_where_it_belongs() {
        let find = ClientCall::Find("264710".into());
        assert!(host_fits(Kind::Store, &find, STORE_HOST));
        assert!(!host_fits(Kind::Store, &find, "steamcommunity.com"), "the store button lives on the store");
        assert!(!host_fits(Kind::Window, &find, WINDOW_HOST));
        assert!(!allowed(Kind::Window, &find), "the window never finds games");
        assert!(host_fits(Kind::Store, &ClientCall::OpenApp, "steamcommunity.com"));
        assert!(!host_fits(Kind::Store, &ClientCall::OpenApp, "evil.example"));
        assert!(host_fits(Kind::Window, &ClientCall::OpenApp, WINDOW_HOST));
        assert!(!host_fits(Kind::Window, &ClientCall::OpenApp, STORE_HOST));
        let answer = json!({ "id": 7, "result": { "result": { "type": "string", "value": STORE_HOST } } });
        assert_eq!(host_answer(&answer, 7, Kind::Store, &find), Some(true));
        assert_eq!(host_answer(&answer, 8, Kind::Store, &find), None, "another question's answer");
        let forged = json!({ "id": 7, "result": { "result": { "type": "string", "value": "store.steampowered.com.evil.example" } } });
        assert_eq!(host_answer(&forged, 7, Kind::Store, &find), Some(false));
    }

    #[test]
    fn steam_client_panel_lines_travel_as_json() {
        let job = Job {
            app_id: "264710".into(),
            name: "Subnautica </script><img src=x>".into(),
            tone: "busy",
            line: "\"ligne\"".into(),
            install: false,
            library: false,
            site: true,
            at: 0,
        };
        let call = job_call(&job);
        assert!(call.starts_with("typeof lvStoreJob === \"function\" && lvStoreJob({"), "{call}");
        assert!(call.contains("\\\"ligne\\\"") && call.contains("\"site\":true") && !call.contains("\"at\""), "{call}");
        let script = store_script_with(&BTreeMap::from([("264710".to_string(), "absent")]), &[job]);
        assert!(script.starts_with("var lvStoreKnown = {\"264710\":\"absent\"};\nvar lvStoreJobs = [{"), "{script}");
    }

    #[test]
    fn steam_client_panel_keeps_one_line_per_game_and_forgets_old_ones() {
        let mk = |id: &str, at: i64| Job { app_id: id.into(), name: id.into(), tone: "ok", line: String::new(), install: false, library: false, site: false, at };
        let mut list = Vec::new();
        for i in 1..=10 {
            merge_job(&mut list, mk(&i.to_string(), 0));
        }
        merge_job(&mut list, mk("5", 1));
        assert_eq!(list.len(), MAX_JOBS);
        assert_eq!(list.last().unwrap().app_id, "5", "the newest last");
        assert_eq!(list.iter().filter(|j| j.app_id == "5").count(), 1);
        assert!(live_jobs(&[mk("1", 0)], JOB_TTL_SECS).is_empty());
        assert_eq!(live_jobs(&[mk("1", 0)], JOB_TTL_SECS - 1).len(), 1);
    }

    #[test]
    fn steam_client_wants_the_switch_the_experimental_and_an_installed_edition() {
        let on = AppConfig { in_steam: Some(true), ..Default::default() };
        assert!(wanted(&on, false, true));
        assert!(!wanted(&on, true, true), "portable");
        assert!(!wanted(&on, false, false), "stable");
        assert!(!wanted(&AppConfig::default(), false, true), "off unless ticked");
        assert!(marker_is_ours(&on));
        assert!(!marker_is_ours(&AppConfig::default()), "a file another tool wrote stays");
    }

    #[test]
    fn steam_client_marker_is_written_and_removed_quietly() {
        let dir = std::env::temp_dir().join(format!("lv_marker_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        set_marker(&dir, true).unwrap();
        assert!(marker_path(&dir).is_file());
        set_marker(&dir, true).unwrap();
        set_marker(&dir, false).unwrap();
        assert!(!marker_path(&dir).exists());
        set_marker(&dir, false).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

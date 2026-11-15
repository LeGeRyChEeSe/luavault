//! The interactive guide, drawn inside the Steam client.
//!
//! The engine lives in the window's front (`src/lib/steam-guide/`): its texts come from the same
//! catalogues as the rest of the app, and that window runs, shown or hidden, in every instance that
//! equips Steam. This module is the relay, both ways, and checks everything that crosses:
//!
//! - **from Steam**: the page script (`scripts/steam-guide.js`) calls the one binding with a
//!   `guide:` word (`GuideCall::parse`, closed sets, strict AppIDs). `steam_client` believes it
//!   only after the page's host was read back, then hands it here and it becomes the event
//!   `steam-guide://call` for the engine. What the client does in the meantime (a « Trouver sur
//!   le site » click, a line of the store panel) is relayed too (`steam-guide://fetch`,
//!   `steam-guide://job`): the guide listens to the real gestures, it never replaces them.
//! - **to Steam**: the engine sends one `View` per step (`steam_guide_show`). It is sanitized
//!   (`View::sanitized`: targets and places of a closed set, texts without control characters and
//!   capped, at most three choices with strict ids), kept (`CURRENT`: a page loaded later gets it
//!   with its script) and sent as JSON built by serde to the main window and the store pages,
//!   whose script sets every text with `textContent`.
//!
//! Nothing here writes anything: the guide's gestures are the real buttons' own calls.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::steam_client::{self, Kind};

/// The page script, given to the main window and to the web view after their own scripts.
pub const SCRIPT: &str = include_str!("../../scripts/steam-guide.js");

/// What the page script can point at. Each name is resolved in the page (`TARGETS` of the
/// script): our own nodes (`data-lv-*`), plus three anchors read on the real client.
pub const TARGETS: &[&str] = &[
    "pill",
    "tab-store",
    "tab-library",
    "settings",
    "store-search",
    "store-find",
    "store-owned",
    "store-panel",
    "store-job",
    "store-install",
    "play-button",
    "play-block",
    "play-manage",
    "play-menu",
    "list-dot",
];

/// The views a step is drawn in: the client's main window, or the web view laid over it.
pub const WHERE: &[&str] = &["window", "web", "any"];

pub const MAX_CHOICES: usize = 3;
const TEXT_MAX: usize = 400;
const LABEL_MAX: usize = 60;
const SEGMENTS_MAX: usize = 16;

/// Where a page says the client is. A web page speaks for itself, the window for what it shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "place", rename_all = "kebab-case")]
pub enum Place {
    /// A game's page of the store: `owned` = Steam's own « already in your library » banner,
    /// `known` = LuaVault holds it already (`lvStoreKnown`).
    StoreApp {
        #[serde(rename = "appId")]
        app_id: String,
        owned: bool,
        known: bool,
    },
    /// A bundle or a package page: not one game.
    StoreBundle,
    /// Any other page of the store (home, search).
    Store,
    /// Another of Valve's sites (community, profile).
    Web,
    /// The main window shows its web view (the store and the others): what it says is the page's.
    WebShown,
    /// The library shows this game's page (play bar found).
    LibraryApp {
        #[serde(rename = "appId")]
        app_id: String,
    },
    /// The main window shows something else.
    Window,
}

impl Place {
    fn parse(text: &str) -> Option<Place> {
        match text {
            "store-bundle" => return Some(Place::StoreBundle),
            "store" => return Some(Place::Store),
            "web" => return Some(Place::Web),
            "web-shown" => return Some(Place::WebShown),
            "window" => return Some(Place::Window),
            _ => {}
        }
        if let Some(rest) = text.strip_prefix("store-app:") {
            let (id, flags) = rest.split_once(':').unwrap_or((rest, ""));
            if !flags.chars().all(|c| c == 'o' || c == 'k') || flags.len() > 2 {
                return None;
            }
            let app_id = crate::steam_store::parse_app_id(id)?;
            return Some(Place::StoreApp { app_id, owned: flags.contains('o'), known: flags.contains('k') });
        }
        if let Some(id) = text.strip_prefix("library-app:") {
            return crate::steam_store::parse_app_id(id).map(|app_id| Place::LibraryApp { app_id });
        }
        None
    }

    /// Only the web view speaks for a page of the store, only the window for the window.
    fn said_by(&self, kind: Kind) -> bool {
        match self {
            Place::StoreApp { .. } | Place::StoreBundle | Place::Store | Place::Web => kind == Kind::Store,
            Place::WebShown | Place::LibraryApp { .. } | Place::Window => kind == Kind::Window,
        }
    }
}

/// What the page script may say, after `guide:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum GuideCall {
    /// The pill's « Visite guidée »: start (or show again) the guide.
    Start,
    Next,
    Back,
    Quit,
    /// One of the step's choices, by its id.
    Choose { choice: String },
    /// The step's target was clicked (its gesture is the click).
    Hit { target: String },
    /// The step's target is not on the page any more.
    Lost { target: String },
    Place(Place),
}

/// Pure. A choice id: what the engine named it, nothing else gets through.
pub fn valid_choice(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && id.len() <= 24
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl GuideCall {
    /// What follows `guide:` in the binding's payload.
    pub fn parse(rest: &str) -> Option<GuideCall> {
        match rest {
            "start" => return Some(GuideCall::Start),
            "next" => return Some(GuideCall::Next),
            "back" => return Some(GuideCall::Back),
            "quit" => return Some(GuideCall::Quit),
            _ => {}
        }
        let (word, value) = rest.split_once(':')?;
        match word {
            "choose" => valid_choice(value).then(|| GuideCall::Choose { choice: value.to_string() }),
            "hit" => TARGETS.contains(&value).then(|| GuideCall::Hit { target: value.to_string() }),
            "lost" => TARGETS.contains(&value).then(|| GuideCall::Lost { target: value.to_string() }),
            "place" => Place::parse(value).map(GuideCall::Place),
            _ => None,
        }
    }

    /// Pure. What a page of this kind may say. Both views draw the guide's bubble, so both answer
    /// it; a place is said only by the view it describes.
    pub fn allowed(&self, kind: Kind) -> bool {
        match self {
            GuideCall::Place(place) => place.said_by(kind),
            _ => kind == Kind::Store || kind == Kind::Window,
        }
    }
}

// ------------------------------------------------------------------ the step, sent to Steam

/// A piece of text: plain, or the name of an interface element (`[[…]]` in the catalogue).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub t: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ui: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub primary: bool,
}

/// One step as the page draws it. Built by the engine, every text already in the person's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// Changes at every step: the page resets what it remembers of the previous one.
    pub run: u32,
    /// The view the target (ring, veil) is in.
    #[serde(rename = "where")]
    pub where_: String,
    /// The view the bubble is in (the target may sit in the title bar while the store is in front).
    pub bubble: String,
    #[serde(default)]
    pub target: Option<String>,
    /// A target that names one game (`[data-lv-lib="<id>"]`, `[data-lv-job="<id>"]`).
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub progress: String,
    #[serde(default)]
    pub info: Vec<Segment>,
    #[serde(default)]
    pub action: Vec<Segment>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    /// « Continuer », when the step's gate is that button (empty: no button).
    #[serde(default)]
    pub next: String,
    /// « Retour » (empty: none).
    #[serde(default)]
    pub back: String,
    pub quit: String,
    /// The click on the target is the gesture: the page reports it (`guide:hit:`).
    #[serde(default)]
    pub gesture: bool,
    /// Dim all but the target, and stop the clicks elsewhere.
    #[serde(default)]
    pub veil: bool,
    /// The bubble in a corner, small (waiting for Steam, the verification panel).
    #[serde(default)]
    pub corner: bool,
}

fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

fn clean_segments(segments: &[Segment]) -> Vec<Segment> {
    let mut left = TEXT_MAX;
    let mut out = Vec::new();
    for s in segments.iter().take(SEGMENTS_MAX) {
        let t = clean(&s.t, left);
        left -= t.chars().count();
        if !t.is_empty() {
            out.push(Segment { t, ui: s.ui });
        }
        if left == 0 {
            break;
        }
    }
    out
}

impl View {
    /// Pure. The view as it may reach a page, or `None` when its frame is not ours: an unknown
    /// view or target, a forged AppID. Texts are cut, never refused (a long translation still
    /// shows); choices beyond the cap or with a forged id are dropped.
    pub fn sanitized(&self) -> Option<View> {
        if !WHERE.contains(&self.where_.as_str()) || !WHERE.contains(&self.bubble.as_str()) || self.bubble == "any" {
            return None;
        }
        if let Some(target) = &self.target {
            if !TARGETS.contains(&target.as_str()) {
                return None;
            }
        }
        let app_id = match &self.app_id {
            Some(id) => Some(crate::steam_store::parse_app_id(id)?),
            None => None,
        };
        Some(View {
            run: self.run,
            where_: self.where_.clone(),
            bubble: self.bubble.clone(),
            target: self.target.clone(),
            app_id,
            title: clean(&self.title, LABEL_MAX),
            progress: clean(&self.progress, LABEL_MAX),
            info: clean_segments(&self.info),
            action: clean_segments(&self.action),
            choices: self
                .choices
                .iter()
                .filter(|c| valid_choice(&c.id))
                .take(MAX_CHOICES)
                .map(|c| Choice { id: c.id.clone(), label: clean(&c.label, LABEL_MAX), primary: c.primary })
                .collect(),
            next: clean(&self.next, LABEL_MAX),
            back: clean(&self.back, LABEL_MAX),
            quit: clean(&self.quit, LABEL_MAX),
            gesture: self.gesture && self.target.is_some(),
            veil: self.veil && !self.corner,
            corner: self.corner,
        })
    }
}

/// What a page runs to draw a step (`null`: the guide is over). JSON from serde.
pub fn guide_call(view: Option<&View>) -> String {
    let json = view.and_then(|v| serde_json::to_string(v).ok()).unwrap_or_else(|| "null".to_string());
    format!("typeof lvGuide === \"function\" && lvGuide({json});")
}

/// The step on screen now, as JSON: a page loaded later (the store moves from page to page) gets
/// it with its script.
static CURRENT: Mutex<Option<String>> = Mutex::new(None);
/// The instance's handle, for the relays that have none (`steam_client::publish`).
static APP: OnceLock<AppHandle> = OnceLock::new();

pub fn set_app(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// The page script preceded by the step on screen.
pub fn script() -> String {
    let current = CURRENT.lock().unwrap().clone().unwrap_or_else(|| "null".to_string());
    format!("var lvGuideView = {current};\n{SCRIPT}")
}

/// Whether the heartbeat runs.
static TICKING: AtomicBool = AtomicBool::new(false);
/// How often the engine is woken while a step is on screen.
pub const TICK_SECS: u64 = 3;

/// The engine waits on Steam (an install, a download) with nothing to tell it: a hidden window's
/// timers sleep, its events do not. So while a step is on screen, this wakes it every few seconds
/// (`steam-guide://tick`), and stops by itself once the guide is taken away.
fn ensure_heartbeat(app: &AppHandle) {
    if TICKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
            if CURRENT.lock().unwrap().is_none() {
                TICKING.store(false, Ordering::SeqCst);
                return;
            }
            let _ = app.emit("steam-guide://tick", ());
        }
    });
}

/// Draw this step in Steam, or take the guide away (`None`).
pub fn show(view: Option<View>) -> Result<(), String> {
    let view = match view {
        Some(v) => Some(v.sanitized().ok_or("étape du guide refusée")?),
        None => None,
    };
    *CURRENT.lock().unwrap() = view.as_ref().and_then(|v| serde_json::to_string(v).ok());
    let call = guide_call(view.as_ref());
    steam_client::to_window(call.clone());
    steam_client::to_store(call);
    Ok(())
}

/// From `steam_client::attach`, once the page's host was read back.
pub fn received(app: &AppHandle, kind: Kind, call: GuideCall) {
    let from = if kind == Kind::Window { "window" } else { "web" };
    let _ = app.emit("steam-guide://call", serde_json::json!({ "from": from, "call": call }));
}

/// A « Trouver sur le site » click from a store page: the guide's button step watches for it.
pub fn fetch_seen(app: &AppHandle, app_ids: &[String]) {
    let _ = app.emit("steam-guide://fetch", serde_json::json!({ "appIds": app_ids }));
}

/// A line of the store panel changed (`steam_client::publish`).
pub fn job_seen(job: &steam_client::Job) {
    if let Some(app) = APP.get() {
        let _ = app.emit("steam-guide://job", job);
    }
}

/// Pure. The address the guide's « Ouvrir pour moi » / « Y retourner » opens, from a closed set.
pub fn place_uri(place: &str) -> Option<String> {
    match place {
        "store" => return Some("steam://store".to_string()),
        "library" => return Some("steam://nav/games".to_string()),
        _ => {}
    }
    if let Some(id) = place.strip_prefix("store-app:") {
        return crate::steam_store::parse_app_id(id).map(|id| format!("steam://store/{id}"));
    }
    if let Some(id) = place.strip_prefix("library-app:") {
        return crate::steam_store::parse_app_id(id).map(|id| format!("steam://nav/games/details/{id}"));
    }
    None
}

/// Whether the guide can be drawn in Steam now, and if not, why (a word the engine translates).
#[derive(Debug, Clone, Serialize)]
pub struct Readiness {
    pub ready: bool,
    /// `ready`, `stable` (not the experimental), `off` (integration switched off), `hidden`,
    /// `discreet`, `steam` (Steam's window not equipped: closed, or started without the port).
    pub reason: &'static str,
}

/// Pure. The reason, from the facts.
pub fn readiness(experimental: bool, wanted: bool, hidden: bool, discreet: bool, window: bool) -> Readiness {
    let reason = if !experimental {
        "stable"
    } else if !wanted {
        "off"
    } else if hidden {
        "hidden"
    } else if discreet {
        "discreet"
    } else if !window {
        "steam"
    } else {
        "ready"
    };
    Readiness { ready: reason == "ready", reason }
}

#[tauri::command]
pub fn steam_guide_ready(state: tauri::State<'_, crate::AppState>) -> Readiness {
    let cfg = state.config.lock().unwrap().clone();
    readiness(
        crate::config::EXPERIMENTAL,
        steam_client::wanted(&cfg, crate::config::is_portable(), crate::config::EXPERIMENTAL),
        crate::kill_switch::engaged(&crate::config::data_dir()),
        steam_client::discreet(&cfg),
        steam_client::window_open(),
    )
}

#[tauri::command]
pub fn steam_guide_show(app: AppHandle, view: Option<View>) -> Result<(), String> {
    let on = view.is_some();
    show(view)?;
    if on {
        ensure_heartbeat(&app);
    }
    Ok(())
}

/// The application's own window.
pub const APP_PLACE: &str = "app";

#[tauri::command]
pub fn steam_guide_open(app: AppHandle, place: String) -> Result<(), String> {
    if place == APP_PLACE {
        crate::guard_service::show_main(&app);
        return Ok(());
    }
    let uri = place_uri(&place).ok_or("lieu inconnu")?;
    crate::install::open_steam_uri(&uri).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View {
        View {
            run: 3,
            where_: "web".into(),
            bubble: "web".into(),
            target: Some("store-find".into()),
            app_id: Some("264710".into()),
            title: "Votre premier jeu".into(),
            progress: "Étape 5 sur 12".into(),
            info: vec![Segment { t: "Ce bouton n'existe que chez vous.".into(), ui: false }],
            action: vec![Segment { t: "Cliquez sur ".into(), ui: false }, Segment { t: "Récupérer".into(), ui: true }],
            choices: vec![],
            next: String::new(),
            back: "Retour".into(),
            quit: "Quitter".into(),
            gesture: true,
            veil: true,
            corner: false,
        }
    }

    #[test]
    fn steam_guide_reads_every_word_of_the_page() {
        assert_eq!(GuideCall::parse("start"), Some(GuideCall::Start));
        assert_eq!(GuideCall::parse("next"), Some(GuideCall::Next));
        assert_eq!(GuideCall::parse("back"), Some(GuideCall::Back));
        assert_eq!(GuideCall::parse("quit"), Some(GuideCall::Quit));
        assert_eq!(GuideCall::parse("choose:open-for-me"), Some(GuideCall::Choose { choice: "open-for-me".into() }));
        assert_eq!(GuideCall::parse("hit:store-find"), Some(GuideCall::Hit { target: "store-find".into() }));
        assert_eq!(GuideCall::parse("lost:play-block"), Some(GuideCall::Lost { target: "play-block".into() }));
        assert_eq!(
            GuideCall::parse("place:store-app:264710:ok"),
            Some(GuideCall::Place(Place::StoreApp { app_id: "264710".into(), owned: true, known: true }))
        );
        assert_eq!(
            GuideCall::parse("place:store-app:264710"),
            Some(GuideCall::Place(Place::StoreApp { app_id: "264710".into(), owned: false, known: false }))
        );
        assert_eq!(GuideCall::parse("place:library-app:620"), Some(GuideCall::Place(Place::LibraryApp { app_id: "620".into() })));
        for word in ["store", "store-bundle", "web", "web-shown", "window"] {
            assert!(GuideCall::parse(&format!("place:{word}")).is_some(), "{word}");
        }
    }

    #[test]
    fn steam_guide_refuses_what_is_not_its_own() {
        for bad in [
            "", "go", "choose:", "choose:Open", "choose:1st", "choose:a b", "choose:aaaaaaaaaaaaaaaaaaaaaaaaa",
            "hit:body", "hit:", "lost:#steam", "place:store-app:0264710", "place:store-app:264710:x",
            "place:store-app:264710:ooo", "place:library-app:", "place:library-app:1&2", "place:elsewhere", "next:1",
        ] {
            assert_eq!(GuideCall::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn steam_guide_a_place_is_said_by_the_view_it_describes() {
        let store = GuideCall::Place(Place::StoreApp { app_id: "264710".into(), owned: false, known: false });
        assert!(store.allowed(Kind::Store));
        assert!(!store.allowed(Kind::Window), "the window does not speak for a store page");
        let lib = GuideCall::Place(Place::LibraryApp { app_id: "264710".into() });
        assert!(lib.allowed(Kind::Window));
        assert!(!lib.allowed(Kind::Store), "a web page does not speak for the library");
        for call in [GuideCall::Next, GuideCall::Quit, GuideCall::Start, GuideCall::Hit { target: "pill".into() }] {
            assert!(call.allowed(Kind::Store) && call.allowed(Kind::Window), "{call:?}");
            assert!(!call.allowed(Kind::Shared), "the shared context is given no function: {call:?}");
        }
    }

    #[test]
    fn steam_guide_a_view_is_kept_in_its_frame() {
        assert_eq!(view().sanitized(), Some(view()));
        let mut bad = view();
        bad.target = Some("body".into());
        assert_eq!(bad.sanitized(), None, "an unknown target");
        let mut bad = view();
        bad.where_ = "elsewhere".into();
        assert_eq!(bad.sanitized(), None);
        let mut bad = view();
        bad.bubble = "any".into();
        assert_eq!(bad.sanitized(), None, "a bubble is drawn in one view");
        let mut bad = view();
        bad.app_id = Some("1;alert(1)".into());
        assert_eq!(bad.sanitized(), None, "a forged AppID");

        let mut long = view();
        long.info = vec![Segment { t: "a\u{0007}b".repeat(500), ui: false }, Segment { t: "rest".into(), ui: true }];
        long.choices = (0..5).map(|i| Choice { id: format!("c{i}"), label: "x".repeat(200), primary: i == 0 }).collect();
        long.choices.push(Choice { id: "Bad Id".into(), label: "x".into(), primary: false });
        let cut = long.sanitized().unwrap();
        let chars: usize = cut.info.iter().map(|s| s.t.chars().count()).sum();
        assert!(chars <= TEXT_MAX, "{chars}");
        assert!(cut.info.iter().all(|s| !s.t.contains('\u{0007}')), "no control character");
        assert_eq!(cut.choices.len(), MAX_CHOICES);
        assert!(cut.choices.iter().all(|c| c.label.chars().count() <= LABEL_MAX));

        let mut corner = view();
        corner.corner = true;
        assert!(!corner.sanitized().unwrap().veil, "a bubble in a corner never dims the page");
        let mut no_target = view();
        no_target.target = None;
        assert!(!no_target.sanitized().unwrap().gesture, "no target, no gesture to report");
    }

    #[test]
    fn steam_guide_the_call_cannot_leave_its_string() {
        let mut v = view();
        v.info = vec![Segment { t: "\"); alert(1); (\"</script>".into(), ui: false }];
        let call = guide_call(v.sanitized().as_ref());
        assert!(call.starts_with("typeof lvGuide === \"function\" && lvGuide({"), "{call}");
        assert!(call.contains(r#"\"); alert(1); (\"</script>"#), "{call}");
        assert_eq!(guide_call(None), "typeof lvGuide === \"function\" && lvGuide(null);");
    }

    #[test]
    fn steam_guide_opens_only_steam_addresses_of_its_set() {
        assert_eq!(place_uri("store").as_deref(), Some("steam://store"));
        assert_eq!(place_uri("library").as_deref(), Some("steam://nav/games"));
        assert_eq!(place_uri("store-app:264710").as_deref(), Some("steam://store/264710"));
        assert_eq!(place_uri("library-app:264710").as_deref(), Some("steam://nav/games/details/264710"));
        for bad in ["", "store-app:", "store-app:0", "store-app:1/../x", "https://evil.example", "library-app:1 2", "settings"] {
            assert_eq!(place_uri(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn steam_guide_readiness_names_the_first_obstacle() {
        assert_eq!(readiness(false, true, false, false, true).reason, "stable");
        assert_eq!(readiness(true, false, false, false, true).reason, "off");
        assert_eq!(readiness(true, true, true, false, true).reason, "hidden");
        assert_eq!(readiness(true, true, false, true, true).reason, "discreet");
        assert_eq!(readiness(true, true, false, false, false).reason, "steam");
        let ok = readiness(true, true, false, false, true);
        assert!(ok.ready && ok.reason == "ready");
    }
}

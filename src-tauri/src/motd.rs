//! Message of the day — a signed notice the author publishes beside the
//! releases, shown at startup until it expires. The Linux `motd`, for an app.
//!
//! Why it exists: a service outage, a broken upstream, a "do not update yet"
//! — none of these can wait for a release, and a release is the only other
//! channel the app has. The document is verified exactly like `manifest.json`
//! and never trusted beyond what the signature proves.
//!
//! Invariants, all inherited from `update.rs` and pinned by tests here:
//! - The signature is verified on the RAW BYTES before any deserialisation —
//!   a compromised server cannot show a message the author did not sign.
//! - A `schema` newer than ours is a silent abort, never an error.
//! - A failed network call is the nominal offline case: `None`, `debug!`.
//! - Nothing is read without a hard size cap, and the client performs no
//!   decompression (`update::build_http_client`).
//! - Expiry is decided by the CLIENT clock against `expires_at` when the author
//!   set one. Without it the message stays until it is withdrawn (a signed
//!   `message: null`) — the author's choice for an outage of unknown length.
//!
//! A message may carry **actions**: buttons that do the recommended thing in one
//! click. The set is closed (`ResolvedAction`) and lives in this binary. The
//! signed document only picks among them and fills their parameters, so a
//! compromised server still cannot make the app run anything the author did not
//! sign, and the author cannot ask for something this build does not know.
//! An action this client does not understand is dropped, never the message.
//!
//! A message can also be **persistent and diagnosed**. When the workaround it offers
//! is the only thing that makes the app work, insisting beats letting it be dismissed
//! for good, and coercing beats neither: nothing here ever runs without a click. The
//! app reads what is true about this Steam (`SteamFacts`) and decides, by a pure
//! function (`workaround`), whether *this* user needs the fix: they have SteamTools,
//! Steam is not already pinned, and its client is at or past the version that
//! breaks it (`affects_from`). Only then does the window refuse to be silenced and a
//! banner stay on every tab; everyone else reads an ordinary notice.
//!
//! The text itself is a small markdown dialect rendered by the frontend
//! (`src/lib/motd-markdown.ts`) without ever going through `{@html}`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use log::debug;
use serde::{Deserialize, Serialize};

use crate::steam_downgrade::{self, Pin};
use crate::update::{self, UpdateClient, RELEASE_PUBLIC_KEYS};

/// The document schema this client understands.
pub const SCHEMA: u32 = 1;

/// Hard ceiling for `motd.json`. A notice is a few paragraphs in two or three
/// languages; 64 KiB is already ten times what a reasonable one needs.
pub const MAX_MOTD_SIZE: u64 = 64 * 1024;

/// Most buttons a message can carry: a notice is a sentence and a gesture, not a menu.
pub const MAX_ACTIONS: usize = 4;

/// One button, as published. `kind` names the action; the other fields are the
/// parameters of the kinds that take one. Kept as plain data so a `kind` this
/// client has never heard of still deserialises — [`MotdAction::resolve`] is
/// where it is refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotdAction {
    /// Unique inside the message; what the UI hands back to run it.
    pub id: String,
    pub kind: String,
    /// `locale → text`. Optional: the UI has a default label per kind.
    #[serde(default)]
    pub label: HashMap<String, String>,
    /// `steam_downgrade`: a 14-digit capture date of the manifest.
    #[serde(default, skip_serializing)]
    pub snapshot: Option<String>,
    /// `steam_downgrade`: SHA-256 the manifest must have.
    #[serde(default, skip_serializing)]
    pub manifest_sha256: Option<String>,
    /// `open_url`: an `https://` address.
    #[serde(default, skip_serializing)]
    pub url: Option<String>,
    /// `steam_downgrade`: the Steam client package version (a number, like
    /// `1791918634`) from which SteamTools stops working. Optional: without it the
    /// fix is considered needed whenever Steam is not pinned. Present but not a
    /// number, the button is dropped: a typo must not turn a diagnosis into "always".
    #[serde(default, skip_serializing)]
    pub affects_from: Option<String>,
    /// `steamtools_source`: which upstream SteamTools asks for manifest request codes
    /// (`steamtools_source::SOURCES`). A name, never a URL: the binary knows the rest.
    #[serde(default, skip_serializing)]
    pub source: Option<String>,
}

/// What an action does, once its parameters are checked. The only thing the
/// backend will ever execute on a message's behalf.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedAction {
    /// Steam back to the pinned build, self-update blocked.
    SteamDowngrade(Pin),
    /// Let Steam update itself again.
    SteamUndo,
    RestartSteam,
    /// This app's own "repair the Steam add-on" gesture.
    RepairTools,
    OpenUrl(String),
    /// Point SteamTools at another upstream for manifest request codes (a known name).
    SteamToolsSource(String),
}

impl MotdAction {
    /// The action this button stands for, or `None` for an unknown kind or
    /// parameters that do not pass: a button that would fail is worse than none.
    pub fn resolve(&self) -> Option<ResolvedAction> {
        match self.kind.as_str() {
            "steam_downgrade" => {
                if self.affects_from.is_some() && self.affects_from_number().is_none() {
                    return None;
                }
                Pin::parse(self.snapshot.as_deref()?, self.manifest_sha256.as_deref()?)
                    .map(ResolvedAction::SteamDowngrade)
            }
            "steam_undo" => Some(ResolvedAction::SteamUndo),
            "restart_steam" => Some(ResolvedAction::RestartSteam),
            "repair_tools" => Some(ResolvedAction::RepairTools),
            "steamtools_source" => {
                let name = self.source.as_deref()?.trim();
                crate::steamtools_source::is_known(name).then(|| ResolvedAction::SteamToolsSource(name.to_string()))
            }
            "open_url" => safe_https_url(self.url.as_deref()?).map(ResolvedAction::OpenUrl),
            _ => None,
        }
    }
}

impl MotdAction {
    /// `affects_from` as a number, `None` when absent or not a plain integer.
    pub fn affects_from_number(&self) -> Option<u64> {
        let raw = self.affects_from.as_deref()?.trim();
        (!raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit())).then(|| raw.parse().ok())?
    }
}

/// An absolute `https://` URL with a host and no credentials; nothing else is
/// ever handed to the system's opener (no `file:`, no `steam:`, no `javascript:`).
pub fn safe_https_url(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw.trim()).ok()?;
    let ok = url.scheme() == "https"
        && url.host_str().is_some_and(|h| !h.is_empty())
        && url.username().is_empty()
        && url.password().is_none();
    ok.then(|| url.to_string())
}

/// One published notice. Every text field is a map `locale → text`, so the
/// frontend picks its own language and falls back on the neutral one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotdMessage {
    /// Unique per publication. "Do not show again" remembers this id, so a
    /// re-publication with a new id shows again — that is the way to insist.
    pub id: String,
    pub published_at: String,
    /// RFC 3339, optional. Present: shown only while `now < expires_at`.
    /// Absent: shown until the author withdraws it. Present but unreadable:
    /// ignored — a typo must not turn into a message nobody can end.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// A tone hint, nothing more — `info`, `warning` or `critical`. The
    /// frontend maps it to a colour and an icon; anything else reads as `info`.
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub title: HashMap<String, String>,
    #[serde(default)]
    pub body: HashMap<String, String>,
    /// Buttons under the text. After [`evaluate`] only those this client can
    /// run remain, at most [`MAX_ACTIONS`], with distinct ids.
    #[serde(default)]
    pub actions: Vec<MotdAction>,
    /// The notice cannot be silenced for good ("do not show again" is withdrawn)
    /// while [`workaround`] says this user still needs the fix.
    #[serde(default)]
    pub persistent: bool,
}

/// The whole file. `message: null` is how the author clears the notice
/// without deleting the file — the sig must still match, so a clear is a
/// publication like any other.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotdDocument {
    pub schema: u32,
    #[serde(default)]
    pub message: Option<MotdMessage>,
}

/// What the frontend receives: the active message plus whether the user
/// already asked never to see this exact one again.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MotdView {
    pub message: MotdMessage,
    pub dismissed: bool,
    /// What the Steam fix means for THIS machine, when the message offers one.
    pub workaround: Option<Workaround>,
    /// Where each button stands on THIS machine, by action id (`statuses`).
    pub statuses: HashMap<String, ActionStatus>,
}

// -------------------------------------------------------------------- the diagnosis

/// What is true about this machine's Steam, read from disk. Plain data so the
/// decision below is a pure function of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SteamFacts {
    /// A Steam folder was found at all.
    pub steam_present: bool,
    /// The client's package version (`package\steam_client_win64.manifest`).
    pub installed_version: Option<String>,
    /// `steam.cfg` already blocks Steam's own updates.
    pub pinned: bool,
    pub steamtools_installed: bool,
    /// `[manifest] url` of `opensteamtool.toml` (`None` = not read, taken as the default).
    pub manifest_source: Option<String>,
}

impl SteamFacts {
    pub fn read(steam_dir: Option<&std::path::Path>) -> Self {
        match steam_dir {
            Some(dir) => Self {
                steam_present: true,
                installed_version: steam_downgrade::installed_version(dir),
                pinned: steam_downgrade::is_pinned(dir),
                steamtools_installed: crate::detect::inspect_steamtools(dir).installed,
                manifest_source: Some(crate::steamtools_source::current(dir)),
            },
            None => Self::default(),
        }
    }
}

/// The verdict the window and the banner act on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Workaround {
    /// The button that applies it (`run_motd_action` takes it back).
    pub action_id: String,
    /// This user needs the fix now.
    pub needed: bool,
    pub pinned: bool,
    pub installed_version: Option<String>,
}

/// Is Steam's client at or past the version that breaks SteamTools? Without a
/// threshold, or when the installed version cannot be read, we cannot tell and say
/// yes: for someone who has SteamTools and an unpinned Steam, a banner that turns
/// out unneeded costs a glance, a missing one costs the app.
pub fn is_affected(installed_version: Option<&str>, affects_from: Option<u64>) -> bool {
    match (affects_from, installed_version.and_then(|v| v.trim().parse::<u64>().ok())) {
        (Some(from), Some(installed)) => installed >= from,
        _ => true,
    }
}

/// The fix this message offers and whether this machine needs it. `None` when the
/// message offers no Steam fix at all. **Needed** means every one of: a Steam is
/// there, SteamTools is installed, Steam is not already pinned, and the client is
/// affected. Pure, so every row of that table is a test.
pub fn workaround(message: &MotdMessage, facts: &SteamFacts) -> Option<Workaround> {
    let action = message
        .actions
        .iter()
        .find(|a| a.kind == "steam_downgrade" && a.resolve().is_some())?;
    let needed = facts.steam_present
        && facts.steamtools_installed
        && !facts.pinned
        && is_affected(facts.installed_version.as_deref(), action.affects_from_number());
    Some(Workaround {
        action_id: action.id.clone(),
        needed,
        pinned: facts.pinned,
        installed_version: facts.installed_version.clone(),
    })
}

/// Should a notification be raised for this message on this machine? A message that
/// offers the Steam fix is for the people who need it: not for someone without
/// SteamTools, nor for someone already fixed. Anything else notifies as usual.
pub fn relevant_for_toast(message: &MotdMessage, facts: &SteamFacts) -> bool {
    workaround(message, facts).is_none_or(|w| w.needed)
}

// -------------------------------------------------------------------- what a button has done
//
// A button that needs pressing once must say, afterwards, that it was: nobody should
// have to wonder whether they already clicked it, nor press it twice "to be sure".
// Two independent sources feed that answer, and neither is enough alone:
//
// - the ledger (`ActionRun`): this machine pressed it and it went through. Says
//   nothing about whether the effect is still there;
// - a probe of the effect itself (`probe`), per kind: is the thing still in place?
//   It also catches what the ledger cannot, e.g. a Steam someone pinned by hand.
//
// A new action kind decides its own probe in `probe`'s exhaustive `match`, so it
// cannot ship without saying what "done" means for it.

/// One button pressed and gone through, on this machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRun {
    pub message_id: String,
    pub action_id: String,
    /// Unix seconds.
    pub at: i64,
}

/// Runs remembered. A notice is replaced every few days; this is a ledger, not a log.
pub const MAX_RUNS_KEPT: usize = 64;

/// The ledger with `(message_id, action_id)` recorded at `at`. Pressing again
/// replaces the old entry rather than stacking a second one; the oldest go first.
pub fn record_run(runs: &[ActionRun], message_id: &str, action_id: &str, at: i64) -> Vec<ActionRun> {
    let mut kept: Vec<ActionRun> = runs
        .iter()
        .filter(|r| !(r.message_id == message_id && r.action_id == action_id))
        .cloned()
        .collect();
    kept.push(ActionRun { message_id: message_id.to_string(), action_id: action_id.to_string(), at });
    if kept.len() > MAX_RUNS_KEPT {
        kept.drain(..kept.len() - MAX_RUNS_KEPT);
    }
    kept
}

/// When `(message_id, action_id)` last went through here, if ever.
pub fn last_run(runs: &[ActionRun], message_id: &str, action_id: &str) -> Option<i64> {
    runs.iter()
        .filter(|r| r.message_id == message_id && r.action_id == action_id)
        .map(|r| r.at)
        .max()
}

/// What the interface should make of a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    /// Not done: press it.
    Todo,
    /// Done and verified still in place (pressed here, or already true without us).
    Done,
    /// Pressed here, and nothing can confirm the effect: the button steps back.
    Recorded,
    /// Pressed here before, no longer in place: press it again.
    Lost,
    /// Nothing to do on this machine.
    NotNeeded,
    /// A gesture rather than a state (restart, open a page): always available.
    Repeatable,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionStatus {
    pub state: ActionState,
    /// Unix seconds of the last press that went through here, when there is one.
    pub ran_at: Option<i64>,
}

/// What looking at the machine says about one action's effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// The effect is there.
    Holds,
    /// The effect is not there.
    Absent,
    /// The effect is not wanted here.
    NotNeeded,
    /// No way to tell from the machine.
    Unknown,
    /// Not a state: pressing it again is always legitimate.
    Gesture,
}

/// Look at the machine for the effect of `action`. Exhaustive on purpose.
pub fn probe(action: &MotdAction, resolved: &ResolvedAction, facts: &SteamFacts) -> Probe {
    match resolved {
        ResolvedAction::SteamDowngrade(_) => {
            let from = action.affects_from_number();
            let installed = facts.installed_version.as_deref().and_then(|v| v.trim().parse::<u64>().ok());
            // Known to be at or past the build that breaks SteamTools. Unknown is not "broken":
            // a pin with no way to compare is taken as the fix being there.
            let broken = matches!((from, installed), (Some(f), Some(i)) if i >= f);
            let below = matches!((from, installed), (Some(f), Some(i)) if i < f);
            if facts.pinned && !broken {
                Probe::Holds
            } else if facts.pinned {
                Probe::Absent
            } else if (facts.steam_present && !facts.steamtools_installed) || below {
                Probe::NotNeeded
            } else {
                Probe::Absent
            }
        }
        ResolvedAction::SteamUndo if !facts.steam_present => Probe::Unknown,
        ResolvedAction::SteamUndo => {
            if facts.pinned {
                Probe::Absent
            } else {
                Probe::Holds
            }
        }
        // Repairing SteamTools has no signature on disk that tells a repaired one from a
        // broken one: the ledger is all there is.
        ResolvedAction::RepairTools => Probe::Unknown,
        // The file says which upstream is configured: that is the effect, verifiable.
        ResolvedAction::SteamToolsSource(_) if !facts.steam_present => Probe::Unknown,
        ResolvedAction::SteamToolsSource(_) if !facts.steamtools_installed => Probe::NotNeeded,
        ResolvedAction::SteamToolsSource(want) => {
            let now = facts.manifest_source.as_deref().unwrap_or(crate::steamtools_source::DEFAULT_SOURCE);
            if now == want {
                Probe::Holds
            } else {
                Probe::Absent
            }
        }
        ResolvedAction::RestartSteam | ResolvedAction::OpenUrl(_) => Probe::Gesture,
    }
}

/// Combine the probe and the ledger. Pure, one test per row.
pub fn status_of(probe: Probe, ran_at: Option<i64>) -> ActionStatus {
    let state = match probe {
        Probe::Gesture => ActionState::Repeatable,
        Probe::Holds => ActionState::Done,
        Probe::NotNeeded => ActionState::NotNeeded,
        Probe::Absent if ran_at.is_some() => ActionState::Lost,
        Probe::Absent => ActionState::Todo,
        Probe::Unknown if ran_at.is_some() => ActionState::Recorded,
        Probe::Unknown => ActionState::Todo,
    };
    ActionStatus { state, ran_at }
}

/// Where every button of `message` stands on this machine, by action id.
pub fn statuses(message: &MotdMessage, facts: &SteamFacts, runs: &[ActionRun]) -> HashMap<String, ActionStatus> {
    message
        .actions
        .iter()
        .filter_map(|a| {
            let resolved = a.resolve()?;
            let ran_at = last_run(runs, &message.id, &a.id);
            Some((a.id.clone(), status_of(probe(a, &resolved, facts), ran_at)))
        })
        .collect()
}

// -------------------------------------------------------------------- base URL

/// The notice is an asset of a dedicated **pre-release** tagged `motd`, not of
/// the latest release: a pre-release never becomes "latest", so publishing a
/// notice can never move the update pointer, and a new release never erases
/// the notice. `gh release upload motd motd.json motd.json.sig --clobber` is
/// the whole publication (`scripts/publish-motd.ps1`).
///
/// Same signature, same keys as the manifest: one trust decision. The
/// override is honoured in every build for the same reason `LV_UPDATE_BASE`
/// is — a redirected host still cannot produce a document
/// `RELEASE_PUBLIC_KEYS` accept.
const DEFAULT_BASE: &str = "https://github.com/LeGeRyChEeSe/luavault/releases/download/motd";

pub fn base_url() -> String {
    std::env::var("LV_MOTD_BASE").unwrap_or_else(|_| DEFAULT_BASE.to_string())
}

// -------------------------------------------------------------------- fetch

/// Fetch `motd.json` + `motd.json.sig`, verify the signature on the raw bytes
/// BEFORE deserialising, and enforce the schema cap. Every failure is the
/// nominal offline/tampered case and yields `None`.
pub async fn fetch_verified_motd(http: &UpdateClient, base: &str) -> Option<MotdDocument> {
    fetch_verified_motd_with_keys(http, base, &RELEASE_PUBLIC_KEYS).await
}

/// [`fetch_verified_motd`] over a caller-supplied key list (tests).
pub async fn fetch_verified_motd_with_keys(
    http: &UpdateClient,
    base: &str,
    keys: &[&str],
) -> Option<MotdDocument> {
    let (doc_resp, sig_resp) = match tokio::join!(
        http.inner().get(format!("{base}/motd.json")).send(),
        http.inner().get(format!("{base}/motd.json.sig")).send(),
    ) {
        (Ok(m), Ok(s)) => (m, s),
        (Err(e), _) | (_, Err(e)) => {
            debug!("check_motd: réseau indisponible ({e})");
            return None;
        }
    };

    // A 404 is the ordinary "nothing published" state. Reading its body as a
    // document would only produce a misleading "illisible" line in the log.
    if !doc_resp.status().is_success() || !sig_resp.status().is_success() {
        debug!(
            "check_motd: aucun message publié ({} / {})",
            doc_resp.status(),
            sig_resp.status()
        );
        return None;
    }

    let doc_bytes = match update::read_capped(doc_resp, MAX_MOTD_SIZE).await {
        Ok(b) => b,
        Err(e) => {
            debug!("check_motd: lecture du message impossible ({e})");
            return None;
        }
    };
    let sig_bytes = match update::read_capped(sig_resp, update::MAX_SIGNATURE_SIZE).await {
        Ok(b) => b,
        Err(e) => {
            debug!("check_motd: lecture de la signature impossible ({e})");
            return None;
        }
    };
    let sig_text = match String::from_utf8(sig_bytes) {
        Ok(s) => s,
        Err(_) => {
            debug!("check_motd: signature non-UTF8 — rejetée");
            return None;
        }
    };

    parse_verified(&doc_bytes, &sig_text, keys)
}

/// The pure half of the fetch: signature first, then JSON, then schema.
/// Split out so the rejection order is tested without a socket.
pub fn parse_verified(doc_bytes: &[u8], sig_text: &str, keys: &[&str]) -> Option<MotdDocument> {
    if update::verify_manifest_with_keys(doc_bytes, sig_text, keys).is_none() {
        debug!("check_motd: signature invalide — message rejeté");
        return None;
    }

    let doc: MotdDocument = match serde_json::from_slice(doc_bytes) {
        Ok(d) => d,
        Err(e) => {
            debug!("check_motd: message illisible ({e})");
            return None;
        }
    };

    if doc.schema > SCHEMA {
        debug!(
            "check_motd: schema {} > {} — abandon silencieux",
            doc.schema, SCHEMA
        );
        return None;
    }

    Some(doc)
}

// -------------------------------------------------------------------- evaluation

/// Reduce a verified document to the message worth showing at `now`, or
/// `None`. Pure, so every branch is tested by value:
/// - no message, or an empty id → nothing;
/// - `expires_at` absent → active until withdrawn;
/// - `expires_at` present but unreadable → nothing (a typo must not become a
///   message nobody can end);
/// - expired → nothing;
/// - no body in any language → nothing to render, nothing shown.
pub fn evaluate(doc: MotdDocument, now: DateTime<Utc>) -> Option<MotdMessage> {
    let message = doc.message?;
    if message.id.trim().is_empty() {
        debug!("check_motd: message sans identifiant — ignoré");
        return None;
    }
    if let Some(raw) = message.expires_at.as_deref() {
        let expires = match DateTime::parse_from_rfc3339(raw) {
            Ok(t) => t.with_timezone(&Utc),
            Err(e) => {
                debug!("check_motd: expires_at illisible ({e}) — ignoré");
                return None;
            }
        };
        if now >= expires {
            debug!("check_motd: message {} expiré ({raw})", message.id);
            return None;
        }
    }
    if message.body.values().all(|s| s.trim().is_empty()) {
        debug!("check_motd: message {} sans corps — ignoré", message.id);
        return None;
    }
    let mut message = message;
    message.actions = usable_actions(std::mem::take(&mut message.actions));
    Some(message)
}

/// Keep the actions this client can run: known kind, valid parameters, a
/// non-empty id not already taken, within the cap.
fn usable_actions(actions: Vec<MotdAction>) -> Vec<MotdAction> {
    let mut seen: Vec<String> = Vec::new();
    let mut kept = Vec::new();
    for a in actions {
        if a.id.trim().is_empty() || seen.contains(&a.id) || a.resolve().is_none() {
            debug!("check_motd: action {:?} ({}) ignorée", a.id, a.kind);
            continue;
        }
        seen.push(a.id.clone());
        kept.push(a);
        if kept.len() == MAX_ACTIONS {
            break;
        }
    }
    kept
}

/// The action `action_id` of `message`, if the message is still live at `now`
/// (re-checked: the window may have stayed open past the expiry).
pub fn find_action(message: &MotdMessage, action_id: &str, now: DateTime<Utc>) -> Option<ResolvedAction> {
    if let Some(raw) = message.expires_at.as_deref() {
        let expires = DateTime::parse_from_rfc3339(raw).ok()?.with_timezone(&Utc);
        if now >= expires {
            return None;
        }
    }
    message.actions.iter().find(|a| a.id == action_id)?.resolve()
}

/// Whether the message should stay quiet at startup: only when the user
/// asked to forget THIS id. Any other id — including a re-publication of the
/// same text — shows again.
pub fn is_dismissed(message: &MotdMessage, dismissed_id: Option<&str>) -> bool {
    dismissed_id.is_some_and(|id| id == message.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::tests::{http_ok, sign_b64, spawn_raw_server, test_keypair};
    use chrono::TimeZone;
    use ed25519_dalek::SigningKey;

    fn doc_json(id: &str, expires: &str) -> String {
        format!(
            r#"{{"schema":1,"message":{{"id":"{id}","published_at":"2026-09-16T10:00:00Z","expires_at":"{expires}","severity":"warning","title":{{"fr":"Panne","en":"Outage"}},"body":{{"fr":"Le **serveur** est en panne.","en":"The **server** is down."}}}}}}"#
        )
    }

    fn at(rfc: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc).unwrap().with_timezone(&Utc)
    }

    fn parse(json: &str) -> MotdDocument {
        serde_json::from_str(json).unwrap()
    }

    // ── evaluate ──

    #[test]
    fn evaluate_active_message_is_returned() {
        let doc = parse(&doc_json("m1", "2026-09-19T10:00:00Z"));
        let m = evaluate(doc, at("2026-09-17T00:00:00Z")).expect("active");
        assert_eq!(m.id, "m1");
        assert_eq!(m.title["fr"], "Panne");
    }

    #[test]
    fn evaluate_expired_message_is_none() {
        let doc = parse(&doc_json("m1", "2026-09-19T10:00:00Z"));
        assert!(evaluate(doc, at("2026-09-19T10:00:00Z")).is_none(), "expiry is exclusive");
        let doc = parse(&doc_json("m1", "2026-09-19T10:00:00Z"));
        assert!(evaluate(doc, at("2026-10-01T00:00:00Z")).is_none());
    }

    #[test]
    fn evaluate_one_second_before_expiry_is_still_active() {
        let doc = parse(&doc_json("m1", "2026-09-19T10:00:00Z"));
        assert!(evaluate(doc, at("2026-09-19T09:59:59Z")).is_some());
    }

    #[test]
    fn evaluate_honours_timezone_offsets() {
        // 12:00+02:00 is 10:00Z — one second past it is expired.
        let doc = parse(&doc_json("m1", "2026-09-19T12:00:00+02:00"));
        assert!(evaluate(doc, at("2026-09-19T10:00:01Z")).is_none());
        let doc = parse(&doc_json("m1", "2026-09-19T12:00:00+02:00"));
        assert!(evaluate(doc, at("2026-09-19T09:59:59Z")).is_some());
    }

    #[test]
    fn evaluate_unreadable_expiry_is_none() {
        // A notice with no readable end would be shown forever.
        let doc = parse(&doc_json("m1", "demain"));
        assert!(evaluate(doc, at("2026-09-17T00:00:00Z")).is_none());
    }

    #[test]
    fn evaluate_without_expiry_is_active_until_withdrawn() {
        // No `expires_at` at all: the author will withdraw it by hand.
        let doc = parse(
            r#"{"schema":1,"message":{"id":"open","published_at":"2026-09-16T10:00:00Z","severity":"warning","body":{"fr":"Panne en cours."}}}"#,
        );
        let m = evaluate(doc, at("2030-01-01T00:00:00Z")).expect("no expiry means still active");
        assert_eq!(m.expires_at, None);
    }

    #[test]
    fn evaluate_explicit_null_expiry_is_active() {
        let doc = parse(
            r#"{"schema":1,"message":{"id":"open","published_at":"2026-09-16T10:00:00Z","expires_at":null,"body":{"fr":"x"}}}"#,
        );
        assert!(evaluate(doc, at("2030-01-01T00:00:00Z")).is_some());
    }

    #[test]
    fn evaluate_null_message_is_none() {
        let doc = parse(r#"{"schema":1,"message":null}"#);
        assert!(evaluate(doc, at("2026-09-17T00:00:00Z")).is_none());
        let doc = parse(r#"{"schema":1}"#);
        assert!(evaluate(doc, at("2026-09-17T00:00:00Z")).is_none());
    }

    #[test]
    fn evaluate_blank_id_is_none() {
        let doc = parse(&doc_json("  ", "2026-09-19T10:00:00Z"));
        assert!(evaluate(doc, at("2026-09-17T00:00:00Z")).is_none());
    }

    #[test]
    fn evaluate_empty_body_is_none() {
        let doc = parse(
            r#"{"schema":1,"message":{"id":"m","published_at":"2026-09-16T10:00:00Z","expires_at":"2026-09-19T10:00:00Z","title":{"fr":"x"},"body":{"fr":"  ","en":""}}}"#,
        );
        assert!(evaluate(doc, at("2026-09-17T00:00:00Z")).is_none());
    }

    #[test]
    fn deserialises_without_optional_fields() {
        let doc = parse(
            r#"{"schema":1,"message":{"id":"m","published_at":"2026-09-16T10:00:00Z","expires_at":"2026-09-19T10:00:00Z"}}"#,
        );
        let m = doc.message.unwrap();
        assert_eq!(m.severity, None);
        assert!(m.title.is_empty());
    }

    // ── is_dismissed ──

    #[test]
    fn dismissed_only_for_the_same_id() {
        let m = evaluate(parse(&doc_json("m1", "2026-09-19T10:00:00Z")), at("2026-09-17T00:00:00Z")).unwrap();
        assert!(is_dismissed(&m, Some("m1")));
        assert!(!is_dismissed(&m, Some("m0")));
        assert!(!is_dismissed(&m, None));
    }

    // ── parse_verified: order of rejections ──

    fn keypair(seed: u8) -> (SigningKey, String) {
        test_keypair(seed)
    }

    #[test]
    fn parse_verified_accepts_signed_document() {
        let (sk, pk) = keypair(0x71);
        let json = doc_json("m1", "2026-09-19T10:00:00Z");
        let sig = sign_b64(&sk, json.as_bytes());
        let doc = parse_verified(json.as_bytes(), &sig, &[&pk]).expect("signed");
        assert_eq!(doc.message.unwrap().id, "m1");
    }

    #[test]
    fn parse_verified_rejects_wrong_key() {
        let (attacker, _) = keypair(0x72);
        let (_, pk) = keypair(0x73);
        let json = doc_json("m1", "2026-09-19T10:00:00Z");
        let sig = sign_b64(&attacker, json.as_bytes());
        assert!(parse_verified(json.as_bytes(), &sig, &[&pk]).is_none());
    }

    #[test]
    fn parse_verified_accepts_fallback_key() {
        let (_, pk0) = keypair(0x74);
        let (sk1, pk1) = keypair(0x75);
        let json = doc_json("m1", "2026-09-19T10:00:00Z");
        let sig = sign_b64(&sk1, json.as_bytes());
        assert!(parse_verified(json.as_bytes(), &sig, &[&pk0, &pk1]).is_some());
    }

    #[test]
    fn parse_verified_rejects_tampered_bytes() {
        // Signed, then one byte of the body changed: the message a compromised
        // server would most like to alter.
        let (sk, pk) = keypair(0x76);
        let json = doc_json("m1", "2026-09-19T10:00:00Z");
        let sig = sign_b64(&sk, json.as_bytes());
        let tampered = json.replace("en panne", "à jour");
        assert_ne!(tampered, json);
        assert!(parse_verified(tampered.as_bytes(), &sig, &[&pk]).is_none());
    }

    #[test]
    fn parse_verified_signature_is_checked_before_json() {
        // Invalid JSON with a VALID signature is a JSON failure; invalid JSON
        // with an invalid signature must be indistinguishable — both None —
        // and a valid signature over garbage is still None.
        let (sk, pk) = keypair(0x77);
        let garbage = b"{not json";
        let sig = sign_b64(&sk, garbage);
        assert!(parse_verified(garbage, &sig, &[&pk]).is_none());
    }

    #[test]
    fn parse_verified_schema_above_is_silent_none() {
        let (sk, pk) = keypair(0x78);
        let json = r#"{"schema":2,"message":null}"#;
        let sig = sign_b64(&sk, json.as_bytes());
        assert!(parse_verified(json.as_bytes(), &sig, &[&pk]).is_none());
    }

    #[test]
    fn parse_verified_schema_below_is_accepted() {
        let (sk, pk) = keypair(0x79);
        let json = r#"{"schema":0,"message":null}"#;
        let sig = sign_b64(&sk, json.as_bytes());
        assert!(parse_verified(json.as_bytes(), &sig, &[&pk]).is_some());
    }

    // ── fetch (real sockets) ──

    fn motd_routes(json: &str, sk: &SigningKey) -> std::collections::HashMap<String, Vec<u8>> {
        let sig = sign_b64(sk, json.as_bytes());
        let mut routes = std::collections::HashMap::new();
        routes.insert("/motd.json".to_string(), http_ok(json.as_bytes()));
        routes.insert("/motd.json.sig".to_string(), http_ok(sig.as_bytes()));
        routes
    }

    #[tokio::test]
    async fn fetch_motd_happy_path() {
        let (sk, pk) = keypair(0x7a);
        let base = spawn_raw_server(motd_routes(&doc_json("m1", "2026-09-19T10:00:00Z"), &sk)).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        let doc = fetch_verified_motd_with_keys(&http, &base, &[&pk]).await.expect("signed");
        assert_eq!(doc.message.unwrap().id, "m1");
    }

    #[tokio::test]
    async fn fetch_motd_rejects_invalid_signature() {
        let (attacker, _) = keypair(0x7b);
        let (_, pk) = keypair(0x7c);
        let base = spawn_raw_server(motd_routes(&doc_json("m1", "2026-09-19T10:00:00Z"), &attacker)).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        assert!(fetch_verified_motd_with_keys(&http, &base, &[&pk]).await.is_none());
    }

    #[tokio::test]
    async fn fetch_motd_absent_is_none() {
        // No routes at all: the server answers 404 to both, the nominal
        // "nothing published" state.
        let (_, pk) = keypair(0x7d);
        let base = spawn_raw_server(std::collections::HashMap::new()).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        assert!(fetch_verified_motd_with_keys(&http, &base, &[&pk]).await.is_none());
    }

    #[tokio::test]
    async fn fetch_motd_oversized_announcement_is_refused() {
        let (sk, pk) = keypair(0x7e);
        let json = doc_json("m1", "2026-09-19T10:00:00Z");
        let sig = sign_b64(&sk, json.as_bytes());
        let mut routes = std::collections::HashMap::new();
        // Lies about its length: far above MAX_MOTD_SIZE, with a real body.
        let lying = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            MAX_MOTD_SIZE + 1,
            json
        );
        routes.insert("/motd.json".to_string(), lying.into_bytes());
        routes.insert("/motd.json.sig".to_string(), http_ok(sig.as_bytes()));
        let base = spawn_raw_server(routes).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        assert!(fetch_verified_motd_with_keys(&http, &base, &[&pk]).await.is_none());
    }

    #[test]
    fn utc_now_is_comparable_with_parsed_expiry() {
        // Guards the type plumbing: `evaluate` takes the same clock the
        // command passes (`Utc::now()`), so a naive/aware mismatch cannot
        // creep in unnoticed.
        let now = Utc.with_ymd_and_hms(2026, 9, 17, 0, 0, 0).unwrap();
        let doc = parse(&doc_json("m1", "2026-09-19T10:00:00Z"));
        assert!(evaluate(doc, now).is_some());
    }

    // ── actions ──

    const SHA: &str = "40ec2f2aac99fb65c62b7b60db77e8e1575e0c8b59cfeba80294eeb1c963731c";

    fn with_actions(actions: &str) -> MotdDocument {
        parse(&format!(
            r#"{{"schema":1,"message":{{"id":"m1","published_at":"2026-10-17T10:00:00Z","body":{{"fr":"x"}},"actions":{actions}}}}}"#
        ))
    }

    fn live(actions: &str) -> Vec<MotdAction> {
        evaluate(with_actions(actions), at("2026-10-17T12:00:00Z")).unwrap().actions
    }

    #[test]
    fn a_message_without_actions_still_reads() {
        let m = evaluate(parse(&doc_json("m1", "2026-09-19T10:00:00Z")), at("2026-09-17T00:00:00Z")).unwrap();
        assert!(m.actions.is_empty());
    }

    #[test]
    fn every_known_kind_resolves() {
        let a = live(&format!(
            r#"[{{"id":"a","kind":"steam_downgrade","snapshot":"20261006145653","manifest_sha256":"{SHA}"}},
               {{"id":"b","kind":"steam_undo"}},{{"id":"c","kind":"restart_steam"}},
               {{"id":"d","kind":"repair_tools"}},{{"id":"e","kind":"open_url","url":"https://example.org/x"}}]"#
        ));
        // MAX_ACTIONS cuts the fifth.
        assert_eq!(a.len(), MAX_ACTIONS);
        assert_eq!(
            a[0].resolve(),
            Some(ResolvedAction::SteamDowngrade(Pin::parse("20261006145653", SHA).unwrap()))
        );
        assert_eq!(a[1].resolve(), Some(ResolvedAction::SteamUndo));
        assert_eq!(a[2].resolve(), Some(ResolvedAction::RestartSteam));
        assert_eq!(a[3].resolve(), Some(ResolvedAction::RepairTools));
    }

    #[test]
    fn an_unknown_kind_drops_the_button_not_the_message() {
        let a = live(r#"[{"id":"a","kind":"format_disk"},{"id":"b","kind":"restart_steam"}]"#);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].id, "b");
    }

    #[test]
    fn a_downgrade_with_bad_parameters_is_dropped() {
        for params in [
            r#""snapshot":"20261006145653""#,                                      // no hash
            &format!(r#""manifest_sha256":"{SHA}""#),                              // no date
            &format!(r#""snapshot":"../x","manifest_sha256":"{SHA}""#),            // date that is a path
            r#""snapshot":"20261006145653","manifest_sha256":"abc""#,              // short hash
        ] {
            let a = live(&format!(r#"[{{"id":"a","kind":"steam_downgrade",{params}}}]"#));
            assert!(a.is_empty(), "{params}");
        }
    }

    #[test]
    fn duplicate_and_blank_ids_are_dropped() {
        let a = live(r#"[{"id":"a","kind":"restart_steam"},{"id":"a","kind":"steam_undo"},{"id":" ","kind":"repair_tools"}]"#);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].kind, "restart_steam");
    }

    #[test]
    fn open_url_accepts_only_plain_https() {
        for ok in ["https://example.org", "https://example.org/a?b=1#c", " https://example.org/x "] {
            assert!(safe_https_url(ok).is_some(), "{ok}");
        }
        for bad in [
            "http://example.org", "file:///C:/Windows/System32/calc.exe", "steam://install/1",
            "javascript:alert(1)", "https://user:pw@example.org", "https://user@example.org",
            "https://", "//example.org", "example.org", "",
        ] {
            assert!(safe_https_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn find_action_runs_only_a_live_messages_own_action() {
        let m = evaluate(
            parse(r#"{"schema":1,"message":{"id":"m1","published_at":"2026-10-17T10:00:00Z","expires_at":"2026-10-18T10:00:00Z","body":{"fr":"x"},"actions":[{"id":"a","kind":"restart_steam"}]}}"#),
            at("2026-10-17T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(find_action(&m, "a", at("2026-10-17T13:00:00Z")), Some(ResolvedAction::RestartSteam));
        assert_eq!(find_action(&m, "zzz", at("2026-10-17T13:00:00Z")), None, "an id the message never carried");
        assert_eq!(find_action(&m, "a", at("2026-10-18T10:00:00Z")), None, "expired while the window was open");
    }

    #[test]
    fn find_action_without_expiry_stays_available() {
        let m = evaluate(with_actions(r#"[{"id":"a","kind":"steam_undo"}]"#), at("2026-10-17T12:00:00Z")).unwrap();
        assert!(find_action(&m, "a", at("2030-01-01T00:00:00Z")).is_some());
    }

    #[test]
    fn parameters_never_reach_the_frontend() {
        let m = evaluate(
            with_actions(&format!(r#"[{{"id":"a","kind":"steam_downgrade","snapshot":"20261006145653","manifest_sha256":"{SHA}"}},{{"id":"b","kind":"open_url","url":"https://example.org/x"}}]"#)),
            at("2026-10-17T12:00:00Z"),
        )
        .unwrap();
        let sent = serde_json::to_string(&MotdView { message: m, dismissed: false, workaround: None, statuses: HashMap::new() }).unwrap();
        for leaked in ["20261006145653", SHA, "example.org"] {
            assert!(!sent.contains(leaked), "{leaked} serialised to the UI");
        }
        assert!(sent.contains("steam_downgrade"));
    }

    #[test]
    fn a_tampered_action_breaks_the_signature() {
        // The whole point of signing the raw bytes: swapping the hash a button
        // installs is the attack that matters most.
        let (sk, pk) = keypair(0x80);
        let json = format!(
            r#"{{"schema":1,"message":{{"id":"m1","published_at":"2026-10-17T10:00:00Z","body":{{"fr":"x"}},"actions":[{{"id":"a","kind":"steam_downgrade","snapshot":"20261006145653","manifest_sha256":"{SHA}"}}]}}}}"#
        );
        let sig = sign_b64(&sk, json.as_bytes());
        assert!(parse_verified(json.as_bytes(), &sig, &[&pk]).is_some());
        let evil = json.replace(&SHA[..8], "deadbeef");
        assert_ne!(evil, json);
        assert!(parse_verified(evil.as_bytes(), &sig, &[&pk]).is_none());
    }

    // ── persistent and diagnosed ──

    fn msg_with(actions: &str, persistent: bool) -> MotdMessage {
        let doc = with_actions(actions);
        let mut m = evaluate(doc, at("2026-10-17T12:00:00Z")).unwrap();
        m.persistent = persistent;
        m
    }

    fn downgrade(extra: &str) -> String {
        format!(r#"[{{"id":"fix","kind":"steam_downgrade","snapshot":"20261006145653","manifest_sha256":"{SHA}"{extra}}}]"#)
    }

    fn facts(version: Option<&str>, pinned: bool, st: bool) -> SteamFacts {
        SteamFacts {
            steam_present: true,
            installed_version: version.map(str::to_string),
            pinned,
            steamtools_installed: st,
            manifest_source: None,
        }
    }

    #[test]
    fn persistent_defaults_to_false_and_reads_when_set() {
        let plain = evaluate(parse(&doc_json("m1", "2026-09-19T10:00:00Z")), at("2026-09-17T00:00:00Z")).unwrap();
        assert!(!plain.persistent);
        let doc = parse(r#"{"schema":1,"message":{"id":"m","published_at":"x","persistent":true,"body":{"fr":"x"}}}"#);
        assert!(evaluate(doc, at("2026-10-17T00:00:00Z")).unwrap().persistent);
    }

    #[test]
    fn affects_from_must_be_a_plain_number_or_the_button_is_dropped() {
        for bad in ["\"abc\"", "\"12 34\"", "\"-5\"", "\"1.5\"", "\"\"", "\"99999999999999999999999\""] {
            let a = live(&downgrade(&format!(r#","affects_from":{bad}"#)));
            assert!(a.is_empty(), "affects_from {bad} must drop the action");
        }
        let a = live(&downgrade(r#","affects_from":"1791918634""#));
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].affects_from_number(), Some(1_791_918_634));
        assert_eq!(live(&downgrade("")).len(), 1, "absent is fine");
    }

    #[test]
    fn affects_from_never_reaches_the_frontend() {
        let m = msg_with(&downgrade(r#","affects_from":"1791918634""#), true);
        let sent = serde_json::to_string(&m).unwrap();
        assert!(!sent.contains("1791918634") && !sent.contains("affects_from"), "{sent}");
        assert!(sent.contains("\"persistent\":true"));
    }

    #[test]
    fn is_affected_compares_numerically_and_says_yes_when_it_cannot_tell() {
        assert!(is_affected(Some("1791918634"), Some(1_791_918_634)), "at the version");
        assert!(is_affected(Some("1800000000"), Some(1_791_918_634)), "past it");
        assert!(!is_affected(Some("1788652215"), Some(1_791_918_634)), "before it: the downgrade worked");
        // 9 > 10 as text, not as a number.
        assert!(!is_affected(Some("9"), Some(10)));
        assert!(is_affected(None, Some(1)), "unreadable version");
        assert!(is_affected(Some("not a number"), Some(1)));
        assert!(is_affected(Some("1"), None), "no threshold");
    }

    #[test]
    fn workaround_is_needed_only_when_every_condition_holds() {
        let m = msg_with(&downgrade(r#","affects_from":"1791918634""#), true);
        let broken = Some("1791918634");
        let needed = |f: SteamFacts| workaround(&m, &f).unwrap().needed;
        assert!(needed(facts(broken, false, true)), "affected, unpinned, with SteamTools");
        assert!(!needed(facts(broken, true, true)), "already pinned");
        assert!(!needed(facts(broken, false, false)), "no SteamTools: nothing to break");
        assert!(!needed(facts(Some("1788652215"), false, true)), "client before the break");
        assert!(!needed(SteamFacts::default()), "no Steam at all");
        // Undoing the fix puts the user back in the affected set: the nag returns.
        assert!(needed(facts(Some("1800000000"), false, true)));
    }

    #[test]
    fn workaround_names_its_button_and_reports_the_state() {
        let m = msg_with(&downgrade(r#","affects_from":"1791918634""#), true);
        let w = workaround(&m, &facts(Some("1800000000"), true, true)).unwrap();
        assert_eq!(w.action_id, "fix");
        assert!(w.pinned && !w.needed);
        assert_eq!(w.installed_version.as_deref(), Some("1800000000"));
    }

    #[test]
    fn a_message_without_the_steam_fix_has_no_workaround() {
        let m = msg_with(r#"[{"id":"r","kind":"restart_steam"}]"#, true);
        assert_eq!(workaround(&m, &facts(Some("1"), false, true)), None);
        assert_eq!(workaround(&msg_with("[]", true), &facts(Some("1"), false, true)), None);
    }

    #[test]
    fn a_toast_goes_to_the_people_who_need_the_fix() {
        let m = msg_with(&downgrade(r#","affects_from":"1791918634""#), true);
        assert!(relevant_for_toast(&m, &facts(Some("1791918634"), false, true)));
        assert!(!relevant_for_toast(&m, &facts(Some("1791918634"), true, true)), "already fixed");
        assert!(!relevant_for_toast(&m, &facts(Some("1791918634"), false, false)), "no SteamTools");
        // A message that offers no Steam fix notifies everyone, as before.
        let plain = msg_with(r#"[{"id":"r","kind":"restart_steam"}]"#, false);
        assert!(relevant_for_toast(&plain, &SteamFacts::default()));
    }

    // ---------------------------------------------------------- what a button has done

    fn downgrade_action(affects_from: Option<&str>) -> MotdAction {
        MotdAction {
            id: "fix".into(),
            kind: "steam_downgrade".into(),
            label: HashMap::new(),
            snapshot: Some("20261006145653".into()),
            manifest_sha256: Some("0".repeat(64)),
            url: None,
            affects_from: affects_from.map(String::from),
            source: None,
        }
    }

    fn plain_action(id: &str, kind: &str) -> MotdAction {
        MotdAction {
            id: id.into(),
            kind: kind.into(),
            label: HashMap::new(),
            snapshot: None,
            manifest_sha256: None,
            url: None,
            affects_from: None,
            source: None,
        }
    }

    fn probe_of(a: &MotdAction, f: &SteamFacts) -> Probe {
        probe(a, &a.resolve().expect("resolvable"), f)
    }

    fn steam(installed: Option<&str>, pinned: bool, steamtools: bool) -> SteamFacts {
        SteamFacts {
            steam_present: true,
            installed_version: installed.map(String::from),
            pinned,
            steamtools_installed: steamtools,
            manifest_source: None,
        }
    }

    #[test]
    fn status_of_covers_every_row() {
        use ActionState::*;
        let at = Some(1_700_000_000);
        assert_eq!(status_of(Probe::Gesture, None).state, Repeatable);
        assert_eq!(status_of(Probe::Gesture, at).state, Repeatable, "a gesture never locks");
        assert_eq!(status_of(Probe::Holds, None).state, Done, "already true without us");
        assert_eq!(status_of(Probe::Holds, at).state, Done);
        assert_eq!(status_of(Probe::NotNeeded, at).state, NotNeeded);
        assert_eq!(status_of(Probe::Absent, None).state, Todo);
        assert_eq!(status_of(Probe::Absent, at).state, Lost, "pressed, but gone");
        assert_eq!(status_of(Probe::Unknown, None).state, Todo);
        assert_eq!(status_of(Probe::Unknown, at).state, Recorded);
        assert_eq!(status_of(Probe::Unknown, at).ran_at, at, "the date travels with the state");
    }

    #[test]
    fn the_steam_fix_probe_reads_the_machine() {
        let a = downgrade_action(Some("1791918634"));
        assert_eq!(probe_of(&a, &steam(Some("1788652215"), true, true)), Probe::Holds, "pinned on the old build");
        assert_eq!(probe_of(&a, &steam(Some("1791918634"), true, true)), Probe::Absent, "pinned on the broken build");
        assert_eq!(probe_of(&a, &steam(Some("1791918634"), false, true)), Probe::Absent, "the problem case");
        assert_eq!(probe_of(&a, &steam(Some("1788652215"), false, true)), Probe::NotNeeded, "an older Steam is not concerned");
        assert_eq!(probe_of(&a, &steam(None, false, true)), Probe::Absent, "unreadable version: cannot say it is fine");
        assert_eq!(probe_of(&a, &steam(Some("1791918634"), false, false)), Probe::NotNeeded, "no SteamTools");
        // Without a threshold a pin is simply taken as the fix being there.
        let bare = downgrade_action(None);
        assert_eq!(probe_of(&bare, &steam(Some("1791918634"), true, true)), Probe::Holds);
        assert_eq!(probe_of(&bare, &steam(Some("1791918634"), false, true)), Probe::Absent);
    }

    #[test]
    fn every_kind_decides_what_done_means_for_it() {
        let free = steam(Some("1"), false, true);
        let pinned = steam(Some("1"), true, true);
        let undo = plain_action("u", "steam_undo");
        assert_eq!(probe_of(&undo, &free), Probe::Holds, "not pinned = updates are on");
        assert_eq!(probe_of(&undo, &pinned), Probe::Absent);
        assert_eq!(probe_of(&undo, &SteamFacts::default()), Probe::Unknown, "no Steam to look at");
        assert_eq!(probe_of(&plain_action("r", "repair_tools"), &free), Probe::Unknown);
        assert_eq!(probe_of(&plain_action("s", "restart_steam"), &free), Probe::Gesture);
        let mut url = plain_action("o", "open_url");
        url.url = Some("https://example.org/".into());
        assert_eq!(probe_of(&url, &free), Probe::Gesture);
    }

    #[test]
    fn the_ledger_replaces_rather_than_stacks_and_is_capped() {
        let one = record_run(&[], "m1", "a", 10);
        let again = record_run(&one, "m1", "a", 20);
        assert_eq!(again.len(), 1);
        assert_eq!(last_run(&again, "m1", "a"), Some(20));
        assert_eq!(last_run(&again, "m1", "b"), None, "another button");
        assert_eq!(last_run(&again, "m2", "a"), None, "another message: republishing starts afresh");
        let mut runs = Vec::new();
        for i in 0..(MAX_RUNS_KEPT as i64 + 10) {
            runs = record_run(&runs, &format!("m{i}"), "a", i);
        }
        assert_eq!(runs.len(), MAX_RUNS_KEPT);
        assert_eq!(last_run(&runs, "m0", "a"), None, "the oldest went first");
        assert!(last_run(&runs, &format!("m{}", MAX_RUNS_KEPT as i64 + 9), "a").is_some());
    }

    #[test]
    fn statuses_are_keyed_by_action_id_and_follow_the_ledger() {
        let m = MotdMessage {
            id: "msg".into(),
            published_at: "2026-10-19T00:00:00Z".into(),
            expires_at: None,
            severity: None,
            title: HashMap::new(),
            body: HashMap::new(),
            actions: vec![plain_action("repair", "repair_tools"), downgrade_action(None)],
            persistent: false,
        };
        let facts = steam(None, false, true);
        let none = statuses(&m, &facts, &[]);
        assert_eq!(none["repair"].state, ActionState::Todo);
        assert_eq!(none["fix"].state, ActionState::Todo);
        let runs = record_run(&[], "msg", "repair", 5);
        let after = statuses(&m, &facts, &runs);
        assert_eq!(after["repair"].state, ActionState::Recorded);
        assert_eq!(after["repair"].ran_at, Some(5));
        assert_eq!(statuses(&m, &steam(None, true, true), &runs)["fix"].state, ActionState::Done);
    }

    // ---------------------------------------------------------- the manifest source action

    fn source_action(source: Option<&str>) -> MotdAction {
        MotdAction { source: source.map(String::from), ..plain_action("src", "steamtools_source") }
    }

    #[test]
    fn steamtools_source_resolves_only_a_known_name() {
        for name in ["wudrm", "steamrun", "opensteamtool"] {
            assert_eq!(source_action(Some(name)).resolve(), Some(ResolvedAction::SteamToolsSource(name.to_string())));
        }
        assert_eq!(source_action(Some(" wudrm ")).resolve(), Some(ResolvedAction::SteamToolsSource("wudrm".into())), "trimmed");
        // A signed message picks a NAME. It can never hand the app a URL to fetch from.
        for bad in [None, Some(""), Some("WUDRM"), Some("http://gmrc.example/manifest/%llu"), Some("../wudrm"), Some("wudrm,steamrun")] {
            assert_eq!(source_action(bad).resolve(), None, "{bad:?}");
        }
    }

    #[test]
    fn the_source_never_reaches_the_frontend() {
        let m = evaluate(with_actions(r#"[{"id":"s","kind":"steamtools_source","source":"wudrm"}]"#), at("2026-10-19T12:00:00Z")).unwrap();
        assert_eq!(m.actions.len(), 1, "a valid action is kept");
        let sent = serde_json::to_string(&MotdView { message: m, dismissed: false, workaround: None, statuses: HashMap::new() }).unwrap();
        assert!(sent.contains("steamtools_source") && !sent.contains("wudrm"), "{sent}");
    }

    #[test]
    fn an_unknown_source_drops_the_button_not_the_message() {
        let m = evaluate(with_actions(r#"[{"id":"s","kind":"steamtools_source","source":"nope"}]"#), at("2026-10-19T12:00:00Z")).unwrap();
        assert!(m.actions.is_empty());
    }

    #[test]
    fn the_source_probe_reads_the_configuration() {
        let a = source_action(Some("wudrm"));
        let with = |installed: bool, source: Option<&str>| SteamFacts {
            steam_present: true,
            installed_version: None,
            pinned: false,
            steamtools_installed: installed,
            manifest_source: source.map(String::from),
        };
        assert_eq!(probe_of(&a, &with(true, Some("wudrm"))), Probe::Holds, "already configured");
        assert_eq!(probe_of(&a, &with(true, Some("steamrun"))), Probe::Absent, "the common case");
        assert_eq!(probe_of(&a, &with(true, None)), Probe::Absent, "nothing written = the default upstream");
        assert_eq!(probe_of(&source_action(Some("opensteamtool")), &with(true, None)), Probe::Holds, "the default is a source too");
        assert_eq!(probe_of(&a, &with(false, Some("steamrun"))), Probe::NotNeeded, "no SteamTools to configure");
        assert_eq!(probe_of(&a, &SteamFacts::default()), Probe::Unknown, "no Steam folder");
    }

    #[test]
    fn a_source_flipped_back_by_a_repair_comes_back_as_lost() {
        // The repair script alternates the upstream on every run: once pressed here, a later
        // repair can undo it, and the button must offer itself again, saying so.
        let st = status_of(Probe::Absent, Some(10));
        assert_eq!(st.state, ActionState::Lost);
        let st = status_of(Probe::Holds, Some(10));
        assert_eq!(st.state, ActionState::Done);
    }
}

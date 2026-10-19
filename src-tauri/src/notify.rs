//! System notifications for the message of the day, with the app closed.
//!
//! Why it exists: the MOTD reaches whoever opens the app, and the people who most
//! need it (a broken Steam update, a service outage) are the ones who do not. The
//! same goes for a new version of the app: the toast offers it with one button that
//! downloads, verifies and opens the installer, the window's own path.
//! With the user's consent, Windows launches this binary headless at logon and
//! every few hours (`--notify-check`, see `headless.rs`); it fetches the signed
//! notice exactly like the window does and, for a new one, raises a toast whose
//! buttons run the notice's own actions without opening the app.
//!
//! What keeps that safe, all of it pinned by tests below:
//! - **Opt-in** (`config.background_notify`), never on in portable mode: the folder
//!   may move, and the registrations would point at nothing.
//! - **A toast button is a custom URL** (`<scheme>://run?n=<nonce>`), and any web
//!   page can open such a URL. So the URL carries no action and no parameter, only a
//!   128-bit single-use nonce **issued when the toast was raised** and kept on disk
//!   (`NotifyState::pending`). A forged or replayed URL finds nothing and does
//!   nothing. What the nonce stands for is looked up in the freshly fetched,
//!   signature-verified message, never in the URL.
//! - **One toast per message id**, and never for an id the user asked to forget.
//! - Nothing here runs a command: execution goes through `motd_run::execute`, the
//!   same closed set the window uses.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::motd::{MotdAction, MotdMessage};

/// How long a toast's buttons stay valid. A message is a notice, not a standing
/// order: an old toast left in the Action Center must not act next week.
pub const PENDING_TTL_SECS: i64 = 3 * 24 * 3600;

/// What a toast can carry at most: the notice, and two gestures.
pub const MAX_BUTTONS: usize = 2;

// -------------------------------------------------------------------- identity

/// Custom URL scheme of the toast buttons.
pub fn scheme() -> &'static str {
    "luavault"
}

/// The toast sender's identity (what Windows shows as the app, and lets a user mute).
pub fn aumid() -> &'static str {
    "io.github.luavault.app"
}

pub fn display_name() -> &'static str {
    "LuaVault"
}

fn task_name() -> String {
    format!("{} MOTD", display_name())
}

// -------------------------------------------------------------------- state on disk

/// One button (or the body click) of one toast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub nonce: String,
    pub message_id: String,
    /// `None`: the body of the toast, which only opens the app.
    pub action_id: Option<String>,
    pub created_at: i64,
    /// Where the message of this toast was fetched from. The click handler is
    /// started by Windows with its own environment, not the one `--notify-check`
    /// ran in: it must ask the same place, or a message from a redirected server
    /// (a local trial) is never found again. The signature stays the only trust
    /// decision: another base cannot produce a message the author did not sign.
    /// Absent in a state written before this field existed.
    #[serde(default)]
    pub base: Option<String>,
    /// `Some(version)`: this nonce is the « update » button of the toast announcing
    /// that version, not a message action. A separate field rather than a reserved
    /// action id: action ids come from a signed message and could spell anything.
    #[serde(default)]
    pub update_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyState {
    /// The last message id a toast was raised for, or the window showed.
    pub last_notified_id: Option<String>,
    /// The last app version a toast announced.
    pub last_update_notified: Option<String>,
    pub pending: Vec<Pending>,
}

impl NotifyState {
    /// Issue a fresh single-use nonce for `(message, action)`.
    pub fn issue(&mut self, message_id: &str, action_id: Option<&str>, base: &str, now: i64) -> String {
        let nonce: String = rand::random::<[u8; 16]>().iter().map(|b| format!("{b:02x}")).collect();
        self.pending.push(Pending {
            nonce: nonce.clone(),
            message_id: message_id.to_string(),
            action_id: action_id.map(str::to_string),
            created_at: now,
            base: Some(base.to_string()),
            update_version: None,
        });
        nonce
    }

    /// Issue the nonce of the « update » button for `version`.
    pub fn issue_update(&mut self, version: &str, base: &str, now: i64) -> String {
        let nonce = self.issue(&format!("update:{version}"), None, base, now);
        if let Some(p) = self.pending.last_mut() {
            p.update_version = Some(version.to_string());
        }
        nonce
    }

    /// Take the entry for `nonce`: once, and only while it is fresh. A second
    /// call, an unknown nonce and an expired one all return `None`.
    pub fn consume(&mut self, nonce: &str, now: i64) -> Option<Pending> {
        self.prune(now);
        let at = self.pending.iter().position(|p| p.nonce == nonce)?;
        Some(self.pending.remove(at))
    }

    pub fn prune(&mut self, now: i64) {
        self.pending.retain(|p| now - p.created_at < PENDING_TTL_SECS);
    }
}

fn state_path() -> PathBuf {
    crate::config::data_dir().join("notify_state.json")
}

pub fn load_state() -> NotifyState {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Temp + rename, like every other file the app owns.
pub fn save_state(state: &NotifyState) -> std::io::Result<()> {
    let path = state_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, &path)
}

/// The window showed this message: no toast for it afterwards.
pub fn mark_notified(message_id: &str) {
    let mut state = load_state();
    if state.last_notified_id.as_deref() != Some(message_id) {
        state.last_notified_id = Some(message_id.to_string());
        let _ = save_state(&state);
    }
}

// -------------------------------------------------------------------- decisions

/// Raise a toast for this (already live, already evaluated) message?
/// Not for an id the user asked to forget, nor one already announced.
pub fn should_notify(message_id: &str, dismissed_id: Option<&str>, last_notified_id: Option<&str>) -> bool {
    dismissed_id != Some(message_id) && last_notified_id != Some(message_id)
}

/// The interface language of the notification: the config's, else the system's,
/// else English. Only `fr` and `en` are shipped.
pub fn pick_language(configured: Option<&str>, system: Option<&str>) -> &'static str {
    for raw in [configured, system].into_iter().flatten() {
        let tag = raw.trim().to_ascii_lowercase();
        if tag.starts_with("fr") {
            return "fr";
        }
        if tag.starts_with("en") {
            return "en";
        }
    }
    "en"
}

/// What the notification says when the author gave no label.
pub fn default_label(kind: &str, lang: &str) -> &'static str {
    let fr = lang == "fr";
    match kind {
        "steam_downgrade" => if fr { "Appliquer le correctif Steam" } else { "Apply the Steam fix" },
        "steam_undo" => if fr { "Rétablir les mises à jour de Steam" } else { "Turn Steam updates back on" },
        "restart_steam" => if fr { "Redémarrer Steam" } else { "Restart Steam" },
        "repair_tools" => if fr { "Réparer SteamTools" } else { "Repair SteamTools" },
        "open_url" => if fr { "Ouvrir la page" } else { "Open the page" },
        "steamtools_source" => if fr { "Utiliser la source de manifestes stable" } else { "Use the stable manifest source" },
        _ => if fr { "Ouvrir" } else { "Open" },
    }
}

pub fn default_title(lang: &str) -> &'static str {
    if lang == "fr" { "Message du jour" } else { "Message of the day" }
}

/// The text map's entry for `lang`, then `en`, then `fr`, then any: the same
/// fallback as the window (`motd.ts::resolveMotdText`).
pub fn resolve_text(map: &std::collections::HashMap<String, String>, lang: &str) -> String {
    for key in [lang, "en", "fr"] {
        if let Some(v) = map.get(key).filter(|v| !v.trim().is_empty()) {
            return v.clone();
        }
    }
    let mut rest: Vec<_> = map.iter().filter(|(_, v)| !v.trim().is_empty()).collect();
    rest.sort_by(|a, b| a.0.cmp(b.0));
    rest.first().map(|(_, v)| (*v).clone()).unwrap_or_default()
}

/// The label of a button for `lang`: the author's, else the default of its kind.
pub fn button_label(action: &MotdAction, lang: &str) -> String {
    let own = resolve_text(&action.label, lang);
    if own.is_empty() { default_label(&action.kind, lang).to_string() } else { own }
}

// -------------------------------------------------------------------- plain text

/// The motd dialect (`motd-markdown.ts`) reduced to plain text for a toast, which
/// shows no formatting: markers dropped, links keep their text, one line, cut at
/// `max` characters with an ellipsis.
pub fn plain_text(markdown: &str, max: usize) -> String {
    let mut text = String::new();
    for raw in markdown.lines() {
        let line = raw.trim();
        let line = line.trim_start_matches('#').trim_start_matches('>').trim();
        let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).unwrap_or(line);
        if !line.is_empty() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(line);
        }
    }
    // [label](https://…) → label
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if let Some(close) = chars[i..].iter().position(|&c| c == ']') {
                let end = i + close;
                if chars.get(end + 1) == Some(&'(') {
                    if let Some(paren) = chars[end + 1..].iter().position(|&c| c == ')') {
                        out.extend(&chars[i + 1..end]);
                        i = end + 1 + paren + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    // {red}…{/}, {#ff8800}…{/}
    let mut cleaned = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(open) = rest.find('{') {
        cleaned.push_str(&rest[..open]);
        match rest[open..].find('}') {
            Some(close) => {
                let inner = &rest[open + 1..open + close];
                let is_tag = inner == "/" || inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '#');
                if is_tag {
                    rest = &rest[open + close + 1..];
                } else {
                    cleaned.push('{');
                    rest = &rest[open + 1..];
                }
            }
            None => {
                cleaned.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    cleaned.push_str(rest);
    let cleaned: String = cleaned.replace(['*', '`'], "");
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let cut: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    }
}

// -------------------------------------------------------------------- toast xml

fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// The toast document. Every dynamic value is escaped: the title, the body and
/// the labels come from a signed message, but a signature proves who wrote the
/// text, not that it is safe to splice into markup.
pub fn toast_xml(title: &str, body: &str, launch_url: &str, buttons: &[(String, String)]) -> String {
    let actions: String = buttons
        .iter()
        .take(MAX_BUTTONS)
        .map(|(label, url)| {
            format!(
                r#"<action content="{}" arguments="{}" activationType="protocol"/>"#,
                xml_escape(label),
                xml_escape(url)
            )
        })
        .collect();
    // No launch URL (the toast that reports a result): a click on the body does nothing.
    let launch = if launch_url.is_empty() {
        String::new()
    } else {
        format!(r#" launch="{}" activationType="protocol""#, xml_escape(launch_url))
    };
    format!(
        r#"<toast{launch}><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><actions>{}</actions></toast>"#,
        xml_escape(title),
        xml_escape(body),
        actions
    )
}

// -------------------------------------------------------------------- activation urls

/// What a custom-URL activation asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Activation {
    /// The body of a toast: open the app.
    Open { nonce: String },
    /// A button: run its action.
    Run { nonce: String },
}

pub fn activation_url(kind: &str, nonce: &str) -> String {
    format!("{}://{kind}?n={nonce}", scheme())
}

/// Parse what Windows hands over. Strict: our scheme, a known host, one `n`
/// parameter of exactly 32 hex digits. Anything else is `None`.
pub fn parse_activation(raw: &str) -> Option<Activation> {
    let raw = raw.trim().trim_matches('"');
    let rest = raw.strip_prefix(&format!("{}://", scheme()))?;
    let (head, query) = rest.split_once('?')?;
    let host = head.trim_end_matches('/');
    let nonce = query.strip_prefix("n=")?;
    let valid = nonce.len() == 32 && nonce.bytes().all(|b| b.is_ascii_hexdigit()) && !nonce.contains('&');
    if !valid {
        return None;
    }
    let nonce = nonce.to_ascii_lowercase();
    match host {
        "open" => Some(Activation::Open { nonce }),
        "run" => Some(Activation::Run { nonce }),
        _ => None,
    }
}

// -------------------------------------------------------------------- command lines

/// The `Run` key value and the scheduled task's action.
pub fn check_command(exe: &Path) -> String {
    format!("\"{}\" --notify-check", exe.display())
}

/// The URL handler's command: Windows substitutes `%1`.
pub fn handler_command(exe: &Path) -> String {
    format!("\"{}\" --motd-action \"%1\"", exe.display())
}

// -------------------------------------------------------------------- registration

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("{0}")]
    Failed(String),
}

/// The names one registration is written under. The application uses the real ones
/// ([`Identity::real`]); the live test writes under names of its own, so that it can
/// register, run and remove everything for real without ever touching the user's.
#[derive(Debug, Clone)]
pub struct Identity {
    /// The value in the `Run` key.
    pub run_value: String,
    /// The custom URL scheme.
    pub scheme: String,
    /// The toast sender's identity.
    pub aumid: String,
    /// What Windows shows for both.
    pub display: String,
    /// The scheduled task.
    pub task: String,
}

impl Identity {
    pub fn real() -> Self {
        Self {
            run_value: display_name().to_string(),
            scheme: scheme().to_string(),
            aumid: aumid().to_string(),
            display: display_name().to_string(),
            task: task_name(),
        }
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::process::CommandExt;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn fail(what: &str, e: impl std::fmt::Display) -> RegisterError {
        RegisterError::Failed(format!("{what} : {e}"))
    }

    /// Idempotent: also what heals a moved executable at the next launch.
    pub fn register(exe: &Path) -> Result<(), RegisterError> {
        register_as(&Identity::real(), exe)
    }

    pub fn register_as(id: &Identity, exe: &Path) -> Result<(), RegisterError> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);

        let (run, _) = hkcu
            .create_subkey_with_flags(RUN_KEY, KEY_WRITE)
            .map_err(|e| fail("clé de démarrage", e))?;
        run.set_value(&id.run_value, &check_command(exe))
            .map_err(|e| fail("clé de démarrage", e))?;

        let scheme_key = format!(r"Software\Classes\{}", id.scheme);
        let (root, _) = hkcu.create_subkey(&scheme_key).map_err(|e| fail("gestionnaire d'URL", e))?;
        root.set_value("", &format!("URL:{}", id.display)).map_err(|e| fail("gestionnaire d'URL", e))?;
        root.set_value("URL Protocol", &"").map_err(|e| fail("gestionnaire d'URL", e))?;
        let (cmd, _) = hkcu
            .create_subkey(format!(r"{scheme_key}\shell\open\command"))
            .map_err(|e| fail("gestionnaire d'URL", e))?;
        cmd.set_value("", &handler_command(exe)).map_err(|e| fail("gestionnaire d'URL", e))?;

        // The sender's identity: the name Windows shows on the toast.
        let (ident, _) = hkcu
            .create_subkey(format!(r"Software\Classes\AppUserModelId\{}", id.aumid))
            .map_err(|e| fail("identité de notification", e))?;
        ident.set_value("DisplayName", &id.display).map_err(|e| fail("identité de notification", e))?;

        // Every 6 hours while the session is open; the Run key covers the logon.
        let out = std::process::Command::new("schtasks")
            .args(["/Create", "/TN", &id.task, "/SC", "HOURLY", "/MO", "6", "/TR", &check_command(exe), "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| fail("tâche planifiée", e))?;
        if !out.status.success() {
            return Err(fail("tâche planifiée", String::from_utf8_lossy(&out.stderr).trim().to_string()));
        }
        Ok(())
    }

    /// Remove everything `register` wrote. A missing entry is not an error.
    pub fn unregister() {
        unregister_as(&Identity::real());
    }

    pub fn unregister_as(id: &Identity) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(run) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_WRITE) {
            let _ = run.delete_value(&id.run_value);
        }
        let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{}", id.scheme));
        let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\AppUserModelId\{}", id.aumid));
        let _ = std::process::Command::new("schtasks")
            .args(["/Delete", "/TN", &id.task, "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    /// Raise the toast. The caller keeps the process alive for a moment: the
    /// platform takes the notification over, but not before `Show` has returned.
    pub fn show_toast(xml: &str) -> Result<(), RegisterError> {
        use windows::core::HSTRING;
        use windows::Data::Xml::Dom::XmlDocument;
        use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
        use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

        // Already initialised on this thread is fine (S_FALSE / changed mode).
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let doc = XmlDocument::new().map_err(|e| fail("notification", e))?;
        doc.LoadXml(&HSTRING::from(xml)).map_err(|e| fail("notification", e))?;
        let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| fail("notification", e))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid()))
            .and_then(|n| n.Show(&toast))
            .map_err(|e| fail("notification", e))
    }

    /// Same toast, but under a tag: a new one with the tag replaces the old one in the
    /// notification centre instead of piling up. `silent` updates the entry without
    /// raising a banner (the retry counter moving is not worth interrupting for).
    pub fn show_toast_tagged(xml: &str, tag: &str, silent: bool) -> Result<(), RegisterError> {
        use windows::core::HSTRING;
        use windows::Data::Xml::Dom::XmlDocument;
        use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
        use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let doc = XmlDocument::new().map_err(|e| fail("notification", e))?;
        doc.LoadXml(&HSTRING::from(xml)).map_err(|e| fail("notification", e))?;
        let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| fail("notification", e))?;
        toast.SetTag(&HSTRING::from(tag)).map_err(|e| fail("notification", e))?;
        toast.SetGroup(&HSTRING::from(aumid())).map_err(|e| fail("notification", e))?;
        if silent {
            toast.SetSuppressPopup(true).map_err(|e| fail("notification", e))?;
        }
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid()))
            .and_then(|n| n.Show(&toast))
            .map_err(|e| fail("notification", e))
    }

    /// The sender identity alone (the name Windows shows on a toast), without the
    /// URL handler and the scheduled task the message notifications need.
    pub fn ensure_toast_identity() -> Result<(), RegisterError> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (ident, _) = hkcu
            .create_subkey(format!(r"Software\Classes\AppUserModelId\{}", aumid()))
            .map_err(|e| fail("identité de notification", e))?;
        ident.set_value("DisplayName", &display_name()).map_err(|e| fail("identité de notification", e))
    }

    fn guard_run_value() -> String {
        format!("{} (background)", display_name())
    }

    /// The logon entry that starts the resident process, window hidden.
    pub fn set_guard_autostart(exe: &Path, on: bool) -> Result<(), RegisterError> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (run, _) = hkcu.create_subkey_with_flags(RUN_KEY, KEY_WRITE).map_err(|e| fail("clé de démarrage", e))?;
        if on {
            run.set_value(guard_run_value(), &format!("\"{}\" --background", exe.display()))
                .map_err(|e| fail("clé de démarrage", e))
        } else {
            let _ = run.delete_value(guard_run_value());
            Ok(())
        }
    }

    /// The system language tag (`fr-FR`), from the user's regional settings.
    pub fn system_locale() -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Control Panel\International")
            .ok()?
            .get_value::<String, _>("LocaleName")
            .ok()
    }
}

#[cfg(windows)]
pub use win::{ensure_toast_identity, register, set_guard_autostart, show_toast, show_toast_tagged, system_locale, unregister};
#[cfg(all(windows, test))]
use win::{register_as, unregister_as};

#[cfg(not(windows))]
pub fn register(_exe: &Path) -> Result<(), RegisterError> {
    Err(RegisterError::Failed("Windows uniquement".into()))
}
#[cfg(not(windows))]
pub fn unregister() {}
#[cfg(not(windows))]
pub fn show_toast(_xml: &str) -> Result<(), RegisterError> {
    Err(RegisterError::Failed("Windows uniquement".into()))
}
#[cfg(not(windows))]
pub fn show_toast_tagged(_xml: &str, _tag: &str, _silent: bool) -> Result<(), RegisterError> {
    Err(RegisterError::Failed("Windows uniquement".into()))
}
#[cfg(not(windows))]
pub fn ensure_toast_identity() -> Result<(), RegisterError> {
    Err(RegisterError::Failed("Windows uniquement".into()))
}
#[cfg(not(windows))]
pub fn set_guard_autostart(_exe: &Path, _on: bool) -> Result<(), RegisterError> {
    Err(RegisterError::Failed("Windows uniquement".into()))
}
#[cfg(not(windows))]
pub fn system_locale() -> Option<String> {
    None
}

// -------------------------------------------------------------------- update toast

/// Does this offer deserve a toast? Not when the version is out of reach for this
/// client (`upgrade_blocked`: it must reach a pivot first), and only for the
/// installer: the portable folder cannot be registered at all.
pub fn wants_update_toast(offer: &crate::update::UpdateAvailable) -> bool {
    !offer.upgrade_blocked
        && offer.artifacts.first().is_some_and(|a| a.kind == "nsis")
}

/// One toast per version, and none for a version the window already announced.
pub fn should_notify_update(version: &str, announced_by_window: Option<&str>, last: Option<&str>) -> bool {
    announced_by_window != Some(version) && last != Some(version)
}

pub fn update_button_label(lang: &str) -> &'static str {
    if lang == "fr" { "Mettre à jour" } else { "Update now" }
}

/// Title and body of the update toast.
pub fn update_texts(lang: &str, product: &str, version: &str, current: &str) -> (String, String) {
    if lang == "fr" {
        (
            "Mise à jour disponible".to_string(),
            format!("{product} {version} est prête. Vous avez la version {current}."),
        )
    } else {
        (
            "Update available".to_string(),
            format!("{product} {version} is ready. You are on version {current}."),
        )
    }
}

/// The toast announcing `version`: the body opens the app, the one button
/// updates. Records the version so the same one is announced once.
pub fn build_update_toast(
    version: &str,
    current: &str,
    lang: &str,
    state: &mut NotifyState,
    base: &str,
    now: i64,
) -> String {
    let (title, body) = update_texts(lang, display_name(), version, current);
    let launch = activation_url("open", &state.issue(&format!("update:{version}"), None, base, now));
    let button = (
        update_button_label(lang).to_string(),
        activation_url("run", &state.issue_update(version, base, now)),
    );
    state.last_update_notified = Some(version.to_string());
    toast_xml(&title, &body, &launch, &[button])
}

// -------------------------------------------------------------------- building a toast

/// Everything `--notify-check` needs to raise the toast of `message`, and the
/// state to save before showing it. Pure but for the nonce randomness, so the
/// shape is testable.
pub fn build_toast(
    message: &MotdMessage,
    lang: &str,
    state: &mut NotifyState,
    base: &str,
    now: i64,
) -> String {
    let title = {
        let t = resolve_text(&message.title, lang);
        if t.is_empty() { default_title(lang).to_string() } else { plain_text(&t, 80) }
    };
    let body = plain_text(&resolve_text(&message.body, lang), 220);
    let launch = activation_url("open", &state.issue(&message.id, None, base, now));
    let buttons: Vec<(String, String)> = message
        .actions
        .iter()
        .take(MAX_BUTTONS)
        .map(|a| {
            let nonce = state.issue(&message.id, Some(&a.id), base, now);
            (button_label(a, lang), activation_url("run", &nonce))
        })
        .collect();
    state.last_notified_id = Some(message.id.clone());
    toast_xml(&title, &body, &launch, &buttons)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn nonce32() -> String {
        "0123456789abcdef0123456789abcdef".to_string()
    }

    fn msg(json: &str) -> MotdMessage {
        serde_json::from_str(json).unwrap()
    }

    // ── decisions ──

    #[test]
    fn one_toast_per_message_and_never_for_a_forgotten_one() {
        assert!(should_notify("m1", None, None));
        assert!(should_notify("m2", Some("m1"), Some("m1")), "a new id notifies again");
        assert!(!should_notify("m1", Some("m1"), None), "asked to forget");
        assert!(!should_notify("m1", None, Some("m1")), "already announced");
    }

    #[test]
    fn language_falls_back_config_then_system_then_english() {
        assert_eq!(pick_language(Some("fr"), Some("en-US")), "fr");
        assert_eq!(pick_language(None, Some("fr-FR")), "fr");
        assert_eq!(pick_language(None, Some("de-DE")), "en");
        assert_eq!(pick_language(None, None), "en");
        assert_eq!(pick_language(Some("de"), Some("fr-FR")), "fr", "an unshipped language does not hide the system's");
    }

    #[test]
    fn labels_use_the_authors_text_then_the_default_of_the_kind() {
        let a: MotdAction = serde_json::from_str(r#"{"id":"a","kind":"restart_steam","label":{"fr":"Relancer"}}"#).unwrap();
        assert_eq!(button_label(&a, "fr"), "Relancer");
        assert_eq!(button_label(&a, "en"), "Relancer", "falls back on the language that exists");
        let b: MotdAction = serde_json::from_str(r#"{"id":"b","kind":"restart_steam"}"#).unwrap();
        assert_eq!(button_label(&b, "en"), "Restart Steam");
        assert_eq!(button_label(&b, "fr"), "Redémarrer Steam");
    }

    #[test]
    fn resolve_text_matches_the_windows_fallback() {
        let mut m = HashMap::new();
        m.insert("fr".to_string(), "Bonjour".to_string());
        m.insert("de".to_string(), "Hallo".to_string());
        assert_eq!(resolve_text(&m, "fr"), "Bonjour");
        assert_eq!(resolve_text(&m, "en"), "Bonjour", "en missing: fr");
        m.remove("fr");
        assert_eq!(resolve_text(&m, "en"), "Hallo", "any language beats none");
        assert_eq!(resolve_text(&HashMap::new(), "en"), "");
    }

    // ── plain text ──

    #[test]
    fn plain_text_drops_the_dialects_markers() {
        let md = "# Titre\nLe serveur est {red}**indisponible**{/} : voir [l'état](https://example.com/s).\n- puce *une*\n- `code`";
        assert_eq!(
            plain_text(md, 500),
            "Titre Le serveur est indisponible : voir l'état. puce une code"
        );
    }

    #[test]
    fn plain_text_keeps_braces_that_are_not_colour_tags() {
        assert_eq!(plain_text("valeur {pas un tag!} ici", 100), "valeur {pas un tag!} ici");
        assert_eq!(plain_text("{#ff8800}orange{/}", 100), "orange");
        assert_eq!(plain_text("accolade { ouverte", 100), "accolade { ouverte");
    }

    #[test]
    fn plain_text_cuts_with_an_ellipsis_on_characters_not_bytes() {
        let long = "é".repeat(300);
        let cut = plain_text(&long, 50);
        assert_eq!(cut.chars().count(), 50);
        assert!(cut.ends_with('…'));
        assert_eq!(plain_text("court", 50), "court");
    }

    // ── toast xml ──

    #[test]
    fn toast_xml_escapes_every_dynamic_value() {
        let xml = toast_xml(
            "T <b>\"x\"</b>",
            "a & b < c",
            "luavault://open?n=1",
            &[("L'un \"deux\"".to_string(), "luavault://run?n=2&x=<y>".to_string())],
        );
        assert!(!xml.contains("<b>"), "{xml}");
        assert!(xml.contains("T &lt;b&gt;&quot;x&quot;&lt;/b&gt;"));
        assert!(xml.contains("a &amp; b &lt; c"));
        assert!(xml.contains(r#"content="L&apos;un &quot;deux&quot;""#));
        assert!(xml.contains("n=2&amp;x=&lt;y&gt;"));
        // The document is well formed enough to count its own elements.
        assert_eq!(xml.matches("<action ").count(), 1);
    }

    #[test]
    fn a_result_toast_has_no_launch_url() {
        let xml = toast_xml("t", "b", "", &[]);
        assert!(xml.starts_with("<toast>"), "{xml}");
        assert!(!xml.contains("launch"));
    }

    #[test]
    fn toast_xml_carries_at_most_two_buttons() {
        let b: Vec<(String, String)> = (0..5).map(|i| (format!("b{i}"), format!("u{i}"))).collect();
        assert_eq!(toast_xml("t", "b", "l", &b).matches("<action ").count(), MAX_BUTTONS);
    }

    // ── activation urls ──

    #[test]
    fn activation_urls_roundtrip() {
        let n = nonce32();
        assert_eq!(parse_activation(&activation_url("run", &n)), Some(Activation::Run { nonce: n.clone() }));
        assert_eq!(parse_activation(&activation_url("open", &n)), Some(Activation::Open { nonce: n.clone() }));
        // Windows may add a slash and quotes.
        let slashed = format!("\"{}://run/?n={n}\"", scheme());
        assert_eq!(parse_activation(&slashed), Some(Activation::Run { nonce: n }));
    }

    #[test]
    fn activation_parser_refuses_everything_that_is_not_ours() {
        let n = nonce32();
        let s = scheme();
        for bad in [
            String::new(),
            format!("other://run?n={n}"),
            format!("{s}://format?n={n}"),
            format!("{s}://run"),
            format!("{s}://run?n="),
            format!("{s}://run?n={}", &n[..31]),
            format!("{s}://run?n={n}0"),
            format!("{s}://run?n={}g", &n[..31]),
            format!("{s}://run?n={n}&a=restart_steam"),
            format!("{s}://run?a=restart_steam&n={n}"),
            format!("{s}:run?n={n}"),
        ] {
            assert!(parse_activation(&bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn command_lines_quote_the_executable() {
        let exe = Path::new(r"C:\Program Files\LuaVault\luavault.exe");
        assert_eq!(check_command(exe), r#""C:\Program Files\LuaVault\luavault.exe" --notify-check"#);
        assert_eq!(
            handler_command(exe),
            r#""C:\Program Files\LuaVault\luavault.exe" --motd-action "%1""#
        );
    }

    // ── nonces ──

    #[test]
    fn a_nonce_is_single_use() {
        let mut s = NotifyState::default();
        let n = s.issue("m1", Some("a"), "http://x", 1_000);
        let first = s.consume(&n, 1_001).expect("fresh");
        assert_eq!((first.message_id.as_str(), first.action_id.as_deref()), ("m1", Some("a")));
        assert!(s.consume(&n, 1_002).is_none(), "replay");
    }

    #[test]
    fn an_unknown_or_expired_nonce_does_nothing() {
        let mut s = NotifyState::default();
        assert!(s.consume(&nonce32(), 0).is_none(), "never issued: a forged URL");
        let n = s.issue("m1", None, "http://x", 1_000);
        assert!(s.consume(&n, 1_000 + PENDING_TTL_SECS).is_none(), "the toast is too old");
        assert!(s.pending.is_empty(), "expired entries are pruned");
    }

    #[test]
    fn an_update_nonce_names_its_version_and_is_single_use() {
        let mut s = NotifyState::default();
        let n = s.issue_update("1.5.0", "http://x", 10);
        let p = s.consume(&n, 11).expect("fresh");
        assert_eq!(p.update_version.as_deref(), Some("1.5.0"));
        assert_eq!(p.action_id, None, "an update button is not a message action");
        assert!(s.consume(&n, 12).is_none(), "replay");
    }

    #[test]
    fn a_message_action_is_never_an_update() {
        // An action id spelling "update" must not be mistaken for the update button.
        let mut s = NotifyState::default();
        let n = s.issue("m1", Some("update"), "http://x", 0);
        assert_eq!(s.consume(&n, 1).unwrap().update_version, None);
    }

    #[test]
    fn a_state_written_before_updates_existed_still_reads() {
        let old = r#"{"last_notified_id":"m","pending":[{"nonce":"n","message_id":"m","action_id":null,"created_at":1,"base":"b"}]}"#;
        let s: NotifyState = serde_json::from_str(old).unwrap();
        assert_eq!(s.pending[0].update_version, None);
        assert_eq!(s.last_update_notified, None);
    }

    #[test]
    fn one_update_toast_per_version() {
        assert!(should_notify_update("1.5.0", None, None));
        assert!(!should_notify_update("1.5.0", Some("1.5.0"), None), "the window said it");
        assert!(!should_notify_update("1.5.0", None, Some("1.5.0")), "already announced");
        assert!(should_notify_update("1.5.1", Some("1.5.0"), Some("1.5.0")), "a newer version notifies again");
    }

    fn offer(kind: &str, blocked: bool) -> crate::update::UpdateAvailable {
        crate::update::UpdateAvailable {
            version: "1.5.0".into(),
            published_at: "2026-10-17T00:00:00Z".into(),
            notes: None,
            notes_i18n: None,
            artifacts: vec![crate::update::UpdateArtifact {
                kind: kind.into(),
                file: "x.exe".into(),
                size: 1,
                sha256: "0".repeat(64),
            }],
            upgrade_blocked: blocked,
            minimum_upgradable_from: None,
            changes: vec![],
        }
    }

    #[test]
    fn a_toast_is_wanted_only_for_an_installer_upgrade() {
        assert!(wants_update_toast(&offer("nsis", false)));
        assert!(!wants_update_toast(&offer("nsis", true)), "blocked behind a pivot version");
        assert!(!wants_update_toast(&offer("portable", false)), "portable has no background");
        let mut none = offer("nsis", false);
        none.artifacts.clear();
        assert!(!wants_update_toast(&none), "nothing to download");
    }

    #[test]
    fn the_update_toast_has_one_button_and_carries_no_version_in_its_urls() {
        let mut st = NotifyState::default();
        let xml = build_update_toast("1.5.0", "1.4.4", "fr", &mut st, "http://base.test", 100);
        assert_eq!(xml.matches("<action ").count(), 1);
        assert!(xml.contains("Mettre à jour"));
        assert!(xml.contains("Mise à jour disponible"));
        assert!(xml.contains("est prête. Vous avez la version 1.4.4."));
        assert_eq!(st.pending.len(), 2, "the body and the button");
        assert_eq!(st.pending.iter().filter(|p| p.update_version.is_some()).count(), 1);
        assert_eq!(st.last_update_notified.as_deref(), Some("1.5.0"));
        // The URLs carry nonces only: not the version, not the file.
        for p in &st.pending {
            assert!(xml.contains(&p.nonce));
        }
        assert!(!xml.contains("n=1.5.0") && !xml.contains(".exe"));
    }

    #[test]
    fn the_update_toast_speaks_english_when_asked() {
        let xml = build_update_toast("1.5.0", "1.4.4", "en", &mut NotifyState::default(), "b", 0);
        assert!(xml.contains("Update available") && xml.contains("Update now"));
    }

    #[test]
    fn a_nonce_remembers_where_its_message_came_from() {
        // The click handler starts with Windows' environment, not the check's.
        let mut s = NotifyState::default();
        let n = s.issue("m1", Some("a"), "http://127.0.0.1:8765", 10);
        assert_eq!(s.consume(&n, 11).unwrap().base.as_deref(), Some("http://127.0.0.1:8765"));
    }

    #[test]
    fn a_state_written_before_the_base_existed_still_reads() {
        let old = r#"{"last_notified_id":"m","pending":[{"nonce":"n","message_id":"m","action_id":null,"created_at":1}]}"#;
        let s: NotifyState = serde_json::from_str(old).unwrap();
        assert_eq!(s.pending[0].base, None);
    }

    #[test]
    fn nonces_are_128_bits_of_hex_and_distinct() {
        let mut s = NotifyState::default();
        let a = s.issue("m", None, "http://x", 0);
        let b = s.issue("m", None, "http://x", 0);
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn state_survives_a_disk_roundtrip_and_tolerates_garbage() {
        let mut s = NotifyState { last_notified_id: Some("m1".into()), ..Default::default() };
        s.issue("m1", Some("a"), "http://x", 5);
        let back: NotifyState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(serde_json::from_str::<NotifyState>("{}").unwrap(), NotifyState::default());
    }

    // ── building a toast ──

    #[test]
    fn build_toast_issues_a_nonce_per_button_and_one_for_the_body() {
        let m = msg(
            r#"{"id":"m1","published_at":"2026-10-17T10:00:00Z","title":{"fr":"Correctif"},"body":{"fr":"Steam **pose** problème."},
               "actions":[{"id":"a","kind":"steam_downgrade"},{"id":"b","kind":"restart_steam"},{"id":"c","kind":"steam_undo"}]}"#,
        );
        let mut st = NotifyState::default();
        let xml = build_toast(&m, "fr", &mut st, "http://base.test", 100);
        assert_eq!(st.pending.len(), 3, "the body and two buttons, not three");
        assert!(st.pending.iter().all(|p| p.base.as_deref() == Some("http://base.test")));
        assert_eq!(st.pending.iter().filter(|p| p.action_id.is_none()).count(), 1);
        assert_eq!(st.last_notified_id.as_deref(), Some("m1"));
        assert!(xml.contains("<text>Correctif</text>"));
        assert!(xml.contains("Steam pose problème."));
        assert!(xml.contains("Appliquer le correctif Steam") && xml.contains("Redémarrer Steam"));
        assert!(!xml.contains("Rétablir"), "the third action is not shown");
        // No action id and no parameter in any URL: only nonces.
        for p in &st.pending {
            assert!(xml.contains(&p.nonce));
        }
        assert!(!xml.contains("steam_downgrade") && !xml.contains("restart_steam"));
    }

    #[test]
    fn build_toast_without_a_title_uses_the_default() {
        let m = msg(r#"{"id":"m1","published_at":"x","body":{"en":"Hello"}}"#);
        let xml = build_toast(&m, "en", &mut NotifyState::default(), "http://x", 0);
        assert!(xml.contains("<text>Message of the day</text>"));
        assert_eq!(xml.matches("<action ").count(), 0);
    }

    // ── live: the registration, for real ──

    /// Poll a log file for a line, up to `secs`.
    #[cfg(windows)]
    fn wait_for_log(path: &Path, needle: &str, secs: u64) -> bool {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        while std::time::Instant::now() < end {
            if std::fs::read_to_string(path).is_ok_and(|t| t.contains(needle)) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        false
    }

    /// The only check that the registration works on Windows, not just that its
    /// strings look right: write it all under names of its own, then make Windows
    /// start the real executable three ways — the scheduled task, the `Run` command,
    /// the URL handler's command — and read what it did in `notify.log`. Then remove
    /// everything and verify it is gone. The executable is a copy beside a portable
    /// marker, so its data (and its log) live in a temp folder, and with no choice
    /// made the check returns before any network or toast.
    ///
    /// `cargo build && cargo test live_registration -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore = "writes HKCU and the task scheduler under unique names, then removes them"]
    fn live_registration_runs_the_real_executable_three_ways_then_cleans_up() {
        use std::os::windows::process::CommandExt;
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        // The executable's file name in `target\debug`, and the marker that makes a copy of
        // it portable: its data, and so its log, then live beside it.
        const BIN_NAME: &str = "luavault.exe";
        const PORTABLE_MARKER: &str = "LuaVault.portable";

        let built = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("debug").join(BIN_NAME);
        assert!(built.is_file(), "build the app first (cargo build): {}", built.display());

        let tag = format!("{}-{}", std::process::id(), chrono::Utc::now().timestamp());
        let dir = std::env::temp_dir().join(format!("live-reg-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(BIN_NAME);
        std::fs::copy(&built, &exe).unwrap();
        std::fs::write(dir.join(PORTABLE_MARKER), "").unwrap();
        let log = dir.join("notify.log");

        let id = Identity {
            run_value: format!("LiveReg-{tag}"),
            scheme: format!("livereg-{tag}"),
            aumid: format!("cloud.test.livereg-{tag}"),
            display: format!("LiveReg-{tag}"),
            task: format!("LiveReg-{tag} MOTD"),
        };

        // Removes everything even when an assertion fails halfway.
        struct Cleanup(Identity, PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unregister_as(&self.0);
                let _ = std::fs::remove_dir_all(&self.1);
            }
        }
        let _cleanup = Cleanup(id.clone(), dir.clone());

        register_as(&id, &exe).expect("register");

        // 1. what was written
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run_cmd: String = hkcu
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
            .unwrap()
            .get_value(&id.run_value)
            .expect("Run value");
        assert_eq!(run_cmd, check_command(&exe));
        let handler: String = hkcu
            .open_subkey(format!(r"Software\Classes\{}\shell\open\command", id.scheme))
            .unwrap()
            .get_value("")
            .expect("handler command");
        assert_eq!(handler, handler_command(&exe));
        let shown: String = hkcu
            .open_subkey(format!(r"Software\Classes\AppUserModelId\{}", id.aumid))
            .unwrap()
            .get_value("DisplayName")
            .expect("toast identity");
        assert_eq!(shown, id.display);

        // 2. the task as the scheduler stored it
        let xml = std::process::Command::new("schtasks")
            .args(["/Query", "/TN", &id.task, "/XML"])
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        let xml = String::from_utf8_lossy(&xml.stdout).to_string();
        assert!(xml.contains("--notify-check"), "arguments missing from the task: {xml}");
        assert!(xml.to_ascii_lowercase().contains(&BIN_NAME.to_ascii_lowercase()), "executable missing: {xml}");
        assert!(xml.contains("PT6H"), "every 6 hours expected: {xml}");

        // 3. Windows starts the real executable: through the scheduler...
        let run = std::process::Command::new("schtasks")
            .args(["/Run", "/TN", &id.task])
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(run.status.success(), "schtasks /Run: {}", String::from_utf8_lossy(&run.stderr));
        assert!(
            wait_for_log(&log, "vérification lancée", 45),
            "the scheduled task never ran the executable (no line in {})",
            log.display()
        );

        // ...through the command the `Run` key holds (what logon executes)...
        let _ = std::fs::remove_file(&log);
        std::process::Command::new("cmd")
            .raw_arg(format!("/S /C \"{run_cmd}\""))
            .creation_flags(0x0800_0000)
            .status()
            .unwrap();
        assert!(wait_for_log(&log, "vérification lancée", 30), "the Run command did not run the executable");

        // ...and through the URL handler's command, `%1` replaced as Windows does.
        // A well-formed URL whose nonce was never issued: found nowhere, ignored.
        let _ = std::fs::remove_file(&log);
        let url = format!("{}://run?n=00000000000000000000000000000000", scheme());
        std::process::Command::new("cmd")
            .raw_arg(format!("/S /C \"{}\"", handler.replace("%1", &url)))
            .creation_flags(0x0800_0000)
            .status()
            .unwrap();
        assert!(
            wait_for_log(&log, "jeton inconnu", 30),
            "the URL handler's command did not reach the activation parser: {:?}",
            std::fs::read_to_string(&log)
        );

        // 4. removal, verified
        unregister_as(&id);
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        assert!(hkcu
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
            .unwrap()
            .get_value::<String, _>(&id.run_value)
            .is_err());
        assert!(hkcu.open_subkey(format!(r"Software\Classes\{}", id.scheme)).is_err());
        assert!(hkcu.open_subkey(format!(r"Software\Classes\AppUserModelId\{}", id.aumid)).is_err());
        let gone = std::process::Command::new("schtasks")
            .args(["/Query", "/TN", &id.task])
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(!gone.status.success(), "the scheduled task is still there");
    }
}

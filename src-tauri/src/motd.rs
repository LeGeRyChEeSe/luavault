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
//! The text itself is a small markdown dialect rendered by the frontend
//! (`src/lib/motd-markdown.ts`) without ever going through `{@html}`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use log::debug;
use serde::{Deserialize, Serialize};

use crate::update::{self, UpdateClient, RELEASE_PUBLIC_KEYS};

/// The document schema this client understands.
pub const SCHEMA: u32 = 1;

/// Hard ceiling for `motd.json`. A notice is a few paragraphs in two or three
/// languages; 64 KiB is already ten times what a reasonable one needs.
pub const MAX_MOTD_SIZE: u64 = 64 * 1024;

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
    Some(message)
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
}

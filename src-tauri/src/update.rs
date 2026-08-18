//! Client-side update verification — manifest fetch, signature check, version
//! comparison, and artefact download with integrity validation.
//!
//! Security invariants (see UPDATE_PROTOCOL.md):
//! - Signature is verified on the RAW BYTES before any deserialization.
//! - Both release keys are accepted from day one (rotation safety).
//! - A schema newer than ours is a silent abort, never an error.
//! - Unknown artifact kinds are ignored, not rejected.
//! - A failed network call is the nominal offline case: no visible error.
//! - Nothing is ever read from the network without a hard size cap, and the
//!   update client performs no decompression — an unauthenticated manifest
//!   response cannot amplify into gigabytes of RAM.

use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, VerifyingKey};
use log::debug;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::hmac;

// -------------------------------------------------------------------- keys

/// Signature keys for release manifests. Both are accepted: see UPDATE_PROTOCOL.md —
/// with a single compiled-in key, rotating it would lock every installed client out of
/// the very update that fixes the rotation.
pub const RELEASE_PUBLIC_KEYS: [&str; 2] = [
    // primaire
    "10a4ea19d2f8e680205c0c83d81f2c6eb97629aa43126c4cbc1b633f070cdcac",
    // secours
    "5a20b4f3ddd5aa2192cc7148ad4a38ec146460d1101f2b8d4701d7673445414f",
];

/// The manifest schema version this client understands.
pub const SCHEMA: u32 = 1;

// -------------------------------------------------------------------- size caps

/// Hard ceiling for a single artefact, refused before anything is read.
pub const MAX_ARTIFACT_SIZE: u64 = 256 * 1024 * 1024; // 256 MiB
/// Hard ceiling for manifest.json.
pub const MAX_MANIFEST_SIZE: u64 = 256 * 1024; // 256 KiB
/// Hard ceiling for manifest.json.sig (a base64 Ed25519 signature is 88 bytes).
pub const MAX_SIGNATURE_SIZE: u64 = 1024; // 1 KiB

// -------------------------------------------------------------------- types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateArtifact {
    pub kind: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

/// One entry in the release history, attached to a manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub notes_i18n: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub published_at: String,
    #[serde(default)]
    pub minimum_upgradable_from: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub notes_i18n: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    pub artifacts: Vec<UpdateArtifact>,
}

/// What the frontend receives when an update is available.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateAvailable {
    pub version: String,
    pub published_at: String,
    pub notes: Option<String>,
    pub notes_i18n: Option<std::collections::HashMap<String, String>>,
    pub artifacts: Vec<UpdateArtifact>,
    /// True when the local version is below `minimum_upgradable_from`: the view
    /// shows the information but hides the download buttons (UPDATE_PROTOCOL.md).
    pub upgrade_blocked: bool,
    /// The pivot version an older client must reach first, when the manifest
    /// imposes one.
    pub minimum_upgradable_from: Option<String>,
    /// History entries strictly newer than the local version, most recent first.
    pub changes: Vec<HistoryEntry>,
}

// -------------------------------------------------------------------- base URL

/// Production update server, overridable for local testing.
const DEFAULT_BASE: &str = "https://github.com/LeGeRyChEeSe/luavault/releases/latest/download";

pub fn base_url() -> String {
    std::env::var("LV_UPDATE_BASE").unwrap_or_else(|_| DEFAULT_BASE.to_string())
}

// -------------------------------------------------------------------- http client

/// Dedicated client for the update server. Deliberately NOT the LuaVault client:
/// no `Origin`/`Referer`/spoofed Chrome `User-Agent` default headers leak to
/// the update server, and no automatic decompression — a
/// compressed body must not be able to amplify past the size caps below.
/// (`deflate` is not compiled into reqwest here, so there is no `no_deflate()`
/// to call and no deflate decompression to fear.)
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .use_rustls_tls()
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .expect("failed to build update http client")
}

/// The client dedicated to the update server, in a newtype so the compiler — not a
/// test, and not the reviewer's attention — is what keeps the LuaVault client out of
/// here. Both are `reqwest::Client`, so `state.http` and `state.update_http` were
/// interchangeable at every call site: swapping them compiled, passed all 186 tests,
/// and silently put `Origin: https://LuaVault` back on every update request.
#[derive(Clone)]
pub struct UpdateClient(reqwest::Client);

impl UpdateClient {
    pub fn new() -> Self {
        Self(build_http_client())
    }

    fn inner(&self) -> &reqwest::Client {
        &self.0
    }

    /// Wrap an arbitrary client. Tests only: production goes through [`Self::new`],
    /// which is the whole point of the newtype.
    #[cfg(test)]
    fn wrap(client: reqwest::Client) -> Self {
        Self(client)
    }
}

impl Default for UpdateClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Read a response body in streaming, refusing to materialize more than `limit`
/// bytes. A response announcing more via `Content-Length` is rejected before a
/// single body byte is read; one that lies about its length is cut off mid-stream.
/// Never call `resp.bytes()` on an unauthenticated response — it materializes
/// everything before any check can run.
pub async fn read_capped(resp: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    if let Some(announced) = resp.content_length() {
        if announced > limit {
            bail!(
                "réponse annoncée à {} octets — plafond {} dépassé",
                announced,
                limit
            );
        }
    }
    let mut body: Vec<u8> = Vec::new();
    let mut resp = resp;
    while let Some(chunk) = resp.chunk().await.context("lecture de la réponse")? {
        if body.len() as u64 + chunk.len() as u64 > limit {
            bail!("réponse plus grande que le plafond {} octets", limit);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

// -------------------------------------------------------------------- signature

/// Verify the manifest bytes against a caller-supplied key list, returning the
/// index of the key that matched, or None. The production path passes
/// `RELEASE_PUBLIC_KEYS`; tests inject keys they hold the private half of.
///
/// A malformed key makes the loop move on to the next slot, never fail the whole
/// verification: the fallback key exists precisely for the day the primary slot
/// holds garbage, and a `?` here would make that day an impasse anyway.
pub fn verify_manifest_with_keys(
    bytes: &[u8],
    signature_b64: &str,
    keys: &[&str],
) -> Option<usize> {
    use base64::Engine;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .ok()?;
    let signature = Signature::from_slice(&sig_bytes).ok()?;

    for (i, key_hex) in keys.iter().enumerate() {
        let Ok(key_bytes) = hmac::hex_to_bytes(key_hex) else {
            continue;
        };
        let Ok(array): Result<[u8; 32], _> = key_bytes.try_into() else {
            continue;
        };
        let Ok(vk) = VerifyingKey::from_bytes(&array) else {
            continue;
        };
        if vk.verify_strict(bytes, &signature).is_ok() {
            return Some(i);
        }
    }
    None
}

// -------------------------------------------------------------------- manifest fetch

/// Fetch `manifest.json` + `manifest.json.sig`, verify the signature on the raw
/// bytes BEFORE deserializing, and enforce the schema cap. Every failure path is
/// the nominal offline/tampered case and yields `None` — never a visible error.
pub async fn fetch_verified_manifest(http: &UpdateClient, base: &str) -> Option<Manifest> {
    fetch_verified_manifest_with_keys(http, base, &RELEASE_PUBLIC_KEYS).await
}

/// [`fetch_verified_manifest`] over a caller-supplied key list (tests).
pub async fn fetch_verified_manifest_with_keys(
    http: &UpdateClient,
    base: &str,
    keys: &[&str],
) -> Option<Manifest> {
    let (manifest_resp, sig_resp) = match tokio::join!(
        http.inner().get(format!("{base}/manifest.json")).send(),
        http.inner().get(format!("{base}/manifest.json.sig")).send(),
    ) {
        (Ok(m), Ok(s)) => (m, s),
        (Err(e), _) | (_, Err(e)) => {
            debug!("check_update: réseau indisponible ({e})");
            return None;
        }
    };

    // Capped streaming reads: both files are small and fully unauthenticated at
    // this point — a hostile server must not be able to drown us in RAM.
    let manifest_bytes = match read_capped(manifest_resp, MAX_MANIFEST_SIZE).await {
        Ok(b) => b,
        Err(e) => {
            debug!("check_update: lecture du manifeste impossible ({e})");
            return None;
        }
    };
    let sig_bytes = match read_capped(sig_resp, MAX_SIGNATURE_SIZE).await {
        Ok(b) => b,
        Err(e) => {
            debug!("check_update: lecture de la signature impossible ({e})");
            return None;
        }
    };
    let sig_text = match String::from_utf8(sig_bytes) {
        Ok(s) => s,
        Err(_) => {
            debug!("check_update: signature non-UTF8 — rejetée");
            return None;
        }
    };

    // Verify signature BEFORE deserializing anything.
    if verify_manifest_with_keys(&manifest_bytes, &sig_text, keys).is_none() {
        debug!("check_update: signature invalide — manifeste rejeté");
        return None;
    }

    let manifest: Manifest = match serde_json::from_slice(&manifest_bytes) {
        Ok(m) => m,
        Err(e) => {
            debug!("check_update: manifeste illisible ({e})");
            return None;
        }
    };

    // Schema newer than ours: silent abort (the update would fix this anyway).
    if manifest.schema > SCHEMA {
        debug!(
            "check_update: schema {} > {} — abandon silencieux",
            manifest.schema, SCHEMA
        );
        return None;
    }

    Some(manifest)
}

/// Choose the artifact that matches the current edition.
///
/// Portable → `kind == "portable"`. Installed → `kind == "nsis"`.
/// Returns `None` when the right variant is absent — no fallback.
pub fn preferred_artifact(
    artifacts: &[UpdateArtifact],
    portable: bool,
) -> Option<&UpdateArtifact> {
    artifacts.iter().find(|a| {
        if portable {
            a.kind == "portable"
        } else {
            a.kind == "nsis"
        }
    })
}

/// Decide what an authenticated manifest means for this client: version must be
/// strictly newer, unknown artifact kinds are filtered out, and a
/// `minimum_upgradable_from` pivot below which direct installation is blocked.
///
/// `portable` selects which artifact variant to return — the function returns
/// `None` when the right variant is absent (no fallback).
pub fn evaluate_manifest(manifest: Manifest, local_version: &str, portable: bool) -> Option<UpdateAvailable> {
    if !is_newer(&manifest.version, local_version) {
        return None;
    }

    // A client older than the pivot may not jump straight to this version:
    // surface the information, block direct installation (UPDATE_PROTOCOL.md).
    let upgrade_blocked = manifest
        .minimum_upgradable_from
        .as_ref()
        .map(|min| is_newer(min, local_version))
        .unwrap_or(false);

    // Filter to known artifact kinds, then pick the one matching this edition.
    let known: Vec<UpdateArtifact> = manifest
        .artifacts
        .into_iter()
        .filter(|a| a.kind == "nsis" || a.kind == "portable")
        .collect();

    let preferred = preferred_artifact(&known, portable).cloned();

    let changes = changes_since(&manifest.history, local_version);

    preferred.map(|artifact| UpdateAvailable {
        version: manifest.version,
        published_at: manifest.published_at,
        notes: manifest.notes,
        notes_i18n: manifest.notes_i18n,
        artifacts: vec![artifact],
        upgrade_blocked,
        minimum_upgradable_from: manifest.minimum_upgradable_from,
        changes,
    })
}

// -------------------------------------------------------------------- version comparison

/// Return history entries whose version is strictly newer than `current`, from
/// most recent to oldest. Uses [`is_newer`] for numeric comparison — never a raw
/// string comparison.
///
/// When `history` is empty the result is empty (never an error).
pub fn changes_since(history: &[HistoryEntry], current: &str) -> Vec<HistoryEntry> {
    let mut result: Vec<HistoryEntry> = history
        .iter()
        .filter(|e| is_newer(&e.version, current))
        .cloned()
        .collect();
    // Descending order: most recent first.
    result.sort_by(|a, b| {
        if is_newer(&b.version, &a.version) {
            std::cmp::Ordering::Greater  // b is newer → b comes first → a > b
        } else if is_newer(&a.version, &b.version) {
            std::cmp::Ordering::Less  // a is newer → a comes first → a < b
        } else {
            std::cmp::Ordering::Equal
        }
    });
    result
}

/// True when `remote` is strictly newer than `local`. Numeric per component: 1.10.0 is
/// newer than 1.9.0, which a string comparison gets backwards. At equal core, a
/// version WITHOUT a pre-release suffix is newer than one with a suffix: the 1.0.0
/// final must be offered to users sitting on 1.0.0-beta.
pub fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Option<(Vec<u64>, bool)> {
        let (core, prerelease) = match s.split_once('-') {
            Some((c, _suffix)) => (c, true),
            None => (s, false),
        };
        if core.is_empty() {
            return None;
        }
        let parts: Option<Vec<u64>> = core.split('.').map(|p| p.parse::<u64>().ok()).collect();
        parts.map(|p| (p, prerelease))
    };

    let ((r, r_pre), (l, l_pre)) = match (parse(remote), parse(local)) {
        (Some(r), Some(l)) => (r, l),
        _ => return false,
    };

    let max_len = r.len().max(l.len());
    for i in 0..max_len {
        let rv = r.get(i).copied().unwrap_or(0);
        let lv = l.get(i).copied().unwrap_or(0);
        if rv > lv {
            return true;
        }
        if rv < lv {
            return false;
        }
    }
    // Cores equal: a final release outranks its pre-releases.
    !r_pre && l_pre
}

// -------------------------------------------------------------------- path safety

/// Accept only a single, plain path segment: ASCII letters, digits, `.`, `-`, `_`.
/// Mirrors the server's `safe_segment` — the crate is not importable.
pub fn safe_segment(raw: &str) -> Option<&str> {
    if raw.is_empty() {
        return None;
    }
    if raw.len() > 255 {
        return None;
    }
    if raw == "." || raw == ".." {
        return None;
    }
    if raw.starts_with('.') {
        return None;
    }
    for c in raw.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '-' | '_' => {}
            _ => return None,
        }
    }
    Some(raw)
}

// -------------------------------------------------------------------- download + verify

/// Download an artefact, verify size then SHA-256. Returns the path of the verified
/// file, or an error (nothing is written on any mismatch).
pub async fn download_and_verify(
    http: &UpdateClient,
    version: &str,
    file: &str,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<String> {
    download_and_verify_from(
        http,
        &base_url(),
        version,
        file,
        expected_sha256,
        expected_size,
    )
    .await
}

/// [`download_and_verify`] against a caller-supplied base URL (tests).
pub async fn download_and_verify_from(
    http: &UpdateClient,
    base: &str,
    version: &str,
    file: &str,
    expected_sha256: &str,
    expected_size: u64,
) -> Result<String> {
    // Refuse oversized artefacts before a single byte is read.
    if expected_size > MAX_ARTIFACT_SIZE {
        bail!(
            "artefact annoncé à {} octets — plafond {} dépassé",
            expected_size,
            MAX_ARTIFACT_SIZE
        );
    }

    let v = safe_segment(version).context("version invalide")?;
    let f = safe_segment(file).context("nom de fichier invalide")?;

    let url = format!("{}/download/{}/{}", base, v, f);
    let resp = http
        .inner()
        .get(&url)
        .send()
        .await
        .context("téléchargement de la mise à jour")?;

    if !resp.status().is_success() {
        bail!("serveur de mise à jour : HTTP {}", resp.status());
    }

    // A server announcing more than the manifest promised is rejected before
    // any body byte is materialized.
    if let Some(announced) = resp.content_length() {
        if announced != expected_size {
            bail!(
                "taille annoncée incorrecte : attendu {} octets, serveur annonce {}",
                expected_size,
                announced
            );
        }
    }

    // Stream, and cut off the instant the cumulative total exceeds the expected
    // size — a lying or chunked response never materializes beyond that bound.
    let mut bytes: Vec<u8> = Vec::with_capacity(expected_size as usize);
    let mut resp = resp;
    while let Some(chunk) = resp.chunk().await.context("lecture de la réponse")? {
        if bytes.len() as u64 + chunk.len() as u64 > expected_size {
            bail!(
                "réponse plus grande que la taille attendue ({} octets)",
                expected_size
            );
        }
        bytes.extend_from_slice(&chunk);
    }

    // Exact size check (also catches truncated responses).
    if bytes.len() as u64 != expected_size {
        bail!(
            "taille incorrecte : attendu {} octets, reçu {}",
            expected_size,
            bytes.len()
        );
    }

    // SHA-256 check.
    let digest = Sha256::digest(&bytes);
    let hex_digest = hmac::bytes_to_hex(&digest);
    if !hex_digest.eq_ignore_ascii_case(expected_sha256) {
        bail!("empreinte SHA-256 incorrecte — le fichier a été rejeté");
    }

    // Write to temp directory only after both checks pass.
    let dir = std::env::temp_dir().join("luavault-updates");
    tokio::fs::create_dir_all(&dir)
        .await
        .context("création du dossier temporaire")?;
    let dest = dir.join(f);
    tokio::fs::write(&dest, &bytes)
        .await
        .context("écriture du fichier téléchargé")?;

    Ok(dest.to_string_lossy().to_string())
}

// -------------------------------------------------------------------- install validation

/// SHA-256 of a file on disk, streamed in 64 KiB blocks (no full-file buffering).
pub fn sha256_of_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).context("ouverture du fichier")?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).context("lecture du fichier")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hmac::bytes_to_hex(&hasher.finalize()))
}

/// Validate a path passed to `install_update`, returning the canonical path to
/// open. Three independent guards:
///
/// 1. The CANONICALIZED path must live inside the dedicated download directory —
///    `Path::starts_with` compares components and `..` is one, so a non-canonical
///    `%TEMP%\luavault-updates\..\..\..\Windows\System32\calc.exe` sails
///    through a naive prefix check.
/// 2. It must be the very file `download_update` recorded — the directory is
///    world-writable for the user account, the path alone proves nothing.
/// 3. Its SHA-256 must still match the digest verified at download time — the
///    file can be rewritten between download and the "Installer" click.
pub fn validate_install_path(
    requested: &str,
    verified: &Option<(String, String)>,
) -> Result<std::path::PathBuf, String> {
    let (verified_path, expected_sha) = verified.as_ref().ok_or_else(|| {
        "aucun téléchargement vérifié en attente — retéléchargez la mise à jour".to_string()
    })?;

    let expected_dir = std::env::temp_dir().join("luavault-updates");
    let canon_dir = std::fs::canonicalize(&expected_dir)
        .map_err(|_| "dossier de téléchargement introuvable".to_string())?;
    let canon = std::fs::canonicalize(requested).map_err(|_| "le fichier n'existe plus".to_string())?;

    if !canon.starts_with(&canon_dir) {
        return Err(
            "chemin refusé : le fichier ne provient pas d'un téléchargement vérifié".to_string(),
        );
    }

    let canon_verified = std::fs::canonicalize(verified_path)
        .map_err(|_| "le fichier vérifié n'existe plus".to_string())?;
    if canon != canon_verified {
        return Err("chemin refusé : le fichier ne correspond pas au téléchargement vérifié".to_string());
    }

    let digest = sha256_of_file(&canon).map_err(|e| format!("lecture du fichier : {e}"))?;
    if !digest.eq_ignore_ascii_case(expected_sha) {
        return Err(
            "empreinte SHA-256 incorrecte — le fichier a été modifié depuis le téléchargement"
                .to_string(),
        );
    }

    Ok(canon)
}

// -------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::collections::HashMap;

    // ── preferred_artifact ──

    #[test]
    fn preferred_artifact_portable_both_present() {
        let arts = vec![
            UpdateArtifact { kind: "nsis".into(), file: "setup.exe".into(), size: 0, sha256: "a".into() },
            UpdateArtifact { kind: "portable".into(), file: "portable.zip".into(), size: 0, sha256: "b".into() },
        ];
        assert!(preferred_artifact(&arts, true).is_some());
        assert_eq!(preferred_artifact(&arts, true).unwrap().kind, "portable");
    }

    #[test]
    fn preferred_artifact_installed_both_present() {
        let arts = vec![
            UpdateArtifact { kind: "nsis".into(), file: "setup.exe".into(), size: 0, sha256: "a".into() },
            UpdateArtifact { kind: "portable".into(), file: "portable.zip".into(), size: 0, sha256: "b".into() },
        ];
        assert!(preferred_artifact(&arts, false).is_some());
        assert_eq!(preferred_artifact(&arts, false).unwrap().kind, "nsis");
    }

    #[test]
    fn preferred_artifact_portable_only_nsis() {
        let arts = vec![
            UpdateArtifact { kind: "nsis".into(), file: "setup.exe".into(), size: 0, sha256: "a".into() },
        ];
        assert!(preferred_artifact(&arts, true).is_none());
    }

    #[test]
    fn preferred_artifact_installed_only_portable() {
        let arts = vec![
            UpdateArtifact { kind: "portable".into(), file: "portable.zip".into(), size: 0, sha256: "b".into() },
        ];
        assert!(preferred_artifact(&arts, false).is_none());
    }

    #[test]
    fn preferred_artifact_empty_list() {
        let arts: Vec<UpdateArtifact> = vec![];
        assert!(preferred_artifact(&arts, true).is_none());
        assert!(preferred_artifact(&arts, false).is_none());
    }

    // ── version comparison ──

    #[test]
    fn newer_major_minor_patch() {
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
    }

    #[test]
    fn missing_components_treated_as_zero() {
        assert!(is_newer("2.0", "1.9.9"));
        assert!(!is_newer("1.0", "1.0.0"));
        assert!(is_newer("1.0.1", "1.0"));
    }

    #[test]
    fn garbage_versions_never_panic() {
        assert!(!is_newer("abc", "1.0.0"));
        assert!(!is_newer("", "1.0.0"));
        assert!(!is_newer("1.0.0", ""));
        assert!(!is_newer("", ""));
        assert!(std::panic::catch_unwind(|| is_newer("1.0.0-beta", "1.0.0")).is_ok());
    }

    #[test]
    fn prerelease_is_older_than_the_matching_final() {
        // The 1.0.0 final must be offered to a user sitting on 1.0.0-beta.
        assert!(is_newer("1.0.0", "1.0.0-beta"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        // A pre-release of a NEWER core is still newer.
        assert!(is_newer("2.0.0-beta", "1.0.0"));
        assert!(is_newer("1.0.1-beta", "1.0.0"));
        // Two pre-releases of the same core: no strict ordering claimed.
        assert!(!is_newer("1.0.0-alpha", "1.0.0-beta"));
        assert!(!is_newer("1.0.0-beta", "1.0.0-beta"));
        // A pre-release never beats a higher final.
        assert!(!is_newer("1.0.0-beta", "1.0.1"));
    }

    // ── path segments ──

    #[test]
    fn safe_segment_rejects_dangerous_inputs() {
        assert!(safe_segment("LuaVault_1.1.0_x64-setup.exe").is_some());
        assert!(safe_segment("1.1.0").is_some());
        assert!(safe_segment("").is_none());
        assert!(safe_segment(".").is_none());
        assert!(safe_segment("..").is_none());
        assert!(safe_segment(".hidden").is_none());
        assert!(safe_segment("a/b").is_none());
        assert!(safe_segment("a\\b").is_none());
        assert!(safe_segment("C:evil").is_none());
        assert!(safe_segment("file name").is_none());
        assert!(safe_segment(&"x".repeat(256)).is_none());
    }

    #[tokio::test]
    async fn download_rejects_traversal_in_url_segments() {
        // safe_segment guards both URL segments BEFORE any network call —
        // a dead port proves no request was ever attempted.
        let http = UpdateClient::wrap(reqwest::Client::new());
        let bad_file = download_and_verify_from(
            &http,
            "http://127.0.0.1:1",
            "1.0.0",
            "..",
            "00",
            1,
        )
        .await;
        assert!(bad_file.is_err());
        let bad_version = download_and_verify_from(
            &http,
            "http://127.0.0.1:1",
            "..\\..\\Windows",
            "setup.exe",
            "00",
            1,
        )
        .await;
        assert!(bad_version.is_err());
    }

    #[tokio::test]
    async fn download_refuses_artefact_above_hard_cap() {
        // Refused before any network call: MAX_ARTIFACT_SIZE + 1 is a manifest
        // the client must never honour, even signed.
        let http = UpdateClient::wrap(reqwest::Client::new());
        let result = download_and_verify_from(
            &http,
            "http://127.0.0.1:1",
            "1.0.0",
            "setup.exe",
            "00",
            MAX_ARTIFACT_SIZE + 1,
        )
        .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("plafond"), "unexpected error: {err}");
    }

    // ── signature verification ──

    fn test_keypair(seed: u8) -> (SigningKey, String) {
        let sk = SigningKey::from_bytes(&[seed; 32]);
        let pk_hex = hmac::bytes_to_hex(sk.verifying_key().as_bytes());
        (sk, pk_hex)
    }

    fn sign_b64(sk: &SigningKey, msg: &[u8]) -> String {
        use base64::Engine;
        let sig = sk.sign(msg);
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
    }

    #[test]
    fn verify_manifest_accepts_primary_key() {
        let (sk0, pk0) = test_keypair(0x11);
        let (_sk1, pk1) = test_keypair(0x22);
        let msg = b"manifest bytes";
        let sig = sign_b64(&sk0, msg);
        assert_eq!(verify_manifest_with_keys(msg, &sig, &[&pk0, &pk1]), Some(0));
    }

    #[test]
    fn verify_manifest_accepts_fallback_key() {
        // The fallback key is not a disaster procedure: a manifest signed by it
        // verifies from day one, and reports slot 1.
        let (_sk0, pk0) = test_keypair(0x11);
        let (sk1, pk1) = test_keypair(0x22);
        let msg = b"manifest bytes";
        let sig = sign_b64(&sk1, msg);
        assert_eq!(verify_manifest_with_keys(msg, &sig, &[&pk0, &pk1]), Some(1));
    }

    #[test]
    fn verify_manifest_rejects_signature_by_unknown_key() {
        let (_sk0, pk0) = test_keypair(0x11);
        let (_sk1, pk1) = test_keypair(0x22);
        let (attacker, _pk) = test_keypair(0x33);
        let msg = b"manifest bytes";
        let sig = sign_b64(&attacker, msg);
        assert_eq!(verify_manifest_with_keys(msg, &sig, &[&pk0, &pk1]), None);
    }

    #[test]
    fn verify_manifest_rejects_garbage_base64() {
        assert!(
            verify_manifest_with_keys(b"data", "not-valid-base64!!!", &RELEASE_PUBLIC_KEYS)
                .is_none()
        );
    }

    #[test]
    fn verify_manifest_compiled_keys_are_well_formed() {
        for key_hex in &RELEASE_PUBLIC_KEYS {
            let bytes = hmac::hex_to_bytes(key_hex).unwrap();
            assert_eq!(bytes.len(), 32);
            let array: [u8; 32] = bytes.try_into().unwrap();
            assert!(VerifyingKey::from_bytes(&array).is_ok());
        }
    }

    #[test]
    fn unreadable_slot_falls_through_to_the_next_key() {
        // D4: slot 0 holds garbage — undecodable hex, then wrong length. The
        // fallback key in slot 1 must still be tried and still verify. With `?`
        // in the loop this returns None and the rotation rescue is dead.
        let (sk1, pk1) = test_keypair(0x22);
        let msg = b"manifest bytes";
        let sig = sign_b64(&sk1, msg);

        let bad_hex = "zzzz-not-hex";
        assert_eq!(
            verify_manifest_with_keys(msg, &sig, &[bad_hex, &pk1]),
            Some(1),
            "undecodable slot 0 must not stop the loop"
        );

        let short_key = "abcd"; // valid hex, wrong length for Ed25519
        assert_eq!(
            verify_manifest_with_keys(msg, &sig, &[short_key, &pk1]),
            Some(1),
            "wrong-length slot 0 must not stop the loop"
        );

        // Third branch: 32 bytes of the right length that are not a point on the
        // curve. Without this case the `VerifyingKey::from_bytes` guard could go
        // back to `?` and every test would stay green — the two cases above never
        // reach it. All-ones is rejected by Ed25519 decompression.
        let not_on_curve = "02".repeat(32);
        assert!(
            VerifyingKey::from_bytes(&[0x02; 32]).is_err(),
            "fixture must actually be an invalid curve point"
        );
        assert_eq!(
            verify_manifest_with_keys(msg, &sig, &[&not_on_curve, &pk1]),
            Some(1),
            "invalid curve point in slot 0 must not stop the loop"
        );
    }

    // ── evaluate_manifest (pure) ──

    fn manifest(version: &str, min: Option<&str>) -> Manifest {
        Manifest {
            schema: SCHEMA,
            version: version.to_string(),
            published_at: "2026-08-02T10:00:00Z".to_string(),
            minimum_upgradable_from: min.map(|s| s.to_string()),
            notes: Some("notes".to_string()),
            notes_i18n: None,
            history: vec![],
            artifacts: vec![UpdateArtifact {
                kind: "nsis".to_string(),
                file: "setup.exe".to_string(),
                size: 100,
                sha256: "aa".to_string(),
            }],
        }
    }

    #[test]
    fn evaluate_rejects_older_or_equal_version() {
        assert!(evaluate_manifest(manifest("1.0.0", None), "1.0.0", false).is_none());
        assert!(evaluate_manifest(manifest("0.9.0", None), "1.0.0", false).is_none());
        assert!(evaluate_manifest(manifest("1.1.0", None), "1.0.0", false).is_some());
    }

    #[test]
    fn evaluate_blocks_direct_install_below_pivot() {
        // Local 1.0.0, pivot 1.5.0: the update is announced but installation
        // is blocked — the user must reach the pivot first.
        let up = evaluate_manifest(manifest("2.0.0", Some("1.5.0")), "1.0.0", false).unwrap();
        assert!(up.upgrade_blocked);
        assert_eq!(up.minimum_upgradable_from.as_deref(), Some("1.5.0"));
        assert_eq!(up.version, "2.0.0");
    }

    #[test]
    fn evaluate_allows_install_at_or_above_pivot() {
        let up = evaluate_manifest(manifest("2.0.0", Some("1.5.0")), "1.5.0", false).unwrap();
        assert!(!up.upgrade_blocked);
        let up = evaluate_manifest(manifest("2.0.0", Some("1.5.0")), "1.7.3", false).unwrap();
        assert!(!up.upgrade_blocked);
        // No pivot declared: never blocked.
        let up = evaluate_manifest(manifest("2.0.0", None), "0.1.0", false).unwrap();
        assert!(!up.upgrade_blocked);
        assert!(up.minimum_upgradable_from.is_none());
    }

    #[test]
    fn evaluate_filters_unknown_artifact_kinds() {
        let mut m = manifest("2.0.0", None);
        m.artifacts.push(UpdateArtifact {
            kind: "flatpak".to_string(),
            file: "app.flatpak".to_string(),
            size: 100,
            sha256: "bb".to_string(),
        });
        let up = evaluate_manifest(m, "1.0.0", false).unwrap();
        assert_eq!(up.artifacts.len(), 1);
        assert_eq!(up.artifacts[0].kind, "nsis");
    }

    #[test]
    fn evaluate_none_when_only_unknown_kinds() {
        let mut m = manifest("2.0.0", None);
        m.artifacts[0].kind = "flatpak".to_string();
        assert!(evaluate_manifest(m, "1.0.0", false).is_none());
    }

    #[test]
    fn evaluate_manifest_portable_returns_portable_artifact() {
        // Fixture with BOTH nsis and portable artifacts.
        let mut m = manifest("2.0.0", None);
        m.artifacts.push(UpdateArtifact {
            kind: "portable".to_string(),
            file: "portable.zip".to_string(),
            size: 200,
            sha256: "cc".to_string(),
        });
        // portable=true → must return the portable variant.
        let up = evaluate_manifest(m, "1.0.0", true).unwrap();
        assert_eq!(up.artifacts[0].kind, "portable");
        // portable=false → must return nsis.
        let up = evaluate_manifest(
            Manifest {
                schema: SCHEMA,
                version: "2.0.0".to_string(),
                published_at: "2026-08-02T10:00:00Z".to_string(),
                minimum_upgradable_from: None,
                notes: Some("notes".to_string()),
                notes_i18n: None,
                history: vec![],
                artifacts: vec![
                    UpdateArtifact {
                        kind: "nsis".to_string(),
                        file: "setup.exe".to_string(),
                        size: 100,
                        sha256: "aa".to_string(),
                    },
                    UpdateArtifact {
                        kind: "portable".to_string(),
                        file: "portable.zip".to_string(),
                        size: 200,
                        sha256: "cc".to_string(),
                    },
                ],
            },
            "1.0.0",
            false,
        )
        .unwrap();
        assert_eq!(up.artifacts[0].kind, "nsis");
    }

    // ── local test server ──

    /// Spawn a raw HTTP server on 127.0.0.1:0. Routes map a path to the EXACT
    /// bytes written back (status line + headers + body). Returns the base URL.
    async fn spawn_raw_server(routes: HashMap<String, Vec<u8>>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(x) => x,
                    Err(_) => break,
                };
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        data.extend_from_slice(&buf[..n]);
                        if data.windows(4).any(|w| w == b"\r\n\r\n") || data.len() > 65536 {
                            break;
                        }
                    }
                    let head = String::from_utf8_lossy(&data);
                    let path = head
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .to_string();
                    let fallback = b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found".to_vec();
                    let response = routes.get(&path).unwrap_or(&fallback);
                    let _ = sock.write_all(response).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    fn http_ok(body: &[u8]) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes()
        .into_iter()
        .chain(body.iter().copied())
        .collect()
    }

    /// Signed manifest.json + manifest.json.sig routes for the test server.
    fn manifest_routes(manifest_json: &str, sk: &SigningKey) -> HashMap<String, Vec<u8>> {
        let sig = sign_b64(sk, manifest_json.as_bytes());
        let mut routes = HashMap::new();
        routes.insert("/manifest.json".to_string(), http_ok(manifest_json.as_bytes()));
        routes.insert("/manifest.json.sig".to_string(), http_ok(sig.as_bytes()));
        routes
    }

    fn signed_manifest_json(version: &str) -> String {
        format!(
            r#"{{"schema":1,"version":"{version}","published_at":"2026-08-02T10:00:00Z","notes":"notes","artifacts":[{{"kind":"nsis","file":"setup.exe","size":11,"sha256":"aa"}}]}}"#
        )
    }

    // ── fetch_verified_manifest (real network) ──

    #[tokio::test]
    async fn fetch_manifest_happy_path() {
        let (sk, pk) = test_keypair(0x41);
        let base = spawn_raw_server(manifest_routes(&signed_manifest_json("9.9.9"), &sk)).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        let manifest = fetch_verified_manifest_with_keys(&http, &base, &[&pk]).await;
        let manifest = manifest.expect("a correctly signed manifest must be accepted");
        assert_eq!(manifest.version, "9.9.9");
    }

    #[tokio::test]
    async fn fetch_manifest_rejects_invalid_signature() {
        // Manifest signed by a key NOT in the client's list: silent None.
        let (attacker, _pk) = test_keypair(0x42);
        let (_sk, pk) = test_keypair(0x43);
        let base =
            spawn_raw_server(manifest_routes(&signed_manifest_json("9.9.9"), &attacker)).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        assert!(fetch_verified_manifest_with_keys(&http, &base, &[&pk])
            .await
            .is_none());
    }

    #[tokio::test]
    async fn fetch_manifest_accepts_fallback_key() {
        let (_sk0, pk0) = test_keypair(0x44);
        let (sk1, pk1) = test_keypair(0x45);
        let base = spawn_raw_server(manifest_routes(&signed_manifest_json("9.9.9"), &sk1)).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        let manifest = fetch_verified_manifest_with_keys(&http, &base, &[&pk0, &pk1])
            .await
            .expect("fallback-signed manifest must be accepted");
        assert_eq!(manifest.version, "9.9.9");
    }

    #[tokio::test]
    async fn fetch_manifest_schema_above_is_silent_none() {
        let (sk, pk) = test_keypair(0x46);
        let json = r#"{"schema":2,"version":"99.0.0","published_at":"2026-08-02T10:00:00Z","artifacts":[{"kind":"nsis","file":"setup.exe","size":1,"sha256":"aa"}]}"#;
        let base = spawn_raw_server(manifest_routes(json, &sk)).await;
        let http = UpdateClient::wrap(reqwest::Client::new());
        // Validly signed, schema 2 > SCHEMA: the client drops it silently.
        assert!(fetch_verified_manifest_with_keys(&http, &base, &[&pk])
            .await
            .is_none());
    }

    #[tokio::test]
    async fn fetch_manifest_unreachable_server_is_none() {
        // Bind then drop: nothing listens on that port anymore.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let http = UpdateClient::wrap(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .unwrap(),
        );
        let base = format!("http://127.0.0.1:{port}");
        assert!(fetch_verified_manifest_with_keys(&http, &base, &["00"; 1])
            .await
            .is_none());
    }

    #[tokio::test]
    async fn fetch_manifest_oversized_announcement_is_refused() {
        // The manifest body is valid and correctly signed, but at 300 KB it
        // exceeds MAX_MANIFEST_SIZE (256 KB): read_capped must refuse it on the
        // announced size — a swapped-in `resp.bytes()` would accept it.
        let (sk, pk) = test_keypair(0x47);
        let mut body = signed_manifest_json("9.9.9").into_bytes();
        body.resize(300 * 1024, b' '); // JSON tolerates trailing whitespace
        let sig = sign_b64(&sk, &body);
        let mut routes = HashMap::new();
        routes.insert("/manifest.json".to_string(), http_ok(&body));
        routes.insert("/manifest.json.sig".to_string(), http_ok(sig.as_bytes()));
        let base = spawn_raw_server(routes).await;
        let http = UpdateClient::wrap(
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap(),
        );
        assert!(fetch_verified_manifest_with_keys(&http, &base, &[&pk])
            .await
            .is_none());
    }

    // ── download_and_verify (real network) ──

    const PAYLOAD: &[u8] = b"hello world"; // 11 bytes

    fn payload_sha() -> String {
        hmac::bytes_to_hex(&Sha256::digest(PAYLOAD))
    }

    /// Unique-ish file name so parallel tests don't share a temp file (pitfall #18).
    fn unique_file(tag: &str) -> String {
        format!("test-{}-{}.bin", tag, std::process::id())
    }

    #[tokio::test]
    async fn download_good_hash_writes_the_file() {
        let tag = unique_file("good");
        let mut routes = HashMap::new();
        routes.insert(format!("/download/1.2.3/{tag}"), http_ok(PAYLOAD));
        let base = spawn_raw_server(routes).await;
        let http = UpdateClient::wrap(reqwest::Client::new());

        let path = download_and_verify_from(
            &http, &base, "1.2.3", &tag, &payload_sha(),
            PAYLOAD.len() as u64,
        )
        .await
        .expect("a matching artefact must be written");

        let on_disk = tokio::fs::read(&path).await.unwrap();
        assert_eq!(on_disk, PAYLOAD);
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn download_bad_hash_refuses_and_writes_nothing() {
        let tag = unique_file("badhash");
        let mut routes = HashMap::new();
        routes.insert(format!("/download/1.2.3/{tag}"), http_ok(PAYLOAD));
        let base = spawn_raw_server(routes).await;
        let http = UpdateClient::wrap(reqwest::Client::new());

        let wrong_sha = "0".repeat(64);
        let result = download_and_verify_from(
            &http, &base, "1.2.3", &tag, &wrong_sha,
            PAYLOAD.len() as u64,
        )
        .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("SHA-256"), "unexpected error: {err}");

        let target = std::env::temp_dir()
            .join("luavault-updates")
            .join(&tag);
        assert!(!target.exists(), "a mismatched artefact must never reach disk");
    }

    #[tokio::test]
    async fn download_wrong_size_refuses_and_writes_nothing() {
        let tag = unique_file("badsize");
        let mut routes = HashMap::new();
        routes.insert(format!("/download/1.2.3/{tag}"), http_ok(PAYLOAD));
        let base = spawn_raw_server(routes).await;
        let http = UpdateClient::wrap(reqwest::Client::new());

        // Correct hash of the payload but a lying size: rejected up front on the
        // announced Content-Length (a removed announcement check would degrade
        // to the final "taille incorrecte" — the message discriminates).
        let result = download_and_verify_from(
            &http, &base, "1.2.3", &tag, &payload_sha(),
            PAYLOAD.len() as u64 + 5,
        )
        .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("taille annoncée incorrecte"), "unexpected error: {err}");

        let target = std::env::temp_dir()
            .join("luavault-updates")
            .join(&tag);
        assert!(!target.exists(), "a mismatched artefact must never reach disk");
    }

    #[tokio::test]
    async fn download_streaming_stops_beyond_expected_size() {
        // No Content-Length (chunked): the server streams ~100 bytes while the
        // manifest promised 10. The loop must cut off mid-stream, never
        // materialize the whole body, and never write the file.
        let tag = unique_file("stream");
        let mut routes = HashMap::new();
        let mut chunked =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        for _ in 0..2 {
            chunked.extend_from_slice(b"32\r\n"); // hex 50
            chunked.extend_from_slice(&[b'X'; 50]);
            chunked.extend_from_slice(b"\r\n");
        }
        chunked.extend_from_slice(b"0\r\n\r\n");
        routes.insert(format!("/download/1.2.3/{tag}"), chunked);
        let base = spawn_raw_server(routes).await;
        let http = UpdateClient::wrap(reqwest::Client::new());

        let result = download_and_verify_from(
            &http, &base, "1.2.3", &tag, &payload_sha(), 10,
        )
        .await;
        let err = result.unwrap_err().to_string();
        // The streaming cap message — a removed cap would degrade to the final
        // "taille incorrecte" check AFTER materializing the whole body.
        assert!(err.contains("plus grande que la taille attendue"), "unexpected error: {err}");

        let target = std::env::temp_dir()
            .join("luavault-updates")
            .join(&tag);
        assert!(!target.exists());
    }

    // ── install path validation (D1 + D2) ──

    /// Scratch file OUTSIDE the download directory, with a `..`-laden path that
    /// canonicalizes back to it.
    async fn make_outside_file(tag: &str, content: &[u8]) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("lv-fix01-{tag}-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("payload.bin");
        tokio::fs::write(&path, content).await.unwrap();
        let sha = hmac::bytes_to_hex(&Sha256::digest(content));
        (path, sha)
    }

    #[tokio::test]
    async fn install_path_rejects_dotdot_traversal() {
        // The download directory exists and the traversal target's hash matches
        // what is "verified" — so the ONLY guard that can still catch this is
        // the canonicalized starts_with check.
        let updates = std::env::temp_dir().join("luavault-updates");
        tokio::fs::create_dir_all(&updates).await.unwrap();

        let tag = format!("d1-{}", std::process::id());
        let (outside, sha) = make_outside_file(&tag, b"malicious payload").await;

        // %TEMP%\luavault-updates\..\lv-fix01-<tag>\payload.bin
        let traversal = updates.join("..").join(format!("lv-fix01-{tag}-{}", std::process::id()))
            .join("payload.bin");
        assert!(traversal.exists(), "fixture must resolve to the outside file");

        let verified = Some((traversal.to_string_lossy().to_string(), sha));
        let err = validate_install_path(&traversal.to_string_lossy(), &verified).unwrap_err();
        assert!(err.contains("chemin refusé"), "unexpected error: {err}");

        let _ = tokio::fs::remove_dir_all(outside.parent().unwrap()).await;
    }

    #[tokio::test]
    async fn install_path_requires_a_verified_pair() {
        // No download recorded: even a file sitting in the right directory is
        // refused — the path is not the proof, the recorded pair is.
        let updates = std::env::temp_dir().join("luavault-updates");
        tokio::fs::create_dir_all(&updates).await.unwrap();
        let file = updates.join(format!("unverified-{}.bin", std::process::id()));
        tokio::fs::write(&file, b"content").await.unwrap();

        let err = validate_install_path(&file.to_string_lossy(), &None).unwrap_err();
        assert!(err.contains("aucun téléchargement vérifié"), "unexpected: {err}");

        let _ = tokio::fs::remove_file(&file).await;
    }

    #[tokio::test]
    async fn install_path_rejects_file_rewritten_after_download() {
        // The recorded path is legit and inside the directory, but the file was
        // swapped between download and click: the re-hashed digest must refuse.
        let updates = std::env::temp_dir().join("luavault-updates");
        tokio::fs::create_dir_all(&updates).await.unwrap();
        let file = updates.join(format!("tampered-{}.bin", std::process::id()));
        tokio::fs::write(&file, b"original verified content").await.unwrap();

        let stale_sha = "0".repeat(64);
        let verified = Some((file.to_string_lossy().to_string(), stale_sha));
        let err = validate_install_path(&file.to_string_lossy(), &verified).unwrap_err();
        assert!(err.contains("SHA-256"), "unexpected error: {err}");

        let _ = tokio::fs::remove_file(&file).await;
    }

    #[tokio::test]
    async fn install_path_accepts_the_verified_file() {
        let updates = std::env::temp_dir().join("luavault-updates");
        tokio::fs::create_dir_all(&updates).await.unwrap();
        let file = updates.join(format!("legit-{}.bin", std::process::id()));
        let content = b"the exact verified bytes";
        tokio::fs::write(&file, content).await.unwrap();
        let sha = hmac::bytes_to_hex(&Sha256::digest(content));

        let verified = Some((file.to_string_lossy().to_string(), sha));
        let canon = validate_install_path(&file.to_string_lossy(), &verified)
            .expect("the verified file must open");
        assert_eq!(canon, std::fs::canonicalize(&file).unwrap());

        let _ = tokio::fs::remove_file(&file).await;
    }

    // ── dedicated http client (D8) ──

    #[tokio::test]
    async fn update_client_sends_no_lua_vault_headers_and_no_accept_encoding() {
        // A bespoke echo server: it answers every request with the raw request
        // head as the body, so the test can assert on the headers the client
        // actually put on the wire.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(x) => x,
                    Err(_) => break,
                };
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        data.extend_from_slice(&buf[..n]);
                        if data.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    );
                    let _ = sock.write_all(response.as_bytes()).await;
                    let _ = sock.write_all(&data).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        let url = format!("http://127.0.0.1:{port}/echo");

        let update_http = build_http_client();
        let head = update_http
            .get(&url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
            .to_ascii_lowercase();
        assert!(!head.contains("LuaVault"), "LuaVault headers leaked: {head}");
        assert!(!head.contains("origin:"), "an Origin header leaked: {head}");
        assert!(!head.contains("referer"), "a Referer header leaked: {head}");
        assert!(
            !head.contains("accept-encoding"),
            "the update client must not negotiate compression: {head}"
        );

    }

    /// End-to-end against the real update server, with the RELEASE keys compiled
    /// into this binary — the one check no local fixture can stand in for: a
    /// manifest signed with the wrong key, or bytes altered anywhere between the
    /// signing machine and the wire, fails here and nowhere else.
    ///
    /// ```text
    /// cargo test live_update_manifest -- --ignored --nocapture
    /// ```
    /// Point `LV_UPDATE_BASE` at an SSH forward when the hostname is not yet
    /// routed: `ssh -L 8090:127.0.0.1:8090 vps`.
    #[tokio::test]
    #[ignore]
    async fn live_update_manifest_is_signed_by_a_release_key() {
        let http = UpdateClient::new();
        let base = base_url();
        println!("base = {base}");
        let manifest = fetch_verified_manifest(&http, &base)
            .await
            .expect("le manifeste publié doit être accepté par une clé de publication");
        println!("version = {}", manifest.version);
        println!("artefacts = {}", manifest.artifacts.len());
        for a in &manifest.artifacts {
            println!("  {} {} {} {}", a.kind, a.file, a.size, a.sha256);
        }
        assert_eq!(manifest.schema, SCHEMA);
        assert!(!manifest.artifacts.is_empty());
    }

    // ── HistoryEntry / history deserialization ──

    #[test]
    fn manifest_without_history_field_deserializes() {
        // The 1.0.0 manifest already published has no `history` field.
        // serde(default) must make it an empty Vec, never an error.
        let json = r#"{"schema":1,"version":"1.0.0","published_at":"2026-08-02T10:00:00Z","notes":"hello","artifacts":[{"kind":"nsis","file":"setup.exe","size":100,"sha256":"aa"}]}"#;
        let m: Manifest = serde_json::from_str(json).expect("un manifeste sans historique se lit encore");
        assert!(m.history.is_empty());
    }

    #[test]
    fn manifest_with_history_field_deserializes() {
        let json = r#"{"schema":1,"version":"2.0.0","published_at":"2026-08-03T10:00:00Z","notes":"hello","history":[{"version":"1.1.0","published_at":"2026-07-01T10:00:00Z","notes":"fixes"},{"version":"1.0.5","notes":"security"}],"artifacts":[{"kind":"nsis","file":"setup.exe","size":100,"sha256":"aa"}]}"#;
        let m: Manifest = serde_json::from_str(json).expect("un manifeste avec historique se lit");
        assert_eq!(m.history.len(), 2);
        assert_eq!(m.history[0].version, "1.1.0");
        assert_eq!(m.history[1].version, "1.0.5");
        assert_eq!(m.history[1].published_at, None);
    }

    // ── changes_since ──

    fn history_entry(version: &str) -> HistoryEntry {
        HistoryEntry {
            version: version.to_string(),
            published_at: Some("2026-08-02T10:00:00Z".to_string()),
            notes: Some("notes".to_string()),
            notes_i18n: None,
        }
    }

    #[test]
    fn changes_since_empty_history_returns_empty() {
        let empty: Vec<HistoryEntry> = vec![];
        let result = changes_since(&empty, "1.0.0");
        assert!(result.is_empty());
    }

    #[test]
    fn changes_since_1_9_0_vs_1_10_0() {
        // The string comparison trap: "1.10.0" < "1.9.0" lexicographically.
        // A user on 1.9.0 must see 1.10.0 in their changelog.
        let h = vec![
            history_entry("1.10.0"),
            history_entry("1.9.0"),
        ];
        let result = changes_since(&h, "1.9.0");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "1.10.0");
    }

    #[test]
    fn changes_since_excludes_equal_version() {
        let h = vec![
            history_entry("2.0.0"),
            history_entry("1.0.0"),
        ];
        let result = changes_since(&h, "1.0.0");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "2.0.0");
    }

    #[test]
    fn changes_since_excludes_older_versions() {
        let h = vec![
            history_entry("2.0.0"),
            history_entry("0.5.0"),
        ];
        let result = changes_since(&h, "1.0.0");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "2.0.0");
    }

    #[test]
    fn changes_since_returns_descending_order() {
        let h = vec![
            history_entry("1.0.0"),
            history_entry("1.2.0"),
            history_entry("1.1.0"),
            history_entry("2.0.0"),
        ];
        let result = changes_since(&h, "0.9.0");
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].version, "2.0.0");
        assert_eq!(result[1].version, "1.2.0");
        assert_eq!(result[2].version, "1.1.0");
        assert_eq!(result[3].version, "1.0.0");
    }

    #[test]
    fn changes_since_all_missed_versions() {
        let h = vec![
            history_entry("1.3.0"),
            history_entry("1.2.0"),
            history_entry("1.1.0"),
        ];
        let result = changes_since(&h, "1.0.0");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].version, "1.3.0");
        assert_eq!(result[1].version, "1.2.0");
        assert_eq!(result[2].version, "1.1.0");
    }

    // ── evaluate_manifest populates changes ──

    #[test]
    fn evaluate_manifest_populates_changes() {
        let mut m = manifest("2.0.0", None);
        m.history = vec![
            history_entry("1.1.0"),
            history_entry("1.0.0"),
        ];
        let up = evaluate_manifest(m, "0.9.0", false).unwrap();
        assert_eq!(up.changes.len(), 2);
        assert_eq!(up.changes[0].version, "1.1.0");
        assert_eq!(up.changes[1].version, "1.0.0");
    }

    #[test]
    fn evaluate_manifest_no_history_yields_empty_changes() {
        let m = manifest("2.0.0", None);
        let up = evaluate_manifest(m, "1.0.0", false).unwrap();
        assert!(up.changes.is_empty());
    }

    #[test]
    fn test_notes_deserialization() {
        // Chaîne simple
        let json_str = r#"{
            "version": "1.0.0",
            "notes": "Simple string"
        }"#;
        let entry: HistoryEntry = serde_json::from_str(json_str).unwrap();
        assert_eq!(entry.notes.unwrap(), "Simple string");
        assert!(entry.notes_i18n.is_none());

        // Dictionnaire
        let json_dict = r#"{
            "version": "2.0.0",
            "notes": "Fallback string",
            "notes_i18n": {
                "en": "English notes",
                "fr": "Notes en français"
            }
        }"#;
        let entry_dict: HistoryEntry = serde_json::from_str(json_dict).unwrap();
        assert!(entry_dict.notes_i18n.is_some());
        assert_eq!(entry_dict.notes_i18n.unwrap().get("fr").unwrap(), "Notes en français");
    }
}

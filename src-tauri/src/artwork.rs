//! Local artwork cache (LOT-14) — CDN images downloaded once, kept on disk,
//! served to the webview through the `asset:` protocol.
//!
//! Invariants (the lot's contract):
//! - The file name is the SHA-256 of the URL; the extension is derived from
//!   the response's Content-Type, NEVER from the URL text — a URL comes from
//!   the API, deriving a path from it signs for a traversal (pitfalls 22, 23
//!   and 26; n°26 shipped an arbitrary-execution hole in `install_update`).
//! - The final path is checked UNDER the cache directory by canonicalizing
//!   BOTH sides; a failed canonicalization is fail-closed (pitfall 26).
//! - The HTTP client is a newtype (pitfall 28): `state.http` carries
//!   LuaVault's spoofed `Origin`/`Referer`/UA and must never reach a CDN.
//! - Nothing is read without a cap (pitfall 29): the announced
//!   `Content-Length` is refused first, then the stream is cut mid-way. No
//!   decompression either — a compressed body must not amplify past the cap.
//! - A failure is never cached (CLAUDE.md, Caching): the containment gate
//!   runs BEFORE anything is written, then the image goes to a temp file and
//!   is renamed only once complete — a refusal or a cut connection leaves
//!   nothing a later lookup could serve as valid.
//! - Only https leaves for a CDN, and redirects are never followed
//!   (Policy::none): a hop must not walk the scheme gate around into http://
//!   or a private address (blind SSRF). A 3xx comes back and is refused.
//! - The purge evicts least-recently-used — a hit refreshes the file's mtime
//!   (`hit`) — never the file that was just written, and never counts below
//!   zero. Orphan `.part-*` from dead PIDs are swept at its head.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{config, hmac, i18n_log};

/// Per-image cap. A store header weighs a few hundred KB, an icon a few tens:
/// 8 MiB leaves headroom for retina artwork while anything above it is not a
/// box image. Distinct from the 256 MiB cache cap — one bounds a single
/// response, the other the whole folder.
pub const MAX_IMAGE_SIZE: u64 = 8 * 1024 * 1024;

/// Cap for the whole cache, with least-recently-used eviction past it.
/// 256 MiB ≈ 500 store headers at ~480 KB: well above a hundred-game library
/// (icons + headers + a few screenshots land near 100 MiB), small enough that
/// "clear the cache" in the settings stays an honest answer to disk pressure.
pub const MAX_CACHE_SIZE: u64 = 256 * 1024 * 1024;

/// Suffix of in-flight temp writes — a crash can orphan them, the purge
/// ignores them (they may be live), `clear` sweeps them.
const TMP_SUFFIX: &str = ".part";

// -------------------------------------------------------------------- client

/// Dedicated client for the artwork CDNs, in a newtype so the compiler — not
/// a test, not a reviewer's attention — keeps the LuaVault client out of here.
/// Both are `reqwest::Client`; `state.http` and an artwork client were
/// interchangeable at every call site before the newtype (pitfall 28). No
/// decompression: a compressed body must not be able to amplify past
/// [`MAX_IMAGE_SIZE`] (pitfall 29).
#[derive(Clone)]
pub struct ArtworkClient(reqwest::Client);

impl ArtworkClient {
    pub fn new() -> Self {
        Self(
            reqwest::Client::builder()
                .use_rustls_tls()
                .no_gzip()
                .no_brotli()
                .no_zstd()
                // Redirects are NEVER followed. The https gate checks the
                // initial URL only; a followed hop would walk straight past
                // it into http:// or a private address (blind SSRF via an
                // API-controlled image URL). Steam's CDNs do not redirect —
                // a 3xx comes back to store(), which refuses non-2xx.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build artwork http client"),
        )
    }

    fn inner(&self) -> &reqwest::Client {
        &self.0
    }
}

impl Default for ArtworkClient {
    fn default() -> Self {
        Self::new()
    }
}

// -------------------------------------------------------------------- paths

/// The cache folder: `<data_dir>\artwork`. `data_dir` resolves to
/// `%LocalAppData%\LuaVault` in the installable edition and to the
/// exe's folder in portable mode — which is exactly why the asset protocol
/// scope is granted at runtime (`lib.rs`), never written in `tauri.conf.json`.
pub fn cache_dir() -> PathBuf {
    config::data_dir().join("artwork")
}

/// Hex SHA-256 of the URL text — the only part of a URL allowed near a file
/// name. Two distinct URLs collide only as SHA-256 collides.
pub fn url_hash(url: &str) -> String {
    hmac::bytes_to_hex(&Sha256::digest(url.as_bytes()))
}

/// Extension from the response's Content-Type — never from the URL, whose
/// path an attacker controls. Unknown types get the `bin` sentinel: sniffing
/// only works when `infer` recognises the magic bytes, and otherwise the
/// asset protocol falls back to the EXTENSION's MIME. An unknown suffix
/// there means `text/html` served from `http://asset.localhost` — a
/// navigable type where an image belongs, a trap armed for the first lot
/// that opens an iframe. `bin` maps to application/octet-stream and never
/// upgrades.
pub fn extension_for(content_type: &str) -> &'static str {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match ct.as_str() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/bmp" => "bmp",
        _ => "bin",
    }
}

/// Cache file name for a URL: `<sha256 hex>.<ext from Content-Type>`.
pub fn file_name(url: &str, content_type: &str) -> String {
    format!("{}.{}", url_hash(url), extension_for(content_type))
}

/// The name must be ONE plain component: 64 hex chars, one dot, a short
/// extension. Separators, `..`, `:` (NTFS alternate data streams) and device
/// names are all impossible here by construction — the check exists so a
/// future "simplification" of `file_name` fails loudly (pitfall 23).
fn validate_name(name: &str) -> Result<()> {
    let mut components = name.split('.');
    let stem = components.next().unwrap_or("");
    let ext = components.next().unwrap_or("");
    if components.next().is_some() || stem.len() != 64 || ext.is_empty() || ext.len() > 8 {
        bail!("nom de fichier de cache invalide : {name}");
    }
    if !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("nom de fichier de cache invalide : {name}");
    }
    if !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        bail!("nom de fichier de cache invalide : {name}");
    }
    Ok(())
}

/// Path of the cached image for a URL, when present. The extension depends
/// on the Content-Type the server sent, so the lookup matches on the hash
/// prefix — `<64 hex>.<anything>` — rather than a full name.
pub fn cached_path(dir: &Path, url: &str) -> Option<PathBuf> {
    let hex = url_hash(url);
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let raw = entry.file_name();
        let name = raw.to_string_lossy();
        // `is_cache_file` matters here: a `.part` temp from an interrupted
        // download also starts with the hash, and serving it would hand a
        // half-image to the webview.
        if name.starts_with(&hex) && is_cache_file(&name) {
            return Some(entry.path());
        }
    }
    None
}

/// Refresh a file's modification time — the purge's LRU clock. Windows
/// wants write access to set timestamps.
fn touch(path: &Path) -> std::io::Result<()> {
    std::fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(SystemTime::now())
}

/// Cache hit: the cached file for a URL, its LRU clock refreshed. This is
/// the lookup user-facing commands call — a picture looked at today must
/// outlive a screenshot downloaded yesterday and never reopened. A failed
/// touch degrades that one file to download-order eviction; it never
/// refuses an image that is on disk.
pub fn hit(dir: &Path, url: &str) -> Option<PathBuf> {
    let path = cached_path(dir, url)?;
    if let Err(e) = touch(&path) {
        log::warn!("{}", i18n_log::i18n_log(
            format!("horodatage du cache impossible ({}): {e}", path.display()),
            "logs.cache.touch-failed",
            &[("path", serde_json::json!(path.display().to_string())), ("error", serde_json::json!(e.to_string()))],
        ));
    }
    Some(path)
}

/// Canonicalize BOTH sides and require the file under the directory.
/// `Path::starts_with` alone lets `..` components through (pitfall 26);
/// canonicalizing resolves them — and a failed canonicalization fails
/// CLOSED, never open. Returns nothing: the caller keeps handing out the
/// plain path (canonicalized Windows paths carry a `\\?\` prefix the asset
/// protocol scope would not recognise).
pub fn assert_within(dir: &Path, candidate: &Path) -> Result<()> {
    let canon_dir =
        std::fs::canonicalize(dir).context("canonicalisation du dossier de cache")?;
    let canon = std::fs::canonicalize(candidate).context("canonicalisation du fichier")?;
    if !canon.starts_with(&canon_dir) {
        bail!("chemin hors du dossier de cache : {}", candidate.display());
    }
    Ok(())
}

/// The pre-write containment gate: canonicalize the directory (fail closed —
/// a missing cache dir is a refusal, not a silent creation) and normalize
/// the destination into it component by component. `validate_name` already
/// makes an escaping name impossible; this is the loud failure for the day
/// a "simplification" changes that — checked while nothing is on disk yet,
/// so a refusal can never leave a file a later lookup would serve.
fn assert_destination_within(dir: &Path, name: &str) -> Result<()> {
    let canon_dir =
        std::fs::canonicalize(dir).context("canonicalisation du dossier de cache")?;
    let mut destination = canon_dir.clone();
    for component in Path::new(name).components() {
        match component {
            std::path::Component::Normal(part) => destination.push(part),
            std::path::Component::ParentDir => {
                destination.pop();
            }
            std::path::Component::CurDir => {}
            // A prefix or root cannot appear in a relative name — fail closed.
            _ => bail!("composant de nom inattendu : {name}"),
        }
    }
    if !destination.starts_with(&canon_dir) {
        bail!("chemin hors du dossier de cache : {name}");
    }
    Ok(())
}

// -------------------------------------------------------------------- capped read

/// Accumulator for a response body under a hard size cap — the sibling of
/// `update::read_capped`, shaped so tests can feed it a FABRICATED stream
/// (no connection at all): refuse the announced size before a single byte,
/// then cut off mid-stream the instant the total exceeds the cap.
#[derive(Debug)]
pub struct CappedBody {
    limit: u64,
    body: Vec<u8>,
}

impl CappedBody {
    /// `announced` is the `Content-Length` when the response carries one.
    /// A response announcing more than the cap is rejected before any read.
    pub fn new(limit: u64, announced: Option<u64>) -> Result<Self> {
        if let Some(announced) = announced {
            if announced > limit {
                bail!(
                    "réponse annoncée à {} octets — plafond {} dépassé",
                    announced,
                    limit
                );
            }
        }
        Ok(Self {
            limit,
            body: Vec::new(),
        })
    }

    /// One chunk; the cap cuts mid-stream a response that lied about its
    /// length (or announced nothing at all).
    pub fn push(&mut self, chunk: &[u8]) -> Result<()> {
        if self.body.len() as u64 + chunk.len() as u64 > self.limit {
            bail!("réponse plus grande que le plafond {} octets", self.limit);
        }
        self.body.extend_from_slice(chunk);
        Ok(())
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.body
    }
}

// -------------------------------------------------------------------- writes

/// Write beside the final file, NOT at the final file: a cut connection (or
/// anything else that stops the process before `finalize`) leaves a `.part`
/// the cache never serves, never a half-image under the real name.
fn write_tmp(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).context("création du dossier de cache")?;
    let tmp = dir.join(format!("{name}{TMP_SUFFIX}-{}", std::process::id()));
    std::fs::write(&tmp, bytes).context("écriture du fichier temporaire")?;
    Ok(tmp)
}

/// Rename into place — atomic on NTFS, so the final name goes from "absent"
/// to "complete" with no state in between.
fn finalize(dir: &Path, tmp: PathBuf, name: &str) -> Result<PathBuf> {
    let final_path = dir.join(name);
    std::fs::rename(&tmp, &final_path).context("renommage du fichier en cache")?;
    Ok(final_path)
}

/// The decision that "a failure is never cached", kept pure so a test can
/// exercise it without a connection: non-2xx or an empty body refuses BEFORE
/// anything reaches the disk; the containment gate canonicalizes BEFORE any
/// write; a success goes temp-file → rename, then one last canonical check
/// (fail closed, with cleanup).
pub fn store(
    dir: &Path,
    url: &str,
    content_type: &str,
    status: u16,
    bytes: Vec<u8>,
) -> Result<PathBuf> {
    if !(200..300).contains(&status) {
        bail!("image refusée : HTTP {status}");
    }
    if bytes.is_empty() {
        bail!("image refusée : corps vide");
    }
    let name = file_name(url, content_type);
    validate_name(&name)?;
    // BEFORE anything is written: a refusal must leave nothing on disk that
    // a later lookup could serve.
    assert_destination_within(dir, &name)?;
    let tmp = write_tmp(dir, &name, &bytes)?;
    let final_path = finalize(dir, tmp, &name)?;
    if let Err(e) = assert_within(dir, &final_path) {
        // The folder would have to move between the two checks to get here;
        // even so, never leave a file nobody proved stays inside the cache.
        let _ = std::fs::remove_file(&final_path);
        return Err(e);
    }
    remove_hash_siblings(dir, &name);
    Ok(final_path)
}

/// A re-download whose Content-Type changed writes `<hash>.<new>` beside
/// `<hash>.<old>`: remove the siblings, or `cached_path` (a hash-prefix
/// match) would serve whichever `read_dir` returns first — possibly the
/// stale one, forever, with both counting toward the cap. A removal failure
/// only degrades to that previous behaviour: warn, never fail the store.
fn remove_hash_siblings(dir: &Path, name: &str) {
    let hex = name.split('.').next().unwrap_or("");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let raw = entry.file_name();
        let other = raw.to_string_lossy();
        if other.as_ref() != name && other.starts_with(hex) && is_cache_file(&other) {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                log::warn!("{}", i18n_log::i18n_log(
                    format!("ancien fichier de cache non remplacé ({other}): {e}"),
                    "logs.cache.stale-file-not-replaced",
                    &[("file", serde_json::json!(other.as_ref())), ("error", serde_json::json!(e.to_string()))],
                ));
            }
        }
    }
}

/// The ONLY scheme that may leave for a CDN — pure, so a test can exercise
/// the gate without opening a connection. Anything else (`file:`, `data:`,
/// a loopback or private `http:`…) would turn the API's image URL list
/// into a probe — a blind SSRF.
fn ensure_https(url: &str) -> Result<()> {
    if !url.starts_with("https://") {
        bail!("seules les URLs https sont acceptées");
    }
    Ok(())
}

/// The network phase only: scheme gate, then a capped read of the body.
/// Returns `(status, Content-Type, bytes)`. The disk phase is
/// [`store_downloaded`], kept apart so the blocking work runs off the async
/// pool (pitfall 17) and each phase can be tested without a connection.
pub async fn download(client: &ArtworkClient, url: &str) -> Result<(u16, String, Vec<u8>)> {
    // The URL comes from the frontend, which holds it from the API — see
    // ensure_https. Redirects never bypass it: the client follows none.
    ensure_https(url)?;
    let resp = client
        .inner()
        .get(url)
        .send()
        .await
        .context("téléchargement de l'image")?;
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut body = CappedBody::new(MAX_IMAGE_SIZE, resp.content_length())?;
    let mut resp = resp;
    while let Some(chunk) = resp.chunk().await.context("lecture de la réponse")? {
        body.push(&chunk)?;
    }
    Ok((status, content_type, body.into_inner()))
}

/// The disk phase of a download: containment-gated write, stale-sibling
/// cleanup, then a purge that PROTECTS the file that was just written — a
/// download must never be evicted by the very act of landing. `cap` is a
/// parameter so a test can force an eviction without writing 256 MiB.
pub fn store_downloaded(
    dir: &Path,
    url: &str,
    content_type: &str,
    status: u16,
    bytes: Vec<u8>,
    cap: u64,
) -> Result<PathBuf> {
    let name = file_name(url, content_type);
    let path = store(dir, url, content_type, status, bytes)?;
    purge(dir, cap, Some(&name))?;
    Ok(path)
}

// -------------------------------------------------------------------- housekeeping

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CacheStats {
    pub bytes: u64,
    pub file_count: usize,
}

/// True for complete cache files — a hash stem plus a dot, exactly what
/// `file_name` produces. `.part` temps and anything foreign are excluded,
/// so a crashed download never counts toward the size the settings show.
fn is_cache_file(name: &str) -> bool {
    let Some(dot) = name.find('.') else {
        return false;
    };
    let stem = &name[..dot];
    let ext = &name[dot + 1..];
    stem.len() == 64
        && stem.bytes().all(|b| b.is_ascii_hexdigit())
        && !ext.is_empty()
        && !ext.contains('.')
}

/// Walk the completed files with their size and modification time.
fn list_completed(dir: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();
        if !is_cache_file(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        out.push((entry.path(), meta.len(), mtime));
    }
    out
}

/// Size and file count of the cache — what the settings screen shows.
pub fn stats(dir: &Path) -> CacheStats {
    let files = list_completed(dir);
    CacheStats {
        bytes: files.iter().map(|(_, len, _)| len).sum(),
        file_count: files.len(),
    }
}

/// `.part-<pid>` temps whose PID is not ours are orphans by construction:
/// our own in-flight downloads carry our PID, and another PID's file can
/// never complete. Sweep them at the head of the purge — or every crashed
/// run leaves one per interrupted URL, excluded from stats and cap, never
/// deleted. A malformed suffix is left alone: only a digit PID is ours to
/// judge.
fn sweep_orphan_parts(dir: &Path) {
    let ours = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let raw = entry.file_name();
        let name = raw.to_string_lossy();
        let Some((_, pid)) = name.rsplit_once(".part-") else {
            continue;
        };
        if pid == ours || !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Err(e) = std::fs::remove_file(entry.path()) {
            log::debug!("orphelin .part non balayé ({name}): {e}");
        }
    }
}

/// Evict least-recently-used until the total fits under `cap`.
/// `protect` (the file that was just written) is never evicted — if it alone
/// busts the cap, the purge stops in front of it rather than deleting what a
/// download just produced. Sizes are `saturating_sub`'d: the total can never
/// wrap below zero no matter what a concurrent delete already removed.
pub fn purge(dir: &Path, cap: u64, protect: Option<&str>) -> Result<u64> {
    // Before ANY accounting: temps a dead run left behind are not cache
    // files, so neither the cap test below nor `list_completed` would ever
    // touch them.
    sweep_orphan_parts(dir);
    let mut files = list_completed(dir);
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= cap {
        return Ok(0);
    }
    // Oldest first: mtime is the LRU clock — every hit refreshes it (hit()
    // touches the file), every download rewrites it.
    files.sort_by_key(|a| a.2);
    let mut freed: u64 = 0;
    let last_index = files.len().saturating_sub(1);
    for (index, (path, len, _)) in files.iter().enumerate() {
        if total <= cap {
            break;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if protect == Some(name.as_str()) {
            // The just-written file: skip it while anything else remains,
            // stop entirely once only it is left.
            if index < last_index {
                continue;
            }
            break;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {
                total = total.saturating_sub(*len);
                freed += len;
            }
            // Already gone (a concurrent clear): keep counting down.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                total = total.saturating_sub(*len);
                freed += len;
            }
            Err(e) => return Err(e).context("purge du cache d'images"),
        }
    }
    Ok(freed)
}

/// Empty the cache entirely (the settings button). Returns what was freed.
pub fn clear(dir: &Path) -> Result<CacheStats> {
    let mut freed = CacheStats {
        bytes: 0,
        file_count: 0,
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(freed); // no cache yet = nothing to clear
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            continue; // never anything but files in there — leave it be
        }
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if std::fs::remove_file(&path).is_ok() {
            freed.bytes += len;
            freed.file_count += 1;
        }
    }
    Ok(freed)
}

// -------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-test scratch directory — Rust runs tests concurrently and a shared
    /// folder would let them delete each other's files (pitfall 18).
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ast_art_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_aged(dir: &Path, name: &str, size: usize, age_secs: u64) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![b'x'; size]).unwrap();
        let when = SystemTime::now() - std::time::Duration::from_secs(age_secs);
        // Windows wants write access to set a file's timestamps.
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(when)
            .unwrap();
        path
    }

    // ── naming: a hash, never the URL ──

    #[test]
    fn file_name_is_a_hash_of_the_url() {
        let a = file_name("https://cdn.example/a.jpg", "image/jpeg");
        let b = file_name("https://cdn.example/b.jpg", "image/jpeg");
        assert_ne!(a, b, "deux URLs distinctes ne partagent pas un nom");
        // Same URL, same type: stable — the cache must hit twice.
        assert_eq!(a, file_name("https://cdn.example/a.jpg", "image/jpeg"));
        // The extension comes from the Content-Type, not the URL's ".jpg".
        let from_png = file_name("https://cdn.example/a.jpg", "image/png; charset=x");
        assert!(from_png.ends_with(".png"), "l'extension suit le Content-Type");
        // Unknown type: the sentinel, never anything URL-derived.
        assert!(file_name("https://cdn.example/a.exe", "application/octet-stream").ends_with(".bin"));
        // And whatever the URL says, the name is one harmless component.
        let hostile = file_name("https://x/../../evil.png", "image/png");
        assert!(validate_name(&hostile).is_ok());
        assert!(!hostile.contains('/'));
        assert!(!hostile.contains('\\'));
    }

    #[test]
    fn validate_name_rejects_everything_shaped() {
        // 'z' is NOT a hex digit ('a' is — a 64×'a' stem would be accepted).
        assert!(validate_name(&format!("{}.jpg", "z".repeat(64))).is_err());
        assert!(validate_name(&format!("{}.jpg", "g".repeat(64))).is_err());
        assert!(validate_name(&"a".repeat(64)).is_err()); // no extension
        assert!(validate_name(&format!("{}.tar.gz", "0".repeat(64))).is_err());
        // Device names (CON, NUL, COM1…) are stopped too — and the mechanism
        // that stops them IS the stem check: none of those names is 64 hex
        // digits, every one carries a non-hex letter. A bare device dies on
        // the length rule, one padded to 64 chars dies on the hex rule.
        for device in ["CON", "NUL", "PRN", "AUX", "COM1", "LPT1"] {
            assert!(validate_name(&format!("{device}.jpg")).is_err());
            let padded = format!("{device}{}", "0".repeat(64 - device.len()));
            assert!(validate_name(&format!("{padded}.jpg")).is_err());
        }
        assert!(validate_name(&format!("{}.jpg", "0".repeat(64))).is_ok());
    }

    // ── traversal: containment, fail closed ──

    #[test]
    fn a_hostile_url_never_escapes_the_cache_dir() {
        let dir = scratch("traversal");
        for url in [
            "https://x/../../evil.png",
            "https://x/C:evil.png",
            "https://x/..%2F..%2Fevil.png",
        ] {
            let path = store(&dir, url, "image/png", 200, b"png-bytes".to_vec())
                .expect("un hash de nom ne peut pas traverser");
            assert_within(&dir, &path).expect("le chemin final reste sous le cache");
            assert!(
                path.parent() == Some(dir.as_path()),
                "le fichier vit directement dans le cache"
            );
        }
    }

    #[test]
    fn assert_within_fails_closed() {
        let dir = scratch("within");
        let outside = dir.parent().unwrap().join(format!("outside_{}", std::process::id()));
        std::fs::write(&outside, b"x").unwrap();
        // A file outside the directory is refused…
        assert!(assert_within(&dir, &outside).is_err());
        // …and so is a file that does not exist (canonicalization fails).
        assert!(assert_within(&dir, &dir.join("missing.bin")).is_err());
        // A missing directory fails closed too.
        assert!(assert_within(&dir.join("nope"), &outside).is_err());
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn store_refuses_a_missing_cache_dir_before_any_write() {
        // The pre-write gate canonicalizes BEFORE any write and fails closed:
        // no temp file, no final file, not even the directory itself. A
        // refusal must leave nothing a later lookup could serve.
        let dir = scratch("gate");
        let missing = dir.join("does-not-exist");
        assert!(store(&missing, "https://cdn/x", "image/png", 200, b"bytes".to_vec()).is_err());
        assert!(!missing.exists(), "rien n'est créé quand le contrôle échoue");
        assert_eq!(cached_path(&dir, "https://cdn/x"), None);
    }

    // ── the scheme gate: https only, exercised without a connection ──

    #[test]
    fn only_https_ever_leaves_for_a_cdn() {
        assert!(ensure_https("https://cdn.example/a.jpg").is_ok());
        // The SSRF surface: loopback, private ranges, local schemes — all
        // reachable through an API-controlled image URL if the gate weakens.
        for bad in [
            "http://cdn.example/a.jpg",
            "http://127.0.0.1:8080/admin",
            "http://192.168.0.1/",
            "file:///C:/Windows/win.ini",
            "data:image/png;base64,QUJD",
            "https:/cdn.example/a.jpg",
        ] {
            assert!(ensure_https(bad).is_err(), "{bad} doit être refusé");
        }
    }

    // ── capped read: fabricated streams, no connection ──

    #[test]
    fn capped_body_refuses_the_announced_size() {
        let err = CappedBody::new(1024, Some(1025)).unwrap_err();
        assert!(err.to_string().contains("plafond"), "{err}");
        // Exactly at the cap is fine; one byte over is not.
        assert!(CappedBody::new(1024, Some(1024)).is_ok());
    }

    #[test]
    fn capped_body_cuts_a_stream_that_lies() {
        // No announced length — the stream itself must trip the cap.
        let mut body = CappedBody::new(10, None).unwrap();
        body.push(&[0u8; 6]).unwrap();
        let err = body.push(&[0u8; 6]).unwrap_err();
        assert!(err.to_string().contains("plafond"), "{err}");
        // The cap is cumulative, not per-chunk.
        let mut body = CappedBody::new(10, None).unwrap();
        for _ in 0..10 {
            body.push(&[0u8; 1]).unwrap();
        }
        assert!(body.push(&[0u8; 1]).is_err());
        assert_eq!(body.into_inner().len(), 10);
    }

    // ── atomic writes: an interruption leaves nothing servable ──

    #[test]
    fn store_writes_the_exact_bytes_atomically() {
        let dir = scratch("atomic");
        let url = "https://cdn.example/game.png";
        let path = store(&dir, url, "image/png", 200, b"complete".to_vec()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"complete");
        assert_eq!(cached_path(&dir, url), Some(path));
        // A different URL sees nothing.
        assert_eq!(cached_path(&dir, "https://cdn.example/other.png"), None);
    }

    #[test]
    fn an_interrupted_write_leaves_nothing_servable() {
        // The interruption model: the temp file exists, the rename never ran
        // (kill between the two). The final name must not exist and the
        // cache lookup must find nothing — a half-image is never served.
        let dir = scratch("interrupted");
        let name = file_name("https://cdn.example/cut.png", "image/png");
        let tmp = write_tmp(&dir, &name, b"half an image").unwrap();
        assert!(tmp.to_string_lossy().contains(TMP_SUFFIX));
        // …finalize() never runs — that IS the interruption…
        assert!(!dir.join(&name).exists(), "le nom final n'existe pas");
        assert_eq!(cached_path(&dir, "https://cdn.example/cut.png"), None);
        // …and the orphan never counts toward the cache size.
        assert_eq!(stats(&dir).file_count, 0);
        assert_eq!(stats(&dir).bytes, 0);
    }

    // ── a failure is never cached ──

    #[test]
    fn failures_never_reach_the_disk() {
        let dir = scratch("failures");
        let url = "https://cdn.example/dead.jpg";
        // Non-2xx…
        assert!(store(&dir, url, "image/jpeg", 404, b"not found".to_vec()).is_err());
        assert!(store(&dir, url, "image/jpeg", 500, b"oops".to_vec()).is_err());
        // …an empty 200 body…
        assert!(store(&dir, url, "image/jpeg", 200, Vec::new()).is_err());
        // …and NO redirect status is a success: redirects are never followed
        // (Policy::none), so a hop lands here as a final 3xx — fabricated
        // statuses, no connection is opened anywhere in this test.
        for status in [301u16, 302, 303, 307, 308] {
            assert!(store(&dir, url, "image/jpeg", status, b"moved".to_vec()).is_err());
        }
        // Nothing — no final file, no orphan — was left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert_eq!(cached_path(&dir, url), None);
    }

    #[test]
    fn a_new_content_type_replaces_the_old_sibling() {
        let dir = scratch("siblings");
        let url = "https://cdn/game";
        let jpg = store(&dir, url, "image/jpeg", 200, b"jpg-bytes".to_vec()).unwrap();
        assert!(jpg.to_string_lossy().ends_with(".jpg"));
        // The same URL now answers as a PNG: the old extension must go, or
        // cached_path (hash-prefix match) would serve whichever read_dir met
        // first — possibly the stale one, forever, both counting in the cap.
        let png = store(&dir, url, "image/png", 200, b"png-bytes".to_vec()).unwrap();
        assert!(png.to_string_lossy().ends_with(".png"));
        assert!(!jpg.exists(), "l'ancienne extension disparaît");
        assert_eq!(cached_path(&dir, url), Some(png));
        assert_eq!(stats(&dir).file_count, 1, "un seul fichier compte dans le plafond");
    }

    // ── purge: known sizes, LRU order, protection, never below zero ──

    #[test]
    fn purge_evicts_least_recently_used_first() {
        let dir = scratch("purge_lru");
        // Three 1 KiB files, "downloaded" at different times — the aged
        // mtimes model download dates, which really are chronological…
        let old_url = "https://cdn/old";
        let old = write_aged(&dir, &file_name(old_url, "image/jpeg"), 1024, 300);
        let mid = write_aged(&dir, &file_name("https://cdn/mid", "image/jpeg"), 1024, 200);
        let new = write_aged(&dir, &file_name("https://cdn/new", "image/jpeg"), 1024, 100);
        // …then the OLDEST is looked at. hit() is the lookup the commands
        // use: it refreshes the mtime, which must now save the file.
        assert_eq!(hit(&dir, old_url), Some(old.clone()));

        let freed = purge(&dir, 2560, None).unwrap();
        assert_eq!(freed, 1024, "une seule éviction suffit");
        assert!(old.exists(), "le fichier relu survit à la purge");
        assert!(!mid.exists(), "sans relecture, le plus ancien part en premier");
        assert!(new.exists(), "le récent reste");
        // Idempotent: under the cap now, nothing more moves.
        assert_eq!(purge(&dir, 2560, None).unwrap(), 0);
    }

    #[test]
    fn a_hit_never_refuses_an_image_even_if_the_clock_cannot_tick() {
        // A read-only file makes touch() fail (Windows refuses the write
        // access it needs): the image is still served — the LRU clock
        // degrades, the cache lookup never does.
        let dir = scratch("touch_fail");
        let url = "https://cdn/locked";
        let path = store(&dir, url, "image/jpeg", 200, b"locked".to_vec()).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms.clone()).unwrap();
        assert_eq!(hit(&dir, url), Some(path.clone()));
        // Clearing the very bit this test just set, or the scratch folder
        // cannot be deleted by the next run — the lint's "never unset"
        // advice does not apply to a test restoring its own fixture.
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn purge_keeps_going_until_under_the_cap() {
        let dir = scratch("purge2");
        write_aged(&dir, &file_name("https://cdn/a", "image/jpeg"), 1024, 400);
        write_aged(&dir, &file_name("https://cdn/b", "image/jpeg"), 1024, 300);
        write_aged(&dir, &file_name("https://cdn/c", "image/jpeg"), 1024, 200);
        write_aged(&dir, &file_name("https://cdn/d", "image/jpeg"), 1024, 100);
        // Cap 1 KiB: three of the four must go, in ascending age order.
        let freed = purge(&dir, 1024, None).unwrap();
        assert_eq!(freed, 3072);
        let left = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(left, 1, "il ne reste que le plus récent");
        assert!(dir.join(file_name("https://cdn/d", "image/jpeg")).exists());
    }

    #[test]
    fn purge_never_deletes_the_file_just_written() {
        let dir = scratch("protect");
        write_aged(&dir, &file_name("https://cdn/old", "image/jpeg"), 1024, 300);
        let fresh_name = file_name("https://cdn/fresh", "image/jpeg");
        write_aged(&dir, &fresh_name, 1024, 0);
        // Cap below even ONE file: everything else goes, the protected file
        // stays — over the cap, never deleted right after its download.
        let freed = purge(&dir, 512, Some(&fresh_name)).unwrap();
        assert_eq!(freed, 1024);
        assert!(dir.join(&fresh_name).exists(), "le fichier protégé survit");
        assert_eq!(stats(&dir).bytes, 1024, "le total ne descend jamais sous zéro");
    }

    #[test]
    fn store_downloaded_protects_the_download_it_just_wrote() {
        // The protect wiring, exercised through store_downloaded — the exact
        // function the artwork_fetch command calls — not against purge()
        // alone. The cap is below even ONE file, so after this download the
        // purge MUST evict: protect is what stops it eating the download
        // itself.
        let dir = scratch("protect_wiring");
        write_aged(&dir, &file_name("https://cdn/old", "image/jpeg"), 1024, 500);
        let fresh_url = "https://cdn/fresh";
        let path =
            store_downloaded(&dir, fresh_url, "image/jpeg", 200, vec![b'f'; 1024], 512).unwrap();
        assert!(path.exists(), "le téléchargement vient de coûter : la purge le protège");
        assert_eq!(cached_path(&dir, fresh_url), Some(path));
        assert!(
            !dir.join(file_name("https://cdn/old", "image/jpeg")).exists(),
            "l'ancien, non protégé, part à sa place"
        );
    }

    #[test]
    fn part_files_never_count_toward_the_cache_size() {
        let dir = scratch("stats-part");
        write_aged(&dir, &file_name("https://cdn/x", "image/jpeg"), 1024, 100);
        // Our own in-flight temp — the real shape write_tmp produces:
        // `<hash>.<ext>.part-<pid>` — plus a foreign file.
        write_aged(
            &dir,
            &format!("{}.jpg.part-{}", "ab".repeat(32), std::process::id()),
            4096,
            5,
        );
        write_aged(&dir, "notes.txt", 512, 900);
        let before = stats(&dir);
        assert_eq!(before.file_count, 1, "seul le fichier complet compte");
        assert_eq!(before.bytes, 1024);
    }

    #[test]
    fn purge_sweeps_part_files_from_dead_runs_not_ours() {
        let dir = scratch("sweep-part");
        write_aged(&dir, &file_name("https://cdn/x", "image/jpeg"), 1024, 100);
        // Our own in-flight temp (our PID in the name): a live download,
        // in the real shape write_tmp produces.
        let ours = write_aged(
            &dir,
            &format!("{}.jpg.part-{}", "ab".repeat(32), std::process::id()),
            2048,
            5,
        );
        // Another PID's temp: that writer can never finish it — an orphan.
        let orphan = write_aged(&dir, &format!("{}.jpg.part-999999", "cd".repeat(32)), 4096, 900);
        // A foreign name that is not a .part at all: never touched.
        let foreign = write_aged(&dir, "notes.txt", 512, 900);
        // Even under cap — the sweep runs before the early return.
        purge(&dir, MAX_CACHE_SIZE, None).unwrap();
        assert!(ours.exists(), "notre .part en cours d'écriture reste");
        assert!(!orphan.exists(), "l'orphelin d'un autre PID est balayé");
        assert!(foreign.exists(), "un nom étranger n'est jamais touché");
    }

    // ── stats & clear ──

    #[test]
    fn clear_removes_everything_and_reports() {
        let dir = scratch("clear");
        store(&dir, "https://cdn/1", "image/jpeg", 200, vec![1u8; 100]).unwrap();
        store(&dir, "https://cdn/2", "image/png", 200, vec![2u8; 50]).unwrap();
        write_aged(&dir, "orphan.part-1", 10, 10);
        let freed = clear(&dir).unwrap();
        assert_eq!(freed.file_count, 3, "les orphelins .part partent aussi");
        assert_eq!(freed.bytes, 160);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        // Clearing again is a no-op, not an error.
        let again = clear(&dir).unwrap();
        assert_eq!(again.file_count, 0);
        // And clearing a directory that never existed is fine.
        assert_eq!(clear(&dir.join("nope")).unwrap().file_count, 0);
    }

    // ── live: the only check fixtures cannot give (pitfall 36) ──

    /// Fetch a real Steam store header end to end — the path every fixture
    /// test misses: TLS to akamai, real Content-Type, real bytes. Composes
    /// the exact two halves the artwork_fetch command runs.
    #[tokio::test]
    #[ignore]
    async fn live_fetch_a_steam_header() {
        let dir = scratch("live");
        let client = ArtworkClient::new();
        let url = "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/730/header.jpg";
        let (status, content_type, bytes) =
            download(&client, url).await.expect("téléchargement réel");
        let path = store_downloaded(&dir, url, &content_type, status, bytes, MAX_CACHE_SIZE)
            .expect("écriture en cache");
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.is_empty());
        assert!(bytes.len() as u64 <= MAX_IMAGE_SIZE);
        assert_eq!(cached_path(&dir, url), Some(path.clone()));
        assert_within(&dir, &path).expect("le fichier réel reste sous le cache");
    }
}

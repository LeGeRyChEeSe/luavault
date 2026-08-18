//! HMAC-SHA-256 integrity guard for `index.json`.
//!
//! Each installation owns a random 32‑byte key stored in a dedicated file under
//! `config::data_dir()`. The key signs the exact bytes of `index.json` and the
//! resulting 32‑byte tag is written as a hex sidecar (`index.json.hmac`) in the
//! same directory.
//!
//! Threat model — what this *does* and *does not* protect:
//!
//! * **Detects** a corruption or an out‑of‑process modification of `index.json`.
//! * **Does not protect** against an attacker that controls the Windows account,
//!   the binary, and the data folder — such an attacker can read or replace the
//!   local key and forge a valid tag.
//!
//! The HMAC key is distinct from the Ed25519 public key, the network fingerprint,
//! and any export password. It is never exported or logged.
//!
//! This module owns **only** the safe primitives: path helpers, key management,
//! atomic writes, sidecar I/O, sign/verify, and adoption. Cache and index
//! parsing live in `library.rs`, where `INDEX_CACHE` is keyed by `(PathBuf,
//! FileStamp)` — never by stamp alone.

use anyhow::{anyhow, Context, Result};
use hmac::{Hmac, Mac};
use std::fs::File;
use std::io::{ErrorKind, Write};
use std::ops::Drop;
use std::path::{Path, PathBuf};
use std::str;

/// Encode bytes as lower-case hexadecimal for hashes and public keys.
pub fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decode an even-length hexadecimal string.
pub fn hex_to_bytes(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(anyhow!("hex string has an odd length"));
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).map_err(Into::into))
        .collect()
}

// ================================================================ Process-wide index guard

/// Serialises every read and every write of the (index.json, sidecar) pair.
///
/// The two files are published by two successive renames: the sequence
/// cannot be made atomic at the filesystem level, so it is made
/// indivisible for other threads. Without this, a reader passing
/// between the two sees a tag that no longer matches and fails closed
/// on an index that nobody touched.
pub static INDEX_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ================================================================ Constants

/// Magic bytes written by the current sidecar format.
const HMAC_MAGIC: &[u8] = b"LV-HMAC-v1";

/// Sidecar filename suffix.
const HMAC_SUFFIX: &str = ".hmac";

/// Key file name stored under `data_dir()`.
const HMAC_KEY_FILE: &str = "hmac.key";

/// Minimum temp-suffix entropy: 128 bits = 32 hex chars.
const TEMP_SUFFIX_HEX_LEN: usize = 32;

/// Maximum number of retries when creating a temp file (collision protection).
const MAX_RETRIES: usize = 16;

// ================================================================ Paths

/// Path to the local HMAC key file.
pub fn key_path(data_dir: &Path) -> PathBuf {
    data_dir.join(HMAC_KEY_FILE)
}

/// Path to the sidecar file next to `index.json`.
pub fn sidecar_path(index_path: &Path) -> PathBuf {
    let stem = index_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "index".to_string());
    index_path.parent().unwrap_or(Path::new("")).join(format!("{stem}{HMAC_SUFFIX}"))
}

/// Path to the index file (shared with library.rs).
pub fn index_path(lib: &Path) -> PathBuf {
    lib.join("index.json")
}

// ================================================================ RAII temp-file guard

/// A guard that ensures a temp file is never left behind.
///
/// On success the caller renames the temp file and disarms the guard.
/// On drop (failure path) the temp file is deleted.
struct TempGuard {
    path: PathBuf,
    armed: bool,
}

impl TempGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    /// Mark the guard as disarmed (file was renamed successfully).
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// ================================================================ CSPRNG temp suffix

/// Generate a hex suffix of `TEMP_SUFFIX_HEX_LEN` characters (128 bits of entropy).
fn random_hex_suffix() -> String {
    let mut buf = [0u8; TEMP_SUFFIX_HEX_LEN / 2];
    rand::fill(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

// ================================================================ Key management

/// Read or create the installation‑specific HMAC key.
///
/// * If the key file exists, reads and returns its 32 bytes.
/// * If it does not exist, generates a CSPRNG key, writes it atomically, and returns it.
/// * Returns `Err` when the key file is present but malformed (wrong size or unreadable).
pub fn load_or_create_key(data_dir: &Path) -> Result<[u8; 32]> {
    let path = key_path(data_dir);
    if path.exists() {
        let bytes = std::fs::read(&path).context("read hmac key")?;
        if bytes.len() != 32 {
            return Err(anyhow!(
                "hmac key file has wrong size (expected 32, got {})",
                bytes.len()
            ));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(key)
    } else {
        write_key_atomic(data_dir)
    }
}

/// Write a freshly generated key atomically.
fn write_key_atomic(data_dir: &Path) -> Result<[u8; 32]> {
    let path = key_path(data_dir);
    let mut key = [0u8; 32];
    rand::fill(&mut key);
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir).context("create data dir")?;
    write_atomic(dir, HMAC_KEY_FILE, &key).context("write hmac key")?;
    Ok(key)
}

// ================================================================ Atomic file writes

/// Write `data` to `<dir>/<name>.<suffix>` then rename to `<dir>/<name>`.
///
/// `create_new(true)` never claims a preexisting temp. An `AlreadyExists`
/// collision draws a fresh CSPRNG suffix and retries (bounded); any other
/// open error propagates. `flush` + `sync_all` run before the handle is
/// closed, and the rename happens after the close (Windows requirement).
/// The guard stays armed across the rename, so a failed rename still
/// removes the temp file; only a success disarms it.
fn write_atomic(dir: &Path, name: &str, data: &[u8]) -> Result<()> {
    for _ in 0..MAX_RETRIES {
        let suffix = random_hex_suffix();
        let tmp_name = format!("{name}.{suffix}");
        let tmp_path = dir.join(&tmp_name);

        let mut file = match File::options().create_new(true).write(true).open(&tmp_path) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(anyhow!("create temp file {tmp_name}: {e}")),
        };

        let mut guard = TempGuard::new(tmp_path);

        file.write_all(data)
            .with_context(|| format!("write temp file {tmp_name}"))?;
        file.flush()
            .with_context(|| format!("flush temp file {tmp_name}"))?;
        file.sync_all()
            .with_context(|| format!("sync_all temp file {tmp_name}"))?;

        // Close the handle *before* rename (Windows requirement).
        drop(file);

        std::fs::rename(&guard.path, dir.join(name))
            .with_context(|| format!("rename temp file to {name}"))?;
        guard.disarm();

        return Ok(());
    }

    Err(anyhow!(
        "failed to write {name} after {MAX_RETRIES} retries (persistent temp-file collision)"
    ))
}

// ================================================================ Sidecar I/O

/// Write the sidecar file atomically.
pub fn write_sidecar(index_path: &Path, tag: &[u8; 32]) -> Result<()> {
    let sidecar = sidecar_path(index_path);
    let dir = sidecar
        .parent()
        .ok_or_else(|| anyhow!("sidecar has no parent directory"))?;
    // Encode as uppercase hex (deterministic, case-stable).
    let hex: String = tag.iter().map(|b| format!("{:02X}", b)).collect();
    let header = std::str::from_utf8(HMAC_MAGIC)
        .map_err(|_| anyhow!("HMAC_MAGIC is not valid UTF-8"))?;
    let payload = format!("{header}:{hex}");
    write_atomic(dir, sidecar.file_name().unwrap().to_str().unwrap(), payload.as_bytes())
        .context("write sidecar")
}

/// Read and parse the sidecar file.
///
/// Returns `Ok(None)` if absent or empty.
/// Returns `Err` for any malformed content (bad magic, wrong length, bad hex).
pub fn read_sidecar(index_path: &Path) -> Result<Option<[u8; 32]>> {
    let sidecar = sidecar_path(index_path);
    if !sidecar.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&sidecar).context("read sidecar")?;
    let content = content.trim().to_string();
    if content.is_empty() {
        return Ok(None);
    }

    // Strict format: "{magic}:<64 uppercase hex digits>".
    let bytes = content.as_bytes();
    if !bytes.starts_with(HMAC_MAGIC) {
        return Err(anyhow!("sidecar has invalid magic bytes"));
    }
    if bytes.len() < HMAC_MAGIC.len() + 1 + 64 {
        return Err(anyhow!(
            "sidecar too short (expected at least {} bytes)",
            HMAC_MAGIC.len() + 1 + 64
        ));
    }
    if bytes[HMAC_MAGIC.len()] != b':' {
        return Err(anyhow!("sidecar missing ':' separator after magic"));
    }
    let hex_start = HMAC_MAGIC.len() + 1;
    let hex_bytes = &bytes[hex_start..hex_start + 64];

    // Strict: only uppercase hex digits.
    for &b in hex_bytes {
        match b {
            b'0'..=b'9' | b'A'..=b'F' => {}
            _ => return Err(anyhow!("sidecar contains non-uppercase-hex character")),
        }
    }

    let mut tag = [0u8; 32];
    for i in 0..32 {
        let hi = hex_ascii_to_nib(hex_bytes[i * 2]).map_err(|e| anyhow!("sidecar hex error: {e}"))?;
        let lo = hex_ascii_to_nib(hex_bytes[i * 2 + 1]).map_err(|e| anyhow!("sidecar hex error: {e}"))?;
        tag[i] = (hi << 4) | lo;
    }
    Ok(Some(tag))
}

fn hex_ascii_to_nib(c: u8) -> Result<u8> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(anyhow!("invalid hex character")),
    }
}

/// Sign the exact bytes of a file with HMAC-SHA-256.
pub fn sign_bytes(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length is 32");
    mac.update(data);
    let result = mac.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(result.into_bytes().as_slice());
    out
}

/// Compare a tag against data through the crypto crate's constant-time
/// `Mac::verify_slice`. Callers must go through this function, never `==` /
/// `!=` on the bytes — the LOT-21 contract requires the library's
/// constant-time check, not a comparison that short-circuits on the first
/// differing byte.
pub fn verify_tag(key: &[u8; 32], data: &[u8], tag: &[u8; 32]) -> bool {
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC key length is 32");
    mac.update(data);
    mac.verify_slice(tag).is_ok()
}

/// Sign `index.json` and write the sidecar.
pub fn sign_index(index_path: &Path, key: &[u8; 32]) -> Result<()> {
    let raw = std::fs::read(index_path).context("read index.json for signing")?;
    let tag = sign_bytes(key, &raw);
    write_sidecar(index_path, &tag)
}

/// Verify the HMAC of `index.json` against the stored sidecar.
///
/// Returns `Ok(true)` if the signature matches, `Ok(false)` if absent/mismatched,
/// and `Err` only on I/O or parse errors.
pub fn verify(index_path: &Path, key: &[u8; 32]) -> Result<bool> {
    let raw = std::fs::read(index_path).context("read index.json for verification")?;
    let Ok(Some(stored)) = read_sidecar(index_path) else {
        return Ok(false);
    };
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC key length is 32");
    mac.update(&raw);
    Ok(mac.verify_slice(&stored).is_ok())
}

/// Check whether a sidecar exists for `index.json`.
pub fn has_sidecar(index_path: &Path) -> bool {
    sidecar_path(index_path).exists()
}

/// Remove the sidecar file.
pub fn remove_sidecar(index_path: &Path) -> Result<()> {
    let sidecar = sidecar_path(index_path);
    if sidecar.exists() {
        std::fs::remove_file(&sidecar).context("remove sidecar")?;
    }
    Ok(())
}

/// Check whether the local HMAC key exists.
pub fn has_key(data_dir: &Path) -> bool {
    key_path(data_dir).exists()
}

// ================================================================ adopt_index_with_data_dir

/// Adopt an existing unsigned index into HMAC protection.
///
/// - index absent → success (nothing to do);
/// - index signed (sidecar present) → verify it;
/// - index unsigned + valid JSON → sign it (authorise, even if key already exists);
/// - sidecar present but HMAC fails → `Err`;
/// - JSON invalid → `Err`.
pub fn adopt_index_with_data_dir(lib: &Path, data_dir: &Path) -> Result<()> {
    let idx = index_path(lib);
    if !idx.exists() {
        return Ok(());
    }

    let sidecar_exists = has_sidecar(&idx);

    if sidecar_exists {
        // Sidecar present — must verify.
        let key = load_or_create_key(data_dir)?;
        if !verify(&idx, &key)? {
            return Err(anyhow!("adopt failed: existing sidecar HMAC does not match index.json"));
        }
        return Ok(());
    }

    // No sidecar — validate JSON and sign.
    let raw = std::fs::read_to_string(&idx).context("read index.json for adoption")?;
    let _entries: serde_json::Value =
        serde_json::from_str(&raw).context("adopt: index.json is not a JSON value")?;

    // Create key if it doesn't exist, or use existing key.
    let key = load_or_create_key(data_dir)?;
    sign_index(&idx, &key)?;

    Ok(())
}

// ================================================================ save_index_with_data_dir

/// Save entries to `lib/index.json` and sign them.
///
/// 1. Serialises entries to JSON.
/// 2. Writes JSON atomically.
/// 3. Signs the written bytes.
/// 4. Invalidates nothing — cache management is owned by library.rs.
pub fn save_index_with_data_dir(lib: &Path, data_dir: &Path, raw: &[u8]) -> Result<()> {
    let _guard = INDEX_GUARD
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let idx = index_path(lib);
    let dir = idx.parent().unwrap_or(lib);

    // Write JSON atomically.
    write_atomic(dir, "index.json", raw).context("write index.json")?;

    // Sign the written bytes.
    // Need the key — create if not exists (migration path for writers).
    let key = load_or_create_key(data_dir).context("load hmac key for signing")?;
    sign_index(&idx, &key).context("sign index.json")?;

    Ok(())
}

// ================================================================ Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("lv_hmac21_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_index(lib: &Path, content: &str) {
        let idx = index_path(lib);
        std::fs::create_dir_all(lib).unwrap();
        std::fs::write(&idx, content).unwrap();
    }

    // ------------------------------------------------------------------ 1. Paths

    #[test]
    fn index_path_is_lib_join_index_json() {
        let lib = PathBuf::from("C:\\games\\MyLibrary");
        assert_eq!(
            index_path(&lib),
            lib.join("index.json")
        );
    }

    #[test]
    fn sidecar_path_is_next_to_index() {
        let idx = PathBuf::from("C:\\games\\MyLibrary\\index.json");
        assert_eq!(
            sidecar_path(&idx),
            PathBuf::from("C:\\games\\MyLibrary\\index.hmac")
        );
    }

    #[test]
    fn sidecar_path_handles_no_parent() {
        let idx = PathBuf::from("index.json");
        assert_eq!(
            sidecar_path(&idx),
            PathBuf::from("index.hmac")
        );
    }

    // ------------------------------------------------------------------ 2. Key management

    #[test]
    fn load_or_create_key_creates_key() {
        let data = scratch("key_create");
        assert!(!has_key(&data));
        let key = load_or_create_key(&data).unwrap();
        assert!(has_key(&data));
        // Key is 32 random bytes.
        assert_eq!(key.len(), 32);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn load_or_create_key_reads_existing() {
        let data = scratch("key_read");
        let original = [7u8; 32];
        std::fs::write(key_path(&data), original).unwrap();
        let key = load_or_create_key(&data).unwrap();
        assert_eq!(key, original);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn load_or_create_key_rejects_malformed() {
        let data = scratch("key_bad");
        std::fs::write(key_path(&data), b"too short").unwrap();
        assert!(load_or_create_key(&data).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ------------------------------------------------------------------ 3. Sidecar I/O

    #[test]
    fn write_and_read_sidecar_roundtrip() {
        let data = scratch("sidecar_rt");
        let lib = data.join("lib");
        make_index(&lib, "[]");
        let key = [42u8; 32];
        let tag = sign_bytes(&key, b"[]");
        write_sidecar(&index_path(&lib), &tag).unwrap();
        assert!(std::fs::read_to_string(sidecar_path(&index_path(&lib)))
            .unwrap()
            .starts_with("LV-HMAC-v1:"));
        assert!(has_sidecar(&index_path(&lib)));
        let loaded = read_sidecar(&index_path(&lib)).unwrap().unwrap();
        assert_eq!(loaded, tag);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn read_sidecar_rejects_an_unknown_magic_with_a_valid_tag() {
        let data = scratch("sidecar_legacy");
        let lib = data.join("lib");
        make_index(&lib, "[]");
        let key = [42u8; 32];
        let tag = sign_bytes(&key, b"[]");
        let hex: String = tag.iter().map(|b| format!("{b:02X}")).collect();
        let unknown_magic = "FOOBAR-v9";
        std::fs::write(sidecar_path(&index_path(&lib)), format!("{unknown_magic}:{hex}")).unwrap();
        let error = read_sidecar(&index_path(&lib)).unwrap_err();
        assert!(error.to_string().contains("invalid magic bytes"));
        assert!(!verify(&index_path(&lib), &key).unwrap());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn read_sidecar_rejects_an_invalid_tag() {
        let data = scratch("sidecar_bad_tag");
        let lib = data.join("lib");
        make_index(&lib, "[]");
        let sidecar = sidecar_path(&index_path(&lib));
        let header = std::str::from_utf8(HMAC_MAGIC).unwrap();
        std::fs::write(&sidecar, format!("{header}:{}", "Z".repeat(64))).unwrap();
        assert!(read_sidecar(&index_path(&lib)).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn read_sidecar_returns_none_when_absent() {
        let data = scratch("sidecar_none");
        let lib = data.join("lib");
        make_index(&lib, "[]");
        assert!(read_sidecar(&index_path(&lib)).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn read_sidecar_rejects_malformed() {
        let data = scratch("sidecar_bad");
        let lib = data.join("lib");
        make_index(&lib, "[]");
        let sp = sidecar_path(&index_path(&lib));
        std::fs::write(&sp, "garbage").unwrap();
        assert!(read_sidecar(&index_path(&lib)).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ------------------------------------------------------------------ 4. Verify

    #[test]
    fn verify_returns_true_for_valid() {
        let data = scratch("verify_ok");
        let lib = data.join("lib");
        make_index(&lib, "[]");
        let key = load_or_create_key(&data).unwrap();
        sign_index(&index_path(&lib), &key).unwrap();
        assert!(verify(&index_path(&lib), &key).unwrap());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn verify_returns_false_when_tampered() {
        let data = scratch("verify_bad");
        let lib = data.join("lib");
        make_index(&lib, "[]");
        let key = load_or_create_key(&data).unwrap();
        sign_index(&index_path(&lib), &key).unwrap();
        // Tamper the index.
        make_index(&lib, "[1]");
        assert!(!verify(&index_path(&lib), &key).unwrap());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ------------------------------------------------------------------ 5. sign_bytes deterministic

    #[test]
    fn sign_bytes_deterministic() {
        let key = [42u8; 32];
        let data = b"hello";
        let a = sign_bytes(&key, data);
        let b = sign_bytes(&key, data);
        assert_eq!(a, b);
    }

    // ------------------------------------------------------------------ 5b. verify_tag

    #[test]
    fn verify_tag_accepts_a_matching_tag() {
        let key = [42u8; 32];
        let data = b"index bytes";
        let tag = sign_bytes(&key, data);
        assert!(verify_tag(&key, data, &tag));
    }

    #[test]
    fn verify_tag_rejects_a_wrong_tag_or_data() {
        let key = [42u8; 32];
        let tag = sign_bytes(&key, b"index bytes");
        assert!(!verify_tag(&key, b"index byteX", &tag));
        let mut bad = tag;
        bad[0] ^= 0x01;
        assert!(!verify_tag(&key, b"index bytes", &bad));
    }

    // ------------------------------------------------------------------ 6. Adopt

    #[test]
    fn adopt_unsigned_creates_key_and_sidecar() {
        let data = scratch("adopt_create");
        let lib = data.join("lib");
        make_index(&lib, r#"[]"#);
        adopt_index_with_data_dir(&lib, &data).unwrap();
        assert!(has_key(&data));
        assert!(has_sidecar(&index_path(&lib)));
        // Verify passes.
        let key = load_or_create_key(&data).unwrap();
        assert!(verify(&index_path(&lib), &key).unwrap());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn adopt_signed_but_wrong_hmac_errors() {
        let data = scratch("adopt_bad");
        let lib = data.join("lib");
        make_index(&lib, r#"[]"#);
        let key = load_or_create_key(&data).unwrap();
        sign_index(&index_path(&lib), &key).unwrap();
        // Tamper.
        make_index(&lib, "[1]");
        assert!(adopt_index_with_data_dir(&lib, &data).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn adopt_invalid_json_errors() {
        let data = scratch("adopt_json");
        let lib = data.join("lib");
        make_index(&lib, "{bad}");
        assert!(adopt_index_with_data_dir(&lib, &data).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn adopt_absent_index_is_success() {
        let data = scratch("adopt_absent");
        let lib = data.join("lib");
        // No index file.
        adopt_index_with_data_dir(&lib, &data).unwrap();
        // No key or sidecar created.
        assert!(!has_key(&data));
        assert!(!has_sidecar(&index_path(&lib)));
        let _ = std::fs::remove_dir_all(&data);
    }

    // ------------------------------------------------------------------ 7. save_index_with_data_dir

    #[test]
    fn save_index_signs_and_verifies() {
        let data = scratch("save");
        let lib = data.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        let raw = b"[]";
        save_index_with_data_dir(&lib, &data, raw).unwrap();
        assert!(has_sidecar(&index_path(&lib)));
        let key = load_or_create_key(&data).unwrap();
        assert!(verify(&index_path(&lib), &key).unwrap());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ------------------------------------------------------------------ 8. Temp files never left behind

    #[test]
    fn no_residual_temps_on_success() {
        let data = scratch("temp");
        let lib = data.join("lib");
        make_index(&lib, r#"[]"#);
        let key = load_or_create_key(&data).unwrap();
        sign_index(&index_path(&lib), &key).unwrap();
        // No residual temps in lib or data dir.
        for entry in std::fs::read_dir(&data).unwrap().filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("hmac.key.") {
                panic!("residual key temp: {name}");
            }
        }
        for entry in std::fs::read_dir(&lib).unwrap().filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("index.json.") || name.starts_with("index.hmac.") {
                panic!("residual index temp: {name}");
            }
        }
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn no_residual_temps_on_failure() {
        let data = scratch("temp_fail");
        let lib = data.join("lib");
        make_index(&lib, r#"[]"#);
        let key = load_or_create_key(&data).unwrap();
        sign_index(&index_path(&lib), &key).unwrap();
        // Tamper — adopt will fail.
        make_index(&lib, "[1]");
        assert!(adopt_index_with_data_dir(&lib, &data).is_err());
        // No residual temps.
        for entry in std::fs::read_dir(&data).unwrap().filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("hmac.key.") {
                panic!("residual key temp after failure: {name}");
            }
        }
        let _ = std::fs::remove_dir_all(&data);
    }
}

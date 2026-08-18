//! Encrypted `.luabak` v2 format: Argon2id + AES-256-GCM streaming.
//!
//! Format layout (all little-endian unless noted):
//!
//! ```text
//! [magic: 8 bytes  "LVBCK-v2"]
//! [version: u32 LE]
//! [argon2_memory: u32 LE  KiB]
//! [argon2_iters: u32 LE]
//! [argon2_par: u32 LE]
//! [salt: 32 bytes]
//! [stream_nonce: 7 bytes]
//! [plaintext_size: u64 LE]
//! [frame_1 … frame_N]   each = AES-GCM ciphertext (msg.len() + 16)
//! ```
//!
//! * **Magic**: `LVBCK-v2` — exactly 8 bytes.
//! * **Version**: `2` (only version 2 is accepted).
//! * **KDF**: Argon2id with explicit memory (KiB), iterations, parallelism.
//! * **Salt**: 32 random bytes per export.
//! * **Stream nonce**: 7 random bytes (for the 64-bit BE counter in AES-GCM).
//! * **plaintext_size**: uncompressed ZIP byte count.
//! * **No header tag** — the full serialised header (magic … size) is the AAD of
//!   every AES-GCM frame.
//!
//! The header is authenticated as AAD so any bit flip in magic, version, params,
//! salt, nonce or size causes authentication failure.
//!
//! KDF parameters are bounded per version to prevent arbitrarily expensive inputs.
//! Only version 2 is accepted; other versions are rejected before derivation.
//!
//! All temporary files use a cryptographic random suffix to avoid name collisions
//! and preexisting-file attacks.  A helper (`UniqueTemp`) creates the file with
//! `create_new(true)` and cleans it up in `Drop` — it never touches a path it
//! did not itself create.

use anyhow::{anyhow, bail, Context, Result};
use aes_gcm::{
    aead::{
        Payload,
        stream::{DecryptorBE32, EncryptorBE32},
    },
    Aes256Gcm,
    KeyInit,
};
use aes_gcm::aead::generic_array::GenericArray;

use argon2::Argon2;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Encode bytes as lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Magic written by the current encrypted backup format.
const MAGIC: &[u8] = b"LVBCK-v2"; // 8 bytes

/// Current format version.
const VERSION: u32 = 2;

/// Supported memory in KiB (Argon2 param).
const ARGON2_MEM: u32 = 64 * 1024;

/// Supported iterations (Argon2 param).
const ARGON2_ITERS: u32 = 3;

/// Supported parallelism (Argon2 param).
const ARGON2_PAR: u32 = 1;

/// Salt length in bytes.
const SALT_LEN: usize = 32;

/// Stream nonce length (7 bytes → 64-bit BE counter).
const STREAM_NONCE_LEN: usize = 7;

/// AES-GCM tag length.
const TAG_LEN: usize = 16;

/// AES-256 key length.
const KEY_LEN: usize = 32;

/// Plaintext block size for framing.
const BLOCK_SIZE: usize = 65_536;

/// Header payload length after the magic bytes.
const HEADER_PAYLOAD_LEN: usize = 4 + 4 + 4 + 4 + SALT_LEN + STREAM_NONCE_LEN + 8;
const HEADER_LEN: usize = MAGIC.len() + HEADER_PAYLOAD_LEN;

/// Maximum allowed plaintext size (2 GiB) — guard against allocation blow-up.
const MAX_PLAINTEXT_SIZE: u64 = 2 * 1024 * 1024 * 1024;

/// Minimum number of random bytes in a unique temp suffix (128 bits).
const TEMP_SUFFIX_BYTES: usize = 16;

/// Maximum number of retries when a unique temp name collides.
const TEMP_MAX_RETRIES: usize = 8;

// ── Unique temporary file helper ─────────────────────────────────────────────

/// A uniquely-named temporary file next to a destination.
///
/// Created with `OpenOptions::create_new(true)` so a preexisting path is
/// always refused.  On `Drop` the file is unlinked; if `disarm()` was
/// called first the drop is a no-op (the caller now owns the path).
///
/// ## Protocol
///
/// 1. The handle stays open during writes — this prevents the OS from
///    reusing the name while we still have data to flush.
/// 2. Before a rename, call `release()` to close the handle (required on
///    Windows — a rename fails while the file is open). The path stays
///    tracked so `Drop` can still clean up if the rename fails.
/// 3. After a successful rename, call `disarm()` to stop `Drop` from
///    deleting the now-final file.
/// 4. If the rename fails, `Drop` removes the temp path.
struct UniqueTemp {
    path: Option<PathBuf>,
    /// The raw file handle — kept open so the OS does not reuse the name.
    _file: Option<std::fs::File>,
}

impl UniqueTemp {
    /// Create a new unique temp file in `dir` with a given prefix.
    ///
    /// Generates a cryptographically random suffix (16 bytes → 128 bits),
    /// retries up to `TEMP_MAX_RETRIES` times on `AlreadyExists`.
    fn new(dir: &Path, prefix: &str) -> Result<Self> {
        for attempt in 0..TEMP_MAX_RETRIES {
            let suffix: Vec<u8> = (0..TEMP_SUFFIX_BYTES).map(|_| rand::random()).collect();
            let name = format!("{prefix}_{suffix_hex}.tmp", suffix_hex = hex_encode(&suffix));
            let path = dir.join(name);

            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
            {
                Ok(file) => {
                    log::debug!(
                        "created unique temp: {} (attempt {})",
                        path.display(),
                        attempt + 1
                    );
                    return Ok(UniqueTemp {
                        path: Some(path),
                        _file: Some(file),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < TEMP_MAX_RETRIES - 1 => {
                    log::trace!("unique temp collision on {}, retrying", path.display());
                    continue;
                }
                Err(e) => {
                    return Err(e).context("create unique temp file");
                }
            }
        }
        bail!("unique temp name exhausted after {} retries", TEMP_MAX_RETRIES);
    }

    /// Close the file handle so the path can be renamed.
    ///
    /// Required on Windows: a rename fails while another handle is open.
    /// The path stays tracked so `Drop` can still clean up if the rename
    /// fails.
    fn release(&mut self) -> Result<PathBuf> {
        self._file.take()
            .ok_or_else(|| anyhow!("temp file already released"))?;
        Ok(self.path.as_ref().ok_or_else(|| anyhow!("temp already claimed"))?.clone())
    }

    /// Disarm the guard: stop `Drop` from deleting the path.
    ///
    /// Call **after** a successful rename — the caller now owns the path.
    fn disarm(&mut self) {
        self.path.take();
    }
}

impl Drop for UniqueTemp {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

// ── KDF ──────────────────────────────────────────────────────────────────────

/// Derive 256 bits from a password using Argon2id v0x13.
///
/// `Params::new` receives KiB directly — no extra multiplication.
///
/// The derived key is wrapped in `Zeroizing` to prevent leaks on panic.
fn derive_key(password: &[u8], salt: &[u8; SALT_LEN]) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let params = argon2::Params::new(
        ARGON2_MEM,
        ARGON2_ITERS,
        ARGON2_PAR,
        Some(KEY_LEN),
    ).map_err(|e| anyhow!("Argon2id params creation failed: {e}"))?;
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        params,
    );
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(password, salt, &mut *key)
        .map_err(|e| anyhow!("Argon2id derivation failed: {e}"))?;
    Ok(key)
}

/// Validate KDF parameters against version-bounded limits.
///
/// For version 2, **only** the exact values are accepted — no more, no less.
/// Any deviation (including zero or a value below the constant) is rejected
/// before Argon2 is invoked.
fn validate_kdf_params(memory: u32, iterations: u32, parallelism: u32) -> Result<()> {
    if memory != ARGON2_MEM {
        bail!(
            "KDF memory mismatch: got {} KiB, expected {} KiB (version {})",
            memory,
            ARGON2_MEM,
            VERSION
        );
    }
    if iterations != ARGON2_ITERS {
        bail!(
            "KDF iterations mismatch: got {} (expected {} for version {})",
            iterations,
            ARGON2_ITERS,
            VERSION
        );
    }
    if parallelism != ARGON2_PAR {
        bail!(
            "KDF parallelism mismatch: got {} (expected {} for version {})",
            parallelism,
            ARGON2_PAR,
            VERSION
        );
    }
    Ok(())
}

/// Compute the expected file length: header + plaintext + one GCM tag per frame.
///
/// Uses checked arithmetic throughout — returns `None` on any overflow.
fn expected_file_len(plaintext_size: u64, header_len: usize) -> Option<u64> {
    let frame_count = if plaintext_size == 0 {
        1
    } else {
        // Ceiling division: (plaintext_size + BLOCK_SIZE - 1) / BLOCK_SIZE
        let frames = (plaintext_size as usize).div_ceil(BLOCK_SIZE);
        // Guard against absurdly large frame counts (would overflow u64).
        u64::try_from(frames).ok()?
    };
    let plaintext_bytes = plaintext_size.checked_add(frame_count.checked_mul(TAG_LEN as u64)?)?;
    (header_len as u64)
        .checked_add(plaintext_bytes)
}

// ── Export ───────────────────────────────────────────────────────────────────

/// Export a ZIP stream into an encrypted file.
///
/// * `zip_reader`: reads the raw ZIP bytes.
/// * `dest`: final destination path.
/// * `password`: **must be non-empty** — plaintext passthrough is rejected.
///
/// Returns the final file size on success.
///
/// Flow:
/// 1. Validate password is non-empty.
/// 2. Generate salt + stream nonce.
/// 3. Derive key from password + salt.
/// 4. Copy the ZIP reader into a unique temp file (tmp1) next to dest to know its size.
/// 5. Build the final header (magic … plaintext_size) with the real size.
/// 6. Stream-encrypt tmp1 into tmp2 using EncryptorBE32, AAD = header.
/// 7. Concatenate header + tmp2 → tmp3, then rename tmp3 → dest.
/// 8. Clean up all temps on error (UniqueTemp Drop).
pub fn encrypt_export(
    mut zip_reader: Box<dyn Read>,
    dest: &Path,
    password: Option<&str>,
) -> Result<u64> {
    // Reject empty or missing password — no plaintext passthrough.
    let password = password.context("password is required for encrypted export")?;
    if password.is_empty() {
        bail!("password must not be empty");
    }

    let password_bytes = Zeroizing::new(password.as_bytes().to_vec());
    let mut salt = [0u8; SALT_LEN];
    rand::fill(&mut salt);
    let mut stream_nonce = [0u8; STREAM_NONCE_LEN];
    rand::fill(&mut stream_nonce);

    let key = derive_key(&password_bytes, &salt)?;

    // Create temp files in the same directory as dest.
    let dest_dir = dest.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dest_dir).context("create dest dir")?;

    // Unique temps for each stage — they auto-clean on error via Drop.
    let mut tmp_zip = UniqueTemp::new(dest_dir, "enc_zip")?;
    let tmp_zip_path = tmp_zip.path.as_ref().unwrap();
    let mut tmp_enc = UniqueTemp::new(dest_dir, "enc_enc")?;
    let tmp_enc_path = tmp_enc.path.as_ref().unwrap();
    let mut tmp_final = UniqueTemp::new(dest_dir, "enc_final")?;

    // Step 1: Copy ZIP reader into tmp_zip to know its size.
    {
        let zip_file = tmp_zip._file.as_mut().unwrap();
        let _bytes = std::io::copy(&mut zip_reader, zip_file).context("copy zip to tmp")?;
        zip_file.flush().context("flush tmp_zip")?;

        // Read size.
        let plaintext_size = std::fs::metadata(tmp_zip_path).context("stat tmp_zip")?.len();

        // Bound plaintext_size.
        if plaintext_size > MAX_PLAINTEXT_SIZE {
            bail!("plaintext size too large: {plaintext_size} bytes");
        }

        // Step 2: Build final header.
        let mut header = vec![0u8; HEADER_LEN];
        header[..MAGIC.len()].copy_from_slice(MAGIC);
        header[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&VERSION.to_le_bytes());
        let off = MAGIC.len() + 4;
        header[off..off + 4].copy_from_slice(&ARGON2_MEM.to_le_bytes());
        header[off + 4..off + 8].copy_from_slice(&ARGON2_ITERS.to_le_bytes());
        header[off + 8..off + 12].copy_from_slice(&ARGON2_PAR.to_le_bytes());
        header[off + 12..off + 12 + SALT_LEN].copy_from_slice(&salt);
        let nonce_start = off + 12 + SALT_LEN;
        header[nonce_start..nonce_start + STREAM_NONCE_LEN].copy_from_slice(&stream_nonce);
        header[nonce_start + STREAM_NONCE_LEN..nonce_start + STREAM_NONCE_LEN + 8]
            .copy_from_slice(&plaintext_size.to_le_bytes());
        debug_assert_eq!(header.len(), HEADER_LEN);

        // Step 3: Stream-encrypt tmp_zip → tmp_enc, AAD = header.
        {
            let cipher = Aes256Gcm::new_from_slice(&*key).expect("key length is 32");
            let stream_nonce_arr = GenericArray::from_slice(&stream_nonce);
            let mut encryptor = EncryptorBE32::from_aead(cipher, stream_nonce_arr);
            let enc_file = tmp_enc._file.as_mut().unwrap();
            let mut in_f = std::fs::File::open(tmp_zip_path).context("open tmp_zip")?;

            // Ceiling division with checked arithmetic.
            let frame_count = if plaintext_size == 0 {
                1
            } else {
                let pc = plaintext_size as usize;
                pc.div_ceil(BLOCK_SIZE)
            };

            // Full frames (not the last one).
            for frame_idx in 0..frame_count.saturating_sub(1) {
                let mut buf = vec![0u8; BLOCK_SIZE];
                in_f.read_exact(&mut buf).context("read zip block")?;
                let encrypted = encryptor
                    .encrypt_next(Payload {
                        msg: &buf,
                        aad: &header,
                    })
                    .map_err(|e| anyhow!("encrypt block {frame_idx}: {e}"))?;
                enc_file.write_all(&encrypted).context("write encrypted")?;
            }

            // Last frame.
            let remaining = if frame_count == 1 && plaintext_size == 0 {
                0usize
            } else {
                let r = (plaintext_size as usize) - (frame_count - 1) * BLOCK_SIZE;
                std::cmp::min(BLOCK_SIZE, r)
            };
            if remaining > 0 {
                let mut buf = vec![0u8; remaining];
                in_f.read_exact(&mut buf).context("read last zip block")?;
                let encrypted = encryptor
                    .encrypt_last(Payload {
                        msg: &buf,
                        aad: &header,
                    })
                    .map_err(|e| anyhow!("encrypt last block: {e}"))?;
                enc_file.write_all(&encrypted).context("write last encrypted")?;
            } else {
                // Zero-length ZIP: single empty frame.
                let encrypted = encryptor
                    .encrypt_last(Payload {
                        msg: &[][..],
                        aad: &header,
                    })
                    .map_err(|e| anyhow!("encrypt last block: {e}"))?;
                enc_file.write_all(&encrypted).context("write final encrypted")?;
            }
            enc_file.flush().context("flush tmp_enc")?;
        }

        // Step 4: Concatenate header + tmp_enc → tmp_final, then rename.
        {
            let final_file = tmp_final._file.as_mut().unwrap();
            final_file.write_all(&header).context("write header to tmp_final")?;
            let mut src_enc = std::fs::File::open(tmp_enc_path).context("open tmp_enc")?;
            std::io::copy(&mut src_enc, final_file).context("copy cipher to tmp_final")?;
            final_file.flush().context("flush tmp_final")?;
            final_file.sync_all().context("sync_all tmp_final")?;
        }

        // Release the final temp handle (required for rename on Windows),
        // then rename. If rename fails, Drop cleans up tmp_final.
        let final_path = tmp_final.release()?;
        std::fs::rename(&final_path, dest).context("rename encrypted to dest")?;
        tmp_final.disarm(); // rename succeeded — stop Drop from deleting dest.

        // Clean up remaining temps.
        drop(tmp_enc);
        drop(tmp_zip);

        // Return the actual file size on disk (header + ciphertext).
        Ok(std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0))
    }
}

// ── Import ───────────────────────────────────────────────────────────────────

/// Detect whether a file is an encrypted v2 archive.
pub fn is_encrypted(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; MAGIC.len()];
    match f.read_exact(&mut magic) {
        Ok(()) => magic == *MAGIC,
        _ => false,
    }
}

/// Import an encrypted archive.
///
/// * `src`: source encrypted file.
/// * `dest`: destination path for the decrypted ZIP.
/// * `password`: the password to derive the key — **must be non-empty**.
///
/// Flow:
/// 1. Validate password is non-empty and file is encrypted (v2).
/// 2. Refuse `src == dest`.
/// 3. Read and validate header (magic, version, KDF params, salt, nonce, size).
/// 4. Derive key from password + salt.
/// 5. Read payload size, bound it.
/// 6. Validate file length: header + plaintext + 16 × frame_count.
/// 7. Stream-decrypt into a unique temp file using DecryptorBE32, AAD = header.
/// 8. Clean up temp on error (UniqueTemp Drop).
pub fn decrypt_import(src: &Path, dest: &Path, password: &str) -> Result<u64> {
    // Reject empty password — no plaintext passthrough.
    if password.is_empty() {
        bail!("password must not be empty for encrypted import");
    }

    // Refuse src == dest.
    if src == dest {
        bail!("source and destination must not be the same file");
    }

    // Require encrypted format.
    if !is_encrypted(src) {
        bail!("not an LuaVault encrypted archive — plaintext passthrough is not supported");
    }

    let password_bytes = Zeroizing::new(password.as_bytes().to_vec());

    let mut f = std::fs::File::open(src).context("open encrypted file")?;
    let file_len = f.metadata().context("read metadata")?.len();

    // Read header with read_exact (never a single read that might return partial).
    let mut prefix = [0u8; MAGIC.len()];
    f.read_exact(&mut prefix).context("read magic")?;
    if prefix != *MAGIC {
        bail!("not an LuaVault encrypted archive");
    }
    f.rewind().context("rewind encrypted file")?;

    if file_len < (HEADER_LEN + TAG_LEN) as u64 {
        bail!("file too short for header + at least one frame");
    }

    let mut header = vec![0u8; HEADER_LEN];
    f.read_exact(&mut header).context("read header")?;

    // Validate magic.
    if !header.starts_with(MAGIC) {
        bail!("not an LuaVault encrypted archive");
    }

    // Read version.
    let version = u32::from_le_bytes([
        header[MAGIC.len()],
        header[MAGIC.len() + 1],
        header[MAGIC.len() + 2],
        header[MAGIC.len() + 3],
    ]);
    if version != VERSION {
        bail!("unsupported version: {version} (expected {VERSION})");
    }

    // Read KDF params.
    let offset = MAGIC.len() + 4;
    let memory = u32::from_le_bytes([
        header[offset],
        header[offset + 1],
        header[offset + 2],
        header[offset + 3],
    ]);
    let iterations = u32::from_le_bytes([
        header[offset + 4],
        header[offset + 5],
        header[offset + 6],
        header[offset + 7],
    ]);
    let parallelism = u32::from_le_bytes([
        header[offset + 8],
        header[offset + 9],
        header[offset + 10],
        header[offset + 11],
    ]);
    validate_kdf_params(memory, iterations, parallelism)?;

    // Read salt.
    let salt_off = offset + 12;
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&header[salt_off..salt_off + SALT_LEN]);

    // Read stream nonce.
    let nonce_off = salt_off + SALT_LEN;
    let mut stream_nonce = [0u8; STREAM_NONCE_LEN];
    stream_nonce.copy_from_slice(&header[nonce_off..nonce_off + STREAM_NONCE_LEN]);

    // Read plaintext_size.
    let size_off = nonce_off + STREAM_NONCE_LEN;
    let plaintext_size = u64::from_le_bytes([
        header[size_off],
        header[size_off + 1],
        header[size_off + 2],
        header[size_off + 3],
        header[size_off + 4],
        header[size_off + 5],
        header[size_off + 6],
        header[size_off + 7],
    ]);

    // Bound plaintext_size.
    if plaintext_size > MAX_PLAINTEXT_SIZE {
        bail!("plaintext size too large: {plaintext_size} bytes");
    }

    // Validate expected file length using checked arithmetic.
    let expected = expected_file_len(plaintext_size, HEADER_LEN).ok_or_else(|| {
        anyhow!("computed expected file length overflowed")
    })?;
    if file_len != expected {
        bail!("file size mismatch: expected {expected} bytes, got {file_len}");
    }

    // Derive key.
    let key = derive_key(&password_bytes, &salt)?;

    // Create destination dir and unique temp file.
    let dest_dir = dest.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dest_dir).context("create dest dir")?;
    let mut tmp = UniqueTemp::new(dest_dir, "dec")?;

    // Stream-decrypt using DecryptorBE32, AAD = header.
    // Process all but the last frame with decrypt_next, then decrypt_last.
    let mut total_decoded = 0u64;
    {
        let cipher = Aes256Gcm::new_from_slice(&*key).expect("key length is 32");
        let stream_nonce_arr = GenericArray::from_slice(&stream_nonce);
        let mut decryptor = DecryptorBE32::from_aead(cipher, stream_nonce_arr);
        let out_file = tmp._file.as_mut().unwrap();
        let mut buf = [0u8; BLOCK_SIZE + TAG_LEN]; // ciphertext block max

        // Ceiling division with checked arithmetic.
        let frame_count = if plaintext_size == 0 {
            1
        } else {
            let pc = plaintext_size as usize;
            pc.div_ceil(BLOCK_SIZE)
        };

        // Full frames (not the last one).
        for frame_idx in 0..frame_count.saturating_sub(1) {
            let remaining = plaintext_size - total_decoded;
            let plain_len = std::cmp::min(BLOCK_SIZE, remaining as usize);
            let cipher_len = plain_len + TAG_LEN;

            f.read_exact(&mut buf[..cipher_len])
                .context("read encrypted frame")?;

            let payload = Payload {
                msg: &buf[..cipher_len],
                aad: &header,
            };

            let decrypted = decryptor
                .decrypt_next(payload)
                .map_err(|e| anyhow!("decrypt block {frame_idx}: {e}"))?;

            out_file.write_all(&decrypted).context("write decrypted")?;
            total_decoded += decrypted.len() as u64;
        }

        // Last frame.
        let remaining = plaintext_size - total_decoded;
        let plain_len = remaining as usize;
        let cipher_len = plain_len + TAG_LEN;

        f.read_exact(&mut buf[..cipher_len])
            .context("read last encrypted frame")?;

        let payload = Payload {
            msg: &buf[..cipher_len],
            aad: &header,
        };

        let decrypted = decryptor
            .decrypt_last(payload)
            .map_err(|e| anyhow!("finalize decryption: {e}"))?;

        out_file.write_all(&decrypted).context("write final decrypted")?;
        total_decoded += decrypted.len() as u64;
        out_file.flush().context("flush decrypted")?;
        out_file.sync_all().context("sync_all decrypted")?;
        let _ = out_file;
    }

    // Atomic rename: release handle, rename, disarm guard.
    // If rename fails, Drop cleans up the decrypted temp.
    let tmp_path_owned = tmp.release()?;
    std::fs::rename(&tmp_path_owned, dest).context("rename decrypted to dest")?;
    tmp.disarm(); // rename succeeded — stop Drop from deleting dest.
    Ok(total_decoded)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lv_enc2_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ── Derive key ───────────────────────────────────────────────────────

    #[test]
    fn derive_key_deterministic() {
        let password = b"test-password";
        let salt = [1u8; SALT_LEN];
        let k1 = derive_key(password, &salt).unwrap();
        let k2 = derive_key(password, &salt).unwrap();
        assert_eq!(&*k1, &*k2);
        assert_eq!(k1.len(), KEY_LEN);
    }

    #[test]
    fn derive_key_different_salt_different_key() {
        let password = b"test-password";
        let salt1 = [1u8; SALT_LEN];
        let salt2 = [2u8; SALT_LEN];
        let k1 = derive_key(password, &salt1).unwrap();
        let k2 = derive_key(password, &salt2).unwrap();
        assert_ne!(&*k1, &*k2);
    }

    // ── Round-trip ───────────────────────────────────────────────────────

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let data = scratch("roundtrip");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04fake zip content for testing purposes only";
        let password = "my-secret-password";

        let size = encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();
        assert!(size > 0);

        // Must be detected as encrypted.
        assert!(is_encrypted(&dest));

        // Decrypt.
        let out = data.join("decrypted.luabak");
        let dec_size = decrypt_import(&dest, &out, password).unwrap();
        assert_eq!(dec_size, zip_content.len() as u64);

        let decrypted = std::fs::read(&out).unwrap();
        assert_eq!(decrypted, zip_content);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn decrypt_import_rejects_an_unknown_magic_even_when_the_archive_is_valid() {
        let data = scratch("legacy_magic");
        let source = data.join("legacy-magic.luabak");
        let out = data.join("decrypted.luabak");
        let content = b"PK\x03\x04foreign encrypted backup";
        let password = "password";
        let magic = &b"FOOBAR-v9"[..];
        let salt = [7u8; SALT_LEN];
        let stream_nonce = [9u8; STREAM_NONCE_LEN];
        let mut header = vec![0u8; magic.len() + HEADER_PAYLOAD_LEN];
        header[..magic.len()].copy_from_slice(magic);
        header[magic.len()..magic.len() + 4].copy_from_slice(&VERSION.to_le_bytes());
        let offset = magic.len() + 4;
        header[offset..offset + 4].copy_from_slice(&ARGON2_MEM.to_le_bytes());
        header[offset + 4..offset + 8].copy_from_slice(&ARGON2_ITERS.to_le_bytes());
        header[offset + 8..offset + 12].copy_from_slice(&ARGON2_PAR.to_le_bytes());
        header[offset + 12..offset + 12 + SALT_LEN].copy_from_slice(&salt);
        let nonce_offset = offset + 12 + SALT_LEN;
        header[nonce_offset..nonce_offset + STREAM_NONCE_LEN].copy_from_slice(&stream_nonce);
        header[nonce_offset + STREAM_NONCE_LEN..nonce_offset + STREAM_NONCE_LEN + 8]
            .copy_from_slice(&(content.len() as u64).to_le_bytes());

        let password_bytes = Zeroizing::new(password.as_bytes().to_vec());
        let key = derive_key(&password_bytes, &salt).unwrap();
        let cipher = Aes256Gcm::new_from_slice(&*key).unwrap();
        let encryptor = EncryptorBE32::from_aead(cipher, GenericArray::from_slice(&stream_nonce));
        let ciphertext = encryptor
            .encrypt_last(Payload { msg: content, aad: &header })
            .unwrap();
        header.extend_from_slice(&ciphertext);
        std::fs::write(&source, header).unwrap();

        assert!(!is_encrypted(&source));
        let error = decrypt_import(&source, &out, password).unwrap_err();
        assert!(error.to_string().contains("not an LuaVault encrypted archive"));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn encrypt_decrypt_zero_length() {
        let data = scratch("zero");
        let dest = data.join("backup.luabak");

        let zip_content: &[u8] = &[];
        let password = "pw";

        let size = encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();
        assert!(size > 0);
        assert!(is_encrypted(&dest));

        let out = data.join("decrypted.luabak");
        let dec_size = decrypt_import(&dest, &out, password).unwrap();
        assert_eq!(dec_size, 0);

        let decrypted = std::fs::read(&out).unwrap();
        assert!(decrypted.is_empty());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn encrypt_decrypt_exact_block_size() {
        let data = scratch("exact_block");
        let dest = data.join("backup.luabak");

        let zip_content = vec![0xAB; BLOCK_SIZE];
        let password = "pw";

        let size = encrypt_export(
            Box::new(std::io::Cursor::new(zip_content.clone())),
            &dest,
            Some(password),
        )
        .unwrap();
        assert!(size > 0);
        assert!(is_encrypted(&dest));

        let out = data.join("decrypted.luabak");
        let dec_size = decrypt_import(&dest, &out, password).unwrap();
        assert_eq!(dec_size, BLOCK_SIZE as u64);

        let decrypted = std::fs::read(&out).unwrap();
        assert_eq!(decrypted, zip_content);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn encrypt_decrypt_multi_block() {
        let data = scratch("multi_block");
        let dest = data.join("backup.luabak");

        let zip_content = vec![0xCD; BLOCK_SIZE * 3 + 17_000];
        let password = "pw";

        let size = encrypt_export(
            Box::new(std::io::Cursor::new(zip_content.clone())),
            &dest,
            Some(password),
        )
        .unwrap();
        assert!(size > 0);

        let out = data.join("decrypted.luabak");
        let dec_size = decrypt_import(&dest, &out, password).unwrap();
        assert_eq!(dec_size, zip_content.len() as u64);

        let decrypted = std::fs::read(&out).unwrap();
        assert_eq!(decrypted, zip_content);
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Non-determinism ──────────────────────────────────────────────────

    #[test]
    fn encrypt_non_deterministic_salt_nonce() {
        let data = scratch("nondet");
        let zip_content = b"PK\x03\x04same content";
        let password = "same-password";

        let dest1 = data.join("backup1.luabak");
        let dest2 = data.join("backup2.luabak");

        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest1,
            Some(password),
        )
        .unwrap();
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest2,
            Some(password),
        )
        .unwrap();

        let c1 = std::fs::read(&dest1).unwrap();
        let c2 = std::fs::read(&dest2).unwrap();
        assert_ne!(c1, c2);
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Wrong password ───────────────────────────────────────────────────

    #[test]
    fn decrypt_wrong_password() {
        let data = scratch("wrong_pw");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04correct zip";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some("correct"),
        )
        .unwrap();

        let out = data.join("decrypted.luabak");
        let result = decrypt_import(&dest, &out, "wrong");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("decrypt") || msg.contains("auth") || msg.contains("tag") || msg.contains("AAD"),
            "expected auth error, got: {msg}"
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Magic mutation ───────────────────────────────────────────────────

    #[test]
    fn decrypt_mutation_magic() {
        let data = scratch("mut_magic");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut magic test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        enc[0] ^= 0xFF;
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Version mutation ─────────────────────────────────────────────────

    #[test]
    fn decrypt_mutation_version() {
        let data = scratch("mut_ver");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut version test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        // Version is at offset 9 (after MAGIC).
        enc[9] ^= 0xFF;
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Argon2 memory mutation ───────────────────────────────────────────

    #[test]
    fn decrypt_mutation_argon2_memory() {
        let data = scratch("mut_mem");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut mem test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        // Argon2 memory is at offset 13 (MAGIC=9 + version=4).
        enc[13] ^= 0xFF;
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Argon2 iterations mutation ───────────────────────────────────────

    #[test]
    fn decrypt_mutation_argon2_iters() {
        let data = scratch("mut_iters");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut iters test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        // Argon2 iterations at offset 17.
        enc[17] ^= 0xFF;
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Argon2 parallelism mutation ──────────────────────────────────────

    #[test]
    fn decrypt_mutation_argon2_par() {
        let data = scratch("mut_par");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut par test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        // Argon2 parallelism at offset 21.
        enc[21] ^= 0xFF;
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Salt mutation ────────────────────────────────────────────────────

    #[test]
    fn decrypt_mutation_salt() {
        let data = scratch("mut_salt");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut salt test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        // Salt at offset 25 (MAGIC=9 + version=4 + mem=4 + iters=4 + par=4).
        enc[25] ^= 0xFF;
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Nonce mutation ───────────────────────────────────────────────────

    #[test]
    fn decrypt_mutation_nonce() {
        let data = scratch("mut_nonce");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut nonce test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        // Stream nonce at offset 57 (25 + 32 salt).
        enc[57] ^= 0xFF;
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Plaintext size mutation ──────────────────────────────────────────

    #[test]
    fn decrypt_mutation_plaintext_size() {
        let data = scratch("mut_size");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut size test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        // Plaintext size at offset 64 (57 + 7 nonce).
        enc[64] ^= 0xFF;
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── First encrypted block mutation ───────────────────────────────────

    #[test]
    fn decrypt_mutation_first_cipher_block() {
        let data = scratch("mut_first_block");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut first block test content here";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        // Ciphertext starts after the current 8-byte-magic header.
        if enc.len() > HEADER_LEN {
            enc[HEADER_LEN] ^= 0xFF;
        }
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Last encrypted block mutation ────────────────────────────────────

    #[test]
    fn decrypt_mutation_last_cipher_block() {
        let data = scratch("mut_last_block");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04mut last block test content here";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        // Last ciphertext byte is at file_len - TAG_LEN - 1.
        if enc.len() > TAG_LEN + 1 {
            let pos = enc.len() - TAG_LEN - 1;
            enc[pos] ^= 0xFF;
        }
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Truncation ───────────────────────────────────────────────────────

    #[test]
    fn decrypt_truncated() {
        let data = scratch("trunc");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04truncated test content";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let enc = std::fs::read(&dest).unwrap();
        let truncated = &enc[..enc.len() / 2];
        std::fs::write(&dest, truncated).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Extra byte at end ────────────────────────────────────────────────

    #[test]
    fn decrypt_extra_byte() {
        let data = scratch("extra_byte");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04extra byte test content";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        let mut enc = std::fs::read(&dest).unwrap();
        enc.push(0xFF);
        std::fs::write(&dest, &enc).unwrap();

        let out = data.join("decrypted.luabak");
        assert!(decrypt_import(&dest, &out, password).is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Destination preservation on error ────────────────────────────────

    #[test]
    fn decrypt_preserves_existing_dest_on_wrong_password() {
        let data = scratch("preserve_dest");
        let dest = data.join("decrypted.luabak");

        let zip_content = b"PK\x03\x04preserve dest test";
        let dest_path = data.join("backup.luabak");
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest_path,
            Some("correct"),
        )
        .unwrap();

        // Pre-create destination with known content.
        let original = b"ORIGINAL CONTENT";
        std::fs::write(&dest, original).unwrap();
        let before = std::fs::read(&dest).unwrap();

        let result = decrypt_import(&dest_path, &dest, "wrong");
        assert!(result.is_err());

        let after = std::fs::read(&dest).unwrap();
        assert_eq!(before, after, "destination must be byte-identical after failure");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn encrypt_preserves_existing_dest_on_error() {
        let data = scratch("preserve_dest_enc");
        let dest = data.join("backup.luabak");

        // Pre-create destination.
        let original = b"ORIGINAL ENCRYPTED";
        std::fs::write(&dest, original).unwrap();
        let before = std::fs::read(&dest).unwrap();

        // Encrypt with a valid password but force a failure by passing a reader
        // that fails mid-stream.
        struct FailingReader {
            pos: usize,
        }
        impl Read for FailingReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.pos == 0 {
                    self.pos += 1;
                    buf[..4].copy_from_slice(b"PK\x03\x04");
                    Ok(4)
                } else {
                    Err(std::io::Error::other("mid-stream failure"))
                }
            }
        }

        let result = encrypt_export(
            Box::new(FailingReader { pos: 0 }),
            &dest,
            Some("pw"),
        );
        assert!(result.is_err());

        let after = std::fs::read(&dest).unwrap();
        assert_eq!(before, after, "destination must be byte-identical after failure");
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── No residual temporaries ──────────────────────────────────────────

    #[test]
    fn no_residual_temporaries_on_success() {
        let data = scratch("no_tmp_success");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04no tmp test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(password),
        )
        .unwrap();

        // Check no .tmp files remain.
        let tmp_files: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(tmp_files.is_empty(), "no temp files should remain");

        // Source should not be deleted (it's a reader, not a file).
        assert!(dest.exists());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn no_residual_temporaries_on_decrypt_error() {
        let data = scratch("no_tmp_error");
        let src = data.join("backup.luabak");
        let dest = data.join("decrypted.luabak");

        let zip_content = b"PK\x03\x04no tmp error test";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &src,
            Some("correct"),
        )
        .unwrap();

        let result = decrypt_import(&src, &dest, "wrong");
        assert!(result.is_err());

        let tmp_files: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(tmp_files.is_empty(), "no temp files should remain after error");

        // Source must still exist.
        assert!(src.exists());
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Source preservation ──────────────────────────────────────────────

    #[test]
    fn decrypt_preserves_source() {
        let data = scratch("preserve_src");
        let src = data.join("backup.luabak");
        let dest = data.join("decrypted.luabak");

        let zip_content = b"PK\x03\x04preserve source test";
        let password = "pw";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &src,
            Some(password),
        )
        .unwrap();

        let src_before = std::fs::read(&src).unwrap();

        // Successful decrypt.
        decrypt_import(&src, &dest, password).unwrap();
        let src_after = std::fs::read(&src).unwrap();
        assert_eq!(src_before, src_after, "source must be unchanged after success");

        // Failed decrypt.
        let _ = std::fs::remove_file(&dest);
        let result = decrypt_import(&src, &dest, "wrong");
        assert!(result.is_err());
        let src_after_err = std::fs::read(&src).unwrap();
        assert_eq!(src_before, src_after_err, "source must be unchanged after failure");

        let _ = std::fs::remove_dir_all(&data);
    }

    // ── is_encrypted ─────────────────────────────────────────────────────

    #[test]
    fn is_encrypted_returns_true_for_valid() {
        let data = scratch("is_enc_true");
        let path = data.join("backup.luabak");

        encrypt_export(
            Box::new(std::io::Cursor::new(b"PK\x03\x04test")),
            &path,
            Some("pw"),
        )
        .unwrap();

        assert!(is_encrypted(&path));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn is_encrypted_returns_false_for_plain_zip() {
        let data = scratch("is_enc_false");
        let path = data.join("plain.luabak");
        std::fs::write(&path, b"PK\x03\x04plain zip").unwrap();
        assert!(!is_encrypted(&path));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn is_encrypted_returns_false_for_short_file() {
        let data = scratch("is_enc_short");
        let path = data.join("short.luabak");
        std::fs::write(&path, b"LVBC").unwrap();
        assert!(!is_encrypted(&path));
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn is_encrypted_returns_false_for_nonexistent() {
        let path = PathBuf::from("/nonexistent/file.luabak");
        assert!(!is_encrypted(&path));
    }

    // ── No plaintext passthrough ──────────────────────────────────────────

    #[test]
    fn encrypt_rejects_empty_password() {
        let data = scratch("no_pt_empty");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04unencrypted";

        let result = encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some(""),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("empty") || msg.contains("password"), "error should mention empty password: {msg}");
    }

    #[test]
    fn encrypt_rejects_none_password() {
        let data = scratch("no_pt_none");
        let dest = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04unencrypted";

        let result = encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            None,
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("password") || msg.contains("required"), "error should mention password required: {msg}");
    }

    #[test]
    fn decrypt_rejects_empty_password() {
        let data = scratch("no_pt_dec");
        let src = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04encrypted";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &src,
            Some("pw"),
        )
        .unwrap();

        let dest = data.join("decrypted.luabak");
        let result = decrypt_import(&src, &dest, "");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("empty") || msg.contains("password"), "error should mention empty password: {msg}");
    }

    #[test]
    fn decrypt_rejects_plaintext_file() {
        let data = scratch("no_pt_plain");
        let src = data.join("plain.luabak");
        std::fs::write(&src, b"PK\x03\x04plain zip").unwrap();

        let dest = data.join("decrypted.luabak");
        let result = decrypt_import(&src, &dest, "pw");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("encrypted") || msg.contains("plaintext") || msg.contains("passthrough"),
            "error should mention encrypted/plaintext/passthrough: {msg}"
        );
    }

    #[test]
    fn decrypt_refuses_src_eq_dest() {
        let data = scratch("src_eq_dest");
        let path = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04test";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &path,
            Some("pw"),
        )
        .unwrap();

        let result = decrypt_import(&path, &path, "pw");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("same") || msg.contains("source") || msg.contains("dest"), "error should mention same source/dest: {msg}");
    }

    #[test]
    fn src_eq_dest_source_unchanged() {
        let data = scratch("src_eq_preserved");
        let path = data.join("backup.luabak");

        let zip_content = b"PK\x03\x04test src eq dest preserved";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &path,
            Some("pw"),
        )
        .unwrap();

        let before = std::fs::read(&path).unwrap();
        let result = decrypt_import(&path, &path, "pw");
        assert!(result.is_err());
        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "source must be byte-identical after src==dest rejection");
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Unique temporaries ───────────────────────────────────────────────

    #[test]
    fn preexisting_fixed_names_unchanged() {
        // Create files with the old fixed names (e.g. backup.enc_zip.tmp).
        // They should not be touched by the new unique-temp code.
        let data = scratch("preexist_tmp");
        let dest = data.join("backup.luabak");

        // Precreate old-style names with sentinel content.
        let sentinel = b"SENTINEL_CONTENT";
        let old_tmp = data.join("backup.enc_zip.tmp");
        std::fs::write(&old_tmp, sentinel).unwrap();
        let before = std::fs::read(&old_tmp).unwrap();

        // Run encryption.
        let zip_content = b"PK\x03\x04test";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest,
            Some("pw"),
        )
        .unwrap();

        // Old name must still exist with same content.
        let after = std::fs::read(&old_tmp).unwrap();
        assert_eq!(before, after, "preexisting fixed-name temp must be unchanged");
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn two_operations_no_shared_temp() {
        // Two sequential encryptions into the SAME dest stem must not
        // leave any temp behind, and must not collide.
        let data = scratch("same_stem");
        let dest = data.join("backup.luabak");

        encrypt_export(
            Box::new(std::io::Cursor::new(b"PK\x03\x04first into same dest")),
            &dest,
            Some("pw"),
        )
        .unwrap();
        assert!(dest.exists());

        encrypt_export(
            Box::new(std::io::Cursor::new(b"PK\x03\x04second into same dest")),
            &dest,
            Some("pw"),
        )
        .unwrap();
        assert!(dest.exists());

        // Both runs must have produced a valid encrypted file.
        assert!(is_encrypted(&dest));

        // No .tmp files remain.
        let tmp_files: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(tmp_files.is_empty(), "no temp files should remain after two ops on same stem");
        let _ = std::fs::remove_dir_all(&data);
    }

    // ── Real publish-failure tests (section 2 of the brief) ──────────────

    #[test]
    fn decrypt_publish_failure_leaves_no_plaintext_temp() {
        // dest is an existing directory → rename into it fails.
        // The decrypted temp must be cleaned up and no dec_*.tmp should remain.
        let data = scratch("dec_pub_fail");
        let src = data.join("backup.luabak");
        let dest_dir = data.join("decrypted_dir");
        std::fs::create_dir_all(&dest_dir).unwrap();

        let zip_content = b"PK\x03\x04test decrypt publish failure";
        encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &src,
            Some("pw"),
        )
        .unwrap();

        // Snapshot source before.
        let src_before = std::fs::read(&src).unwrap();
        let dest_dir_before = std::fs::read_dir(&dest_dir).unwrap().count();

        // Attempt decrypt with a directory as dest — rename will fail.
        let result = decrypt_import(&src, &dest_dir, "pw");
        assert!(result.is_err(), "decrypt into existing dir must fail");

        // Source must be byte-identical.
        let src_after = std::fs::read(&src).unwrap();
        assert_eq!(src_before, src_after, "source must be unchanged");

        // Dest dir must be intact (same contents).
        let dest_dir_after = std::fs::read_dir(&dest_dir).unwrap().count();
        assert_eq!(dest_dir_before, dest_dir_after, "dest dir must be intact");

        // No dec_*.tmp or other temp files should remain.
        let all_files: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        let tmp_files: Vec<_> = all_files
            .iter()
            .filter(|f| f.ends_with(".tmp"))
            .collect();
        assert!(tmp_files.is_empty(), "no temp files should remain after publish failure: {all_files:?}");

        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn encrypt_publish_failure_leaves_no_temp() {
        // dest is an existing directory → rename fails.
        // No zip_*, enc_*, final_* or other temp should remain.
        let data = scratch("enc_pub_fail");
        let dest_dir = data.join("backup_dir");
        std::fs::create_dir_all(&dest_dir).unwrap();

        let zip_content = b"PK\x03\x04test encrypt publish failure";
        let result = encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest_dir,
            Some("pw"),
        );
        assert!(result.is_err(), "encrypt into existing dir must fail");

        // Dest dir must be intact.
        let dir_contents: Vec<_> = std::fs::read_dir(&dest_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(dir_contents.len(), 0, "dest dir must be empty");

        // No .tmp files should remain.
        let all_files: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        let tmp_files: Vec<_> = all_files
            .iter()
            .filter(|f| f.ends_with(".tmp"))
            .collect();
        assert!(tmp_files.is_empty(), "no temp files should remain after encrypt publish failure: {all_files:?}");

        let _ = std::fs::remove_dir_all(&data);
    }

    // ── KDF param strict validation ──────────────────────────────────────

    #[test]
    fn kdf_params_rejects_zero() {
        let result = validate_kdf_params(0, 0, 0);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("memory") || msg.contains("iterations") || msg.contains("parallelism"), "error should mention the param: {msg}");
    }

    #[test]
    fn kdf_params_rejects_below_constant_memory() {
        let result = validate_kdf_params(ARGON2_MEM - 1, ARGON2_ITERS, ARGON2_PAR);
        assert!(result.is_err());
    }

    #[test]
    fn kdf_params_rejects_below_constant_iters() {
        let result = validate_kdf_params(ARGON2_MEM, ARGON2_ITERS - 1, ARGON2_PAR);
        assert!(result.is_err());
    }

    #[test]
    fn kdf_params_rejects_below_constant_par() {
        let result = validate_kdf_params(ARGON2_MEM, ARGON2_ITERS, ARGON2_PAR - 1);
        assert!(result.is_err());
    }

    #[test]
    fn kdf_params_rejects_above_constant_memory() {
        let result = validate_kdf_params(ARGON2_MEM + 1, ARGON2_ITERS, ARGON2_PAR);
        assert!(result.is_err());
    }

    #[test]
    fn kdf_params_rejects_above_constant_iters() {
        let result = validate_kdf_params(ARGON2_MEM, ARGON2_ITERS + 1, ARGON2_PAR);
        assert!(result.is_err());
    }

    #[test]
    fn kdf_params_rejects_above_constant_par() {
        let result = validate_kdf_params(ARGON2_MEM, ARGON2_ITERS, ARGON2_PAR + 1);
        assert!(result.is_err());
    }

    #[test]
    fn kdf_params_accepts_exact_values() {
        let result = validate_kdf_params(ARGON2_MEM, ARGON2_ITERS, ARGON2_PAR);
        assert!(result.is_ok());
    }

    /// Le coût d'une dérivation, mesuré plutôt que supposé.
    ///
    /// Les paramètres Argon2id ne se jugent pas dans l'absolu : ils sont un
    /// compromis entre ce que coûte une tentative à l'attaquant et ce qu'elle
    /// coûte à l'utilisateur qui tape son mot de passe. Sans le second chiffre,
    /// on ne peut ni défendre les valeurs actuelles ni justifier de les monter.
    ///
    /// Ignoré par défaut : c'est une mesure, pas une assertion — la borne haute
    /// est volontairement large, une machine chargée ou un CI lent ne doivent
    /// pas rendre la suite rouge.
    ///
    /// ```text
    /// cd src-tauri && cargo test --lib argon2_derivation_cost -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "mesure de performance, pas une garde"]
    fn argon2_derivation_cost() {
        let salt = [7u8; SALT_LEN];
        let started = std::time::Instant::now();
        let rounds = 5;
        for _ in 0..rounds {
            derive_key(b"une phrase secrete de longueur realiste", &salt).unwrap();
        }
        let each = started.elapsed() / rounds;
        println!(
            "Argon2id m={} KiB, t={}, p={} → {:?} par dérivation",
            ARGON2_MEM, ARGON2_ITERS, ARGON2_PAR, each
        );
        assert!(
            each < std::time::Duration::from_secs(5),
            "une dérivation ne doit pas dépasser 5 s, mesuré {each:?}"
        );
    }

    // ── expected_file_len ────────────────────────────────────────────────

    #[test]
    fn expected_file_len_zero_plaintext() {
        // Zero plaintext → 1 frame.
        let expected = HEADER_LEN as u64 + TAG_LEN as u64;
        assert_eq!(expected_file_len(0, HEADER_LEN), Some(expected));
    }

    #[test]
    fn expected_file_len_exact_block() {
        let expected = HEADER_LEN as u64 + BLOCK_SIZE as u64 + TAG_LEN as u64;
        assert_eq!(expected_file_len(BLOCK_SIZE as u64, HEADER_LEN), Some(expected));
    }

    #[test]
    fn expected_file_len_two_blocks() {
        let plaintext = BLOCK_SIZE as u64 + 1;
        let expected = HEADER_LEN as u64 + plaintext + 2 * TAG_LEN as u64;
        assert_eq!(expected_file_len(plaintext, HEADER_LEN), Some(expected));
    }

    #[test]
    fn expected_file_len_one_byte() {
        // 1 byte → 1 frame.
        let expected = HEADER_LEN as u64 + 1 + TAG_LEN as u64;
        assert_eq!(expected_file_len(1, HEADER_LEN), Some(expected));
    }

    #[test]
    fn expected_file_len_block_minus_one() {
        // BLOCK_SIZE - 1 → 1 frame.
        let plaintext = (BLOCK_SIZE - 1) as u64;
        let expected = HEADER_LEN as u64 + plaintext + TAG_LEN as u64;
        assert_eq!(expected_file_len(plaintext, HEADER_LEN), Some(expected));
    }

    #[test]
    fn expected_file_len_block_plus_one() {
        // BLOCK_SIZE + 1 → 2 frames.
        let plaintext = (BLOCK_SIZE + 1) as u64;
        let expected = HEADER_LEN as u64 + plaintext + 2 * TAG_LEN as u64;
        assert_eq!(expected_file_len(plaintext, HEADER_LEN), Some(expected));
    }

    #[test]
    fn expected_file_len_max_supported() {
        // 1 GiB → should compute without overflow.
        let plaintext = 1024 * 1024 * 1024;
        let result = expected_file_len(plaintext, HEADER_LEN);
        assert!(result.is_some(), "should handle 1 GiB without overflow");
    }

    // ── Boundary: src == dest for encrypt ──────────────────────────────────

    #[test]
    fn encrypt_export_reader_into_existing_dir_fails() {
        // encrypt_export receives a Box<dyn Read>, not a path, so it
        // cannot detect src == dest.  The only general way to force a
        // failure is to make dest a directory (rename into it fails).
        let data = scratch("enc_into_dir");
        let dest_dir = data.join("target_dir");
        std::fs::create_dir_all(&dest_dir).unwrap();

        let zip_content = b"PK\x03\x04encrypt into dir";
        let result = encrypt_export(
            Box::new(std::io::Cursor::new(zip_content)),
            &dest_dir,
            Some("pw"),
        );
        assert!(result.is_err(), "encrypt into existing dir must fail");

        // No .tmp files should remain.
        let tmp_files: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(tmp_files.is_empty(), "no temp files should remain");

        let _ = std::fs::remove_dir_all(&data);
    }

}

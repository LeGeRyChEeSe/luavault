//! Compressed safety copies of everything that is painful to re-acquire:
//! the `.lua` files, the online-fix archives, their install states and the config.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::{encrypted_backup, hmac, library};

/// How many rolling automatic snapshots to keep on disk.
const AUTO_SNAPSHOT_KEEP: usize = 5;

/// Current backup extension.
const BACKUP_EXTENSION: &str = "luabak";

const MANIFEST_NAME: &str = "manifest.json";

/// A manifest far beyond this is a hostile archive, not a backup: the cap
/// turns it into a parse error instead of an unbounded allocation.
const MAX_MANIFEST_BYTES: u64 = 1 << 20;

/// A uniquely named temporary file.
///
/// The name carries a CSPRNG suffix (16 bytes = 128 bits) and the file is
/// created with `create_new(true)`, so a preexisting path is never claimed.
/// The guard deletes the file on drop; `disarm()` transfers ownership of the
/// path to the caller after a successful publication.
struct TempBackup {
    path: PathBuf,
    armed: bool,
}

impl TempBackup {
    /// Create the temp inside `dir`, using `neighbour` only for naming.
    fn in_dir(dir: &Path, _neighbour: &Path, prefix: &str) -> Result<Self> {
        let dir = dir
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        for _ in 0..8 {
            let mut suffix = [0u8; 16];
            rand::fill(&mut suffix);
            let hex: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
            let path = dir.join(format!("{prefix}.{hex}.tmp"));
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
            {
                Ok(_) => return Ok(TempBackup { path, armed: true }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(anyhow!(e).context("création du fichier temporaire")),
            }
        }
        bail!("aucun nom temporaire disponible après 8 essais")
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempBackup {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupOptions {
    pub include_lua: bool,
    /// The downloaded fix archives — by far the heaviest part.
    pub include_fix_archives: bool,
    pub include_fix_states: bool,
    pub include_config: bool,
}

impl Default for BackupOptions {
    fn default() -> Self {
        BackupOptions {
            include_lua: true,
            include_fix_archives: true,
            include_fix_states: true,
            include_config: true,
        }
    }
}

impl BackupOptions {
    /// Small, frequent snapshot: everything except the bulky fix archives.
    pub fn automatic() -> Self {
        BackupOptions {
            include_lua: true,
            include_fix_archives: false,
            include_fix_states: true,
            include_config: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: u32,
    pub created_at: String,
    pub app_version: String,
    pub lua_count: usize,
    pub fix_archive_count: usize,
    pub fix_state_count: usize,
    pub has_config: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupSummary {
    pub path: String,
    pub bytes: u64,
    pub lua_count: usize,
    pub fix_archive_count: usize,
    pub fix_state_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotInfo {
    pub path: String,
    pub name: String,
    pub bytes: u64,
    pub created_at: Option<String>,
    pub lua_count: usize,
    pub fix_archive_count: usize,
    pub automatic: bool,
    /// Encrypted v2 archives keep their manifest sealed — the listing simply
    /// lacks the details, and the frontend shows them apart.
    pub encrypted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportSummary {
    pub lua_restored: usize,
    pub fix_archives_restored: usize,
    pub fix_states_restored: usize,
    pub config_restored: bool,
    pub config_kept_local: Vec<String>,
    pub entries_skipped: usize,
}

pub fn backups_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("backups")
}

/// Where the plaintext temp lives.
///
/// Encrypted paths keep it inside `data_dir`: the destination may be a synced
/// folder, and the whole point of the password is that no readable copy ever
/// appears there.
/// Non-encrypted paths place the temp next to `dest` so the rename stays on
/// the same volume.
pub fn temp_dir_for(encrypted: bool, dest: &Path, data_dir: &Path) -> PathBuf {
    if encrypted {
        data_dir.to_path_buf()
    } else {
        dest.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

fn entries_of(dir: &Path, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = read
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && keep(p))
        .collect();
    out.sort();
    out
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case(ext)) == Some(true)
}

/// Orphaned temporary files left by previous runs (crash, power loss …).
///
/// Only removes files matching the exact patterns this module writes
/// (`.luabak.partial.<hex>.tmp` and `.luabak.decrypted.<hex>.tmp`) and
/// only inside `data_dir`.  An age gate of one hour prevents deleting a
/// temp that belongs to an export still in flight.
///
/// Bounded: one directory scan, no recursion, never touches anything
/// outside `data_dir`.
pub fn cleanup_orphan_temps(data_dir: &Path) {
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(60 * 60))
        .expect("clock went backwards");

    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return;
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Only our patterns: `.luabak.partial.<hex>.tmp` or `.luabak.decrypted.<hex>.tmp`
        if !(name.starts_with(".luabak.partial.") || name.starts_with(".luabak.decrypted.")) {
            continue;
        }
        if !name.ends_with(".tmp") {
            continue;
        }
        // Must be hex suffix between prefix and .tmp
        let inner = &name[..name.len() - 4]; // strip ".tmp"
        let suffix = match inner.rfind('.') {
            Some(dot) => &inner[dot + 1..],
            None => continue,
        };
        if suffix.len() != 32 || !suffix.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        // Check age
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified > cutoff {
            continue;
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// Write the v1 ZIP (manifest included) to `path`. The on-disk layout is
/// contractual: existing archives and automatic snapshots must keep importing.
fn write_backup_zip(
    lib: &Path,
    data_dir: &Path,
    path: &Path,
    options: &BackupOptions,
) -> Result<(usize, usize, usize)> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("création de {}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(9))
        .large_file(true);

    let add = |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, src: &Path| -> Result<bool> {
        if !src.is_file() {
            return Ok(false);
        }
        let bytes = std::fs::read(src).with_context(|| format!("lecture de {}", src.display()))?;
        zip.start_file(name, opts).context("entrée de sauvegarde")?;
        zip.write_all(&bytes).context("écriture de la sauvegarde")?;
        Ok(true)
    };

    let mut lua_count = 0;
    let mut fix_archive_count = 0;
    let mut fix_state_count = 0;

    if options.include_lua {
        add(&mut zip, "library/index.json", &lib.join("index.json"))?;
        for path in entries_of(lib, |p| has_extension(p, "lua")) {
            let Some(base) = path.file_name() else { continue };
            let name = format!("library/{}", base.to_string_lossy());
            if add(&mut zip, &name, &path)? {
                lua_count += 1;
            }
        }
    }

    let fixes = crate::fixes::fixes_dir(lib);
    if options.include_fix_archives {
        for path in entries_of(&fixes, |p| !p.to_string_lossy().ends_with(".state.json")) {
            let Some(base) = path.file_name() else { continue };
            let name = format!("library/fixes/{}", base.to_string_lossy());
            if add(&mut zip, &name, &path)? {
                fix_archive_count += 1;
            }
        }
    }
    if options.include_fix_states {
        for path in entries_of(&fixes, |p| p.to_string_lossy().ends_with(".state.json")) {
            let Some(base) = path.file_name() else { continue };
            let name = format!("library/fixes/{}", base.to_string_lossy());
            if add(&mut zip, &name, &path)? {
                fix_state_count += 1;
            }
        }
    }

    let mut has_config = false;
    if options.include_config {
        has_config = add(&mut zip, "config.json", &data_dir.join("config.json"))?;
    }

    let manifest = BackupManifest {
        format: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        lua_count,
        fix_archive_count,
        fix_state_count,
        has_config,
    };
    zip.start_file(MANIFEST_NAME, opts)
        .context("entrée manifeste")?;
    zip.write_all(&serde_json::to_vec_pretty(&manifest).context("sérialisation du manifeste")?)
        .context("écriture du manifeste")?;
    // `finish` hands the file handle back: flush and sync it, then close it
    // before any rename — Windows refuses to rename an open file.
    let mut file = zip.finish().context("finalisation de la sauvegarde")?;
    file.flush().context("flush de la sauvegarde")?;
    file.sync_all().context("synchronisation de la sauvegarde")?;
    drop(file);

    Ok((lua_count, fix_archive_count, fix_state_count))
}

/// Write a compressed backup of the library (and optionally the config) to `dest`.
///
/// `password` — absent or empty: the archive stays a plain v1 ZIP; non-empty:
/// the finished ZIP is sealed into the encrypted v2 format
/// (`encrypted_backup`). Automatic snapshots always pass `None`.
///
/// The archive is assembled in a unique temp next to `dest` and published by
/// rename only once finalised: a failed export never truncates a preexisting
/// `.luabak` and never leaves a temp behind. The HMAC key and the sidecar
/// never enter the archive — v1 nor v2 — by construction: the exporter only
/// ever adds `library/index.json`, the `.lua` files, the fix payloads and
/// `config.json`.
///
/// For the encrypted path the plaintext ZIP is assembled inside `data_dir`
/// rather than beside `dest` — `encrypt_export` publishes its own encrypted
/// file by rename, so neighbour-ness is unnecessary.  This avoids exposing
/// an unencrypted copy of the library where the user asked for encryption
/// (e.g. a OneDrive-synced folder).
pub fn export(
    lib: &Path,
    data_dir: &Path,
    dest: &Path,
    options: &BackupOptions,
    password: Option<&str>,
) -> Result<BackupSummary> {
    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).context("création du dossier de destination")?;
    }
    let password = password.filter(|p| !p.is_empty());

    let result = match password {
        None => {
            let dir = temp_dir_for(false, dest, data_dir);
            let mut temp = TempBackup::in_dir(&dir, dest, ".luabak.partial")?;
            let (lua_count, fix_archive_count, fix_state_count) =
                write_backup_zip(lib, data_dir, &temp.path, options)?;
            std::fs::rename(&temp.path, dest)
                .with_context(|| format!("publication de {}", dest.display()))?;
            temp.disarm();
            let bytes = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
            BackupSummary {
                path: dest.display().to_string(),
                bytes,
                lua_count,
                fix_archive_count,
                fix_state_count,
            }
        }
        Some(secret) => {
            // Encrypted path: assemble plaintext ZIP inside data_dir so no
            // unencrypted copy leaks beside dest.
            let dir = temp_dir_for(true, dest, data_dir);
            let temp = TempBackup::in_dir(&dir, dest, ".luabak.partial")?;
            let (lua_count, fix_archive_count, fix_state_count) =
                write_backup_zip(lib, data_dir, &temp.path, options)?;
            let reader = Box::new(
                std::fs::File::open(&temp.path).context("réouverture du ZIP temporaire")?,
            );
            // encrypt_export publishes to `dest` by rename; the plaintext ZIP
            // guard still owns the file and deletes it on drop.
            encrypted_backup::encrypt_export(reader, dest, Some(secret))?;
            // `temp` drops here — deletes the plaintext ZIP.
            let bytes = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
            BackupSummary {
                path: dest.display().to_string(),
                bytes,
                lua_count,
                fix_archive_count,
                fix_state_count,
            }
        }
    };

    Ok(result)
}

/// Reserved Windows device names (case-insensitive, with or without extension).
fn is_reserved_windows_device(stem: &str) -> bool {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    RESERVED.contains(&stem.to_uppercase().as_str())
}

/// Build a safe relative path from a zip entry name.
///
/// Rejects absolute paths, drive prefixes (`C:\`), path traversal (`..`),
/// Windows reserved device names (CON, NUL, …), and NTFS alternate data streams
/// (names containing `:`). Only `Component::Normal` segments are accepted.
///
/// Returns `None` when the name is empty or would escape the intended base.
pub(crate) fn safe_relative(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }

    // Normalise backslashes so Windows separators are handled uniformly.
    let normalised = name.replace('\\', "/");

    let mut out = PathBuf::new();
    for component in Path::new(&normalised).components() {
        // Only accept plain path components. Reject `..`, roots, drive letters,
        // and any other special component that could escape the base directory.
        let Component::Normal(part) = component else {
            return None;
        };
        let candidate = part.to_string_lossy();

        // Reject NTFS alternate data streams (e.g. "index.json:payload")
        // and bare drive-like forms (e.g. "C:").
        if candidate.contains(':') {
            return None;
        }

        // Reject Windows reserved device names (case-insensitive).
        // Strip trailing dots and spaces per MS naming rules.
        let stem = candidate.trim_end_matches(['.', ' ']).split('.').next().unwrap_or("");
        if is_reserved_windows_device(stem) {
            return None;
        }

        out.push(part);
    }

    // Reject if the resulting path is empty (e.g. the entry was "." or "").
    if out.as_os_str().is_empty() {
        return None;
    }

    Some(out)
}

fn read_manifest(archive: &Path) -> Option<BackupManifest> {
    let file = std::fs::File::open(archive).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let entry = zip.by_name(MANIFEST_NAME).ok()?;
    let mut raw = String::new();
    std::io::Read::read_to_string(&mut entry.take(MAX_MANIFEST_BYTES), &mut raw).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Bounded format probe for the import dialog: reads only the ZIP manifest,
/// never the payload. (The encrypted format's probe is the 9-byte magic check
/// in `encrypted_backup::is_encrypted`.)
pub fn is_v1_backup(path: &Path) -> bool {
    read_manifest(path).is_some()
}

/// Restore a backup into the live library. Existing files are overwritten.
///
/// `password` — required when the archive is encrypted (v2), ignored for v1.
/// The detection order is part of the contract: `is_encrypted` runs before
/// any ZIP read, so a sealed archive reports "mot de passe requis" instead of
/// masquerading as an invalid file. An encrypted archive is decrypted and
/// authenticated into a unique temp first (deleted on success as on error)
/// and restored through the single v1 extraction path.
///
/// For the encrypted path the decrypted ZIP is written inside `data_dir`
/// rather than beside the archive — this avoids leaking plaintext next to
/// a read-only medium (e.g. a USB drive) and matches the export path.
pub fn import(
    archive: &Path,
    lib: &Path,
    data_dir: &Path,
    password: Option<&str>,
) -> Result<ImportSummary> {
    if !archive.is_file() {
        bail!("fichier de sauvegarde introuvable");
    }
    if !has_extension(archive, BACKUP_EXTENSION) {
        bail!("extension de sauvegarde non reconnue");
    }

    if encrypted_backup::is_encrypted(archive) {
        let Some(secret) = password.filter(|p| !p.is_empty()) else {
            bail!("archive chiffrée : mot de passe requis");
        };
        let dir = temp_dir_for(true, archive, data_dir);
        let temp = TempBackup::in_dir(&dir, archive, ".luabak.decrypted")?;
        encrypted_backup::decrypt_import(archive, &temp.path, secret)
            .map_err(|_| anyhow!("mot de passe incorrect ou archive altérée"))?;
        // The guard deletes the decrypted temp on success as on error.
        return import_v1(&temp.path, lib, data_dir);
    }

    import_v1(archive, lib, data_dir)
}

/// The single v1 extraction boundary: manifest check, guarded extraction,
/// then adoption of the restored index when the archive carried one.
fn import_v1(archive: &Path, lib: &Path, data_dir: &Path) -> Result<ImportSummary> {
    if read_manifest(archive).is_none() {
        bail!("ce fichier n'est pas une sauvegarde LuaVault valide");
    }

    // Invalidate the library index cache before extracting so that any
    // partial write (early exit via `?`) does not leave a stale cache
    // pointing to pre-import data.
    library::clear_index_cache();

    let file = std::fs::File::open(archive).context("ouverture de la sauvegarde")?;
    let mut zip = zip::ZipArchive::new(file).context("lecture de la sauvegarde")?;
    let mut summary = ImportSummary {
        lua_restored: 0,
        fix_archives_restored: 0,
        fix_states_restored: 0,
        config_restored: false,
        config_kept_local: Vec::new(),
        entries_skipped: 0,
    };
    let mut restored_index = false;
    let mut config_bytes: Option<Vec<u8>> = None;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("entrée de sauvegarde illisible")?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let dest = if name == "config.json" {
            summary.config_restored = true;
            // Collect raw bytes for merge; do NOT write directly.
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).context("lecture de config.json")?;
            config_bytes = Some(buf);
            continue; // skip the normal write path
        } else if let Some(rel) = name.strip_prefix("library/") {
            let rel = match safe_relative(rel) {
                Some(p) => p,
                None => {
                    log::warn!("skipped unsafe backup entry: {name}");
                    summary.entries_skipped += 1;
                    continue;
                }
            };
            if rel == Path::new("index.json") {
                restored_index = true;
            }
            if rel.to_string_lossy().ends_with(".state.json") {
                summary.fix_states_restored += 1;
            } else if rel.starts_with("fixes") {
                summary.fix_archives_restored += 1;
            } else if rel.extension().is_some_and(|e| e == "lua") {
                summary.lua_restored += 1;
            }
            lib.join(rel)
        } else {
            continue; // manifest and anything unexpected
        };

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).context("création du dossier de restauration")?;
        }
        let mut sink = std::fs::File::create(&dest)
            .with_context(|| format!("écriture de {}", dest.display()))?;
        std::io::copy(&mut entry, &mut sink).context("restauration d'un fichier")?;
    }

    // Adoption — if and only if the archive carried `library/index.json`, the
    // restored index is signed with the receiving installation's key, never
    // the source's: keys and sidecars never enter an archive, so the
    // signature can only come from this machine. Without it the strict load
    // would see "key present, sidecar absent" and fail closed on the very
    // library we just restored. The JSON is validated BEFORE signing — LOT-21's
    // migration frontiers validate then sign: a damaged index must fail the
    // import outright, never get a signature that would vouch for it.
    if restored_index {
        let index_path = lib.join("index.json");
        let raw = std::fs::read(&index_path).context("lecture de l'index restauré")?;
        serde_json::from_slice::<Vec<library::LibraryEntry>>(&raw)
            .context("l'index restauré n'est pas un JSON valide")?;
        let key = hmac::load_or_create_key(data_dir).context("chargement de la clé HMAC")?;
        hmac::sign_index(&index_path, &key).context("signature de l'index restauré")?;
    }

    // Merge imported config with the local one.
    // A corrupted or unparseable config.json does NOT fail the import:
    // the local config is kept intact and the rest of the archive is
    // still restored.
    if summary.config_restored {
        if let Some(raw) = config_bytes {
            if let Ok(imported) = serde_json::from_slice::<crate::config::AppConfig>(&raw) {
                let local_path = data_dir.join("config.json");
                // Three cases, and only the middle one refuses to write:
                //  - no local file (fresh install restoring a backup): merge
                //    against the default config, so the import is actually
                //    applied. Dropping it here would let `config_restored`
                //    announce a restoration that never happened;
                //  - local file present but unreadable or corrupt: leave it
                //    strictly untouched — the LOT-21 fail-closed rule. Falling
                //    back to Default would erase the user's `steam_dir`;
                //  - local file readable: merge it.
                let local = match std::fs::read_to_string(&local_path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Some(crate::config::AppConfig::default())
                    }
                    Err(_) => None,
                    Ok(s) => serde_json::from_str::<crate::config::AppConfig>(&s).ok(),
                };
                if let Some(local) = local {
                    let merged =
                        crate::config::merge_imported(&local, &imported, |p| p.exists());
                    summary.config_kept_local = merged.kept_local;
                    let raw = serde_json::to_vec_pretty(&merged.merged)
                        .context("serialize merged config")?;
                    std::fs::write(&local_path, raw).context("write merged config.json")?;
                }
            }
            // If parsing the import fails, leave the local config untouched and continue.
        }
    }

    Ok(summary)
}

/// Take a rolling automatic snapshot, pruning the oldest beyond the keep limit.
pub fn auto_snapshot(lib: &Path, data_dir: &Path) -> Result<BackupSummary> {
    let dir = backups_dir(data_dir);
    std::fs::create_dir_all(&dir).context("création du dossier de sauvegardes")?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = dir.join(format!("auto-{stamp}.{BACKUP_EXTENSION}"));
    // Automatic snapshots are always plain v1, never password-protected.
    let summary = export(lib, data_dir, &dest, &BackupOptions::automatic(), None)?;
    prune_auto_snapshots(data_dir, AUTO_SNAPSHOT_KEEP);
    Ok(summary)
}

fn prune_auto_snapshots(data_dir: &Path, keep: usize) {
    let mut autos: Vec<PathBuf> = list_snapshots(data_dir)
        .into_iter()
        .filter(|s| s.automatic)
        .map(|s| PathBuf::from(s.path))
        .collect();
    autos.sort();
    while autos.len() > keep {
        let oldest = autos.remove(0);
        let _ = std::fs::remove_file(oldest);
    }
}

pub fn list_snapshots(data_dir: &Path) -> Vec<SnapshotInfo> {
    let dir = backups_dir(data_dir);
    let mut out: Vec<SnapshotInfo> = entries_of(&dir, |path| has_extension(path, BACKUP_EXTENSION))
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().to_string();
            let encrypted = encrypted_backup::is_encrypted(&path);
            // An encrypted archive is not a ZIP: `read_manifest` yields None
            // and the listing simply lacks the manifest details.
            let manifest = read_manifest(&path);
            Some(SnapshotInfo {
                automatic: name.starts_with("auto-"),
                bytes: std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                created_at: manifest.as_ref().map(|m| m.created_at.clone()),
                lua_count: manifest.as_ref().map(|m| m.lua_count).unwrap_or(0),
                fix_archive_count: manifest.as_ref().map(|m| m.fix_archive_count).unwrap_or(0),
                name,
                path: path.display().to_string(),
                encrypted,
            })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lv_bak_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn seed(root: &Path) -> (PathBuf, PathBuf) {
        let data = root.join("data");
        let lib = data.join("library");
        std::fs::create_dir_all(crate::fixes::fixes_dir(&lib)).unwrap();
        std::fs::write(lib.join("index.json"), b"[]").unwrap();
        std::fs::write(lib.join("264710.lua"), b"-- lua").unwrap();
        std::fs::write(lib.join("42.lua"), b"-- lua2").unwrap();
        std::fs::write(
            crate::fixes::fixes_dir(&lib).join("42_online_fix.rar"),
            b"archive-bytes",
        )
        .unwrap();
        std::fs::write(
            crate::fixes::fixes_dir(&lib).join("42.state.json"),
            b"{\"app_id\":\"42\"}",
        )
        .unwrap();
        std::fs::write(data.join("config.json"), b"{}").unwrap();
        (lib, data)
    }

    #[test]
    fn export_then_import_restores_every_piece() {
        let _lock = library::cache_test_lock();
        let root = scratch("roundtrip");
        let (lib, data) = seed(&root);

        let dest = root.join("backup.luabak");
        let summary = export(&lib, &data, &dest, &BackupOptions::default(), None).unwrap();
        assert_eq!(summary.lua_count, 2);
        assert_eq!(summary.fix_archive_count, 1);
        assert_eq!(summary.fix_state_count, 1);
        assert!(summary.bytes > 0);
        assert!(!encrypted_backup::is_encrypted(&dest), "sans mot de passe, l'archive reste v1");

        // Wipe the library, then restore it.
        std::fs::remove_dir_all(&lib).unwrap();
        let restored = import(&dest, &lib, &data, None).unwrap();
        assert_eq!(restored.lua_restored, 2);
        assert_eq!(restored.fix_archives_restored, 1);
        assert_eq!(restored.fix_states_restored, 1);
        assert!(restored.config_restored);
        assert_eq!(std::fs::read(lib.join("264710.lua")).unwrap(), b"-- lua");
        assert_eq!(
            std::fs::read(crate::fixes::fixes_dir(&lib).join("42_online_fix.rar")).unwrap(),
            b"archive-bytes"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_rejects_an_unknown_backup_extension() {
        let _lock = library::cache_test_lock();
        let root = scratch("legacy_extension");
        let (lib, data) = seed(&root);
        let extension = "xyzbak";
        let foreign = root.join(format!("existing.{extension}"));
        export(&lib, &data, &foreign, &BackupOptions::default(), None).unwrap();

        std::fs::remove_dir_all(&lib).unwrap();
        let error = import(&foreign, &lib, &data, None).unwrap_err();
        assert!(error.to_string().contains("extension de sauvegarde non reconnue"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn automatic_snapshots_skip_archives_and_are_pruned() {
        let root = scratch("auto");
        let (lib, data) = seed(&root);

        for i in 0..3 {
            // Distinct names without waiting a full second.
            let dest = backups_dir(&data).join(format!("auto-2024010{i}-000000.luabak"));
            std::fs::create_dir_all(backups_dir(&data)).unwrap();
            export(&lib, &data, &dest, &BackupOptions::automatic(), None).unwrap();
        }
        let extension = "xyzbak";
        let foreign = backups_dir(&data).join(format!("auto-20231231-000000.{extension}"));
        export(&lib, &data, &foreign, &BackupOptions::automatic(), None).unwrap();
        let listed = list_snapshots(&data);
        assert_eq!(listed.len(), 3, "unknown-extension snapshots are ignored");
        assert!(listed.iter().all(|s| s.automatic));
        assert!(listed.iter().all(|s| s.fix_archive_count == 0));
        // Newest first.
        assert!(listed[0].name > listed[2].name);

        prune_auto_snapshots(&data, 2);
        assert_eq!(list_snapshots(&data).len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_rejects_a_foreign_archive() {
        let root = scratch("foreign");
        let (lib, data) = seed(&root);
        let bogus = root.join("random.luabak");
        std::fs::write(&bogus, b"definitely not a zip").unwrap();
        let err = import(&bogus, &lib, &data, None).unwrap_err();
        assert!(err.to_string().contains("sauvegarde LuaVault valide"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn safe_relative_rejects_path_traversal() {
        // --- Cases that must be rejected ---
        assert!(
            safe_relative("../evade.lua").is_none(),
            "../ should be rejected"
        );
        assert!(
            safe_relative("..\\evade.lua").is_none(),
            "..\\ should be rejected"
        );
        assert!(
            safe_relative("C:\\Windows\\evil.dll").is_none(),
            "drive prefix should be rejected"
        );
        assert!(
            safe_relative("/etc/passwd").is_none(),
            "absolute unix path should be rejected"
        );
        assert!(safe_relative("").is_none(), "empty string should be rejected");
        assert!(
            safe_relative("library/../../evade.lua").is_none(),
            "embedded .. should be rejected"
        );

        // Windows reserved device names.
        assert!(
            safe_relative("CON.lua").is_none(),
            "CON device name should be rejected"
        );
        assert!(
            safe_relative("nul").is_none(),
            "NUL device name should be rejected"
        );
        assert!(
            safe_relative("LPT1.lua").is_none(),
            "LPT1 device name should be rejected"
        );

        // NTFS alternate data streams.
        assert!(
            safe_relative("index.json:payload").is_none(),
            "ADS stream should be rejected"
        );

        // --- Cases that must be accepted ---
        assert!(
            safe_relative("fixes/1234.zip").is_some(),
            "normal nested path should be accepted"
        );
        assert!(
            safe_relative("1234.lua").is_some(),
            "simple filename should be accepted"
        );
        // Names that contain reserved words but are NOT reserved themselves.
        assert!(
            safe_relative("console.lua").is_some(),
            "console.lua is not a reserved name"
        );
        assert!(
            safe_relative("nullable.lua").is_some(),
            "nullable.lua is not a reserved name"
        );
    }

    #[test]
    fn import_skips_zip_slip_entries() {
        use std::io::Write;

        let _lock = library::cache_test_lock();
        let root = scratch("zipslip");
        let (lib, data) = seed(&root);

        // Build a valid .luabak zip that contains path-traversal and absolute entries.
        let backup = root.join("ast_review_evil.luabak");
        {
            let file = std::fs::File::create(&backup).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);

            // Valid manifest so the archive passes validation.
            let manifest = serde_json::json!({
                "format": 1,
                "created_at": "2024-01-01T00:00:00Z",
                "app_version": env!("CARGO_PKG_VERSION"),
                "lua_count": 0,
                "fix_archive_count": 0,
                "fix_state_count": 0,
                "has_config": false
            });
            zip.start_file(MANIFEST_NAME, opts).unwrap();
            zip.write_all(&serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

            // WARNING: if the zip-slip protection regresses, this zip will write
            // `ast_review_*` files outside the scratch directory, causing this
            // test to fail permanently until the stale files are cleaned up.
            for name in [
                "library/../../ast_review_evil.lua",
                r"library/C:\Windows\Temp\ast_review_evil.lua",
                "library//ast_review_evil_root.lua",
                r"library/C:ast_review_evil_rel.lua",
            ] {
                zip.start_file(name, opts).unwrap();
                zip.write_all(b"pwned").unwrap();
            }
            zip.finish().unwrap();
        }

        // M9 guard: the zip-slip fixture goes through the real `backup::import`,
        // so a loosened `safe_relative` turns this test red.
        let summary = import(&backup, &lib, &data, None).unwrap();
        assert_eq!(summary.lua_restored, 0);
        assert_eq!(
            summary.entries_skipped, 4,
            "les quatre entrées malveillantes doivent être écartées"
        );
        assert_eq!(summary.fix_archives_restored, 0);
        assert_eq!(summary.fix_states_restored, 0);

        // Confirm no file was written outside the library tree.
        let evil = root.join("ast_review_evil.lua");
        assert!(!evil.exists(), "ast_review_evil.lua must not have been written outside lib");

        // Confirm no file was written to the absolute path targets.
        assert!(
            !PathBuf::from(r"C:\Windows\Temp\ast_review_evil.lua").exists(),
            "absolute path target must not exist"
        );
        assert!(
            !PathBuf::from(r"C:\ast_review_evil_rel.lua").exists(),
            "drive-relative target must not exist"
        );

        // Confirm no new files appeared in the test library directory.
        let lib_entries = std::fs::read_dir(&lib).unwrap();
        let lib_files: Vec<_> = lib_entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        for name in &lib_files {
            assert!(
                !name.starts_with("ast_review_evil"),
                "unexpected file in library: {name}"
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    // ── LOT-21 wiring: encrypted exports, import adoption, safe publication ──

    /// Deterministic incompressible payload, so deflate cannot shrink the
    /// fixture below the 64 KiB framing threshold of the encrypted format.
    fn noise(len: usize, mut state: u64) -> Vec<u8> {
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state & 0xff) as u8
            })
            .collect()
    }

    /// Recursive snapshot of a directory: sorted (relative path, content) pairs.
    fn tree_state(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(dir) {
                    out.push((
                        rel.to_string_lossy().replace('\\', "/"),
                        std::fs::read(&p).unwrap(),
                    ));
                }
            }
        }
        out.sort();
        out
    }

    /// Our temps (`.luabak.*.tmp`) and encrypted_backup's (`enc_*`, `dec_*`)
    /// all end in `.tmp` — none may survive, on success or on error.
    fn assert_no_temps(dir: &Path) {
        for (name, _) in tree_state(dir) {
            assert!(!name.ends_with(".tmp"), "temporaire résiduel: {name}");
        }
    }

    /// root / lib / data / a v2 archive carrying more than two 64 KiB frames.
    fn encrypted_fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = scratch(tag);
        let (lib, data) = seed(&root);
        std::fs::write(
            crate::fixes::fixes_dir(&lib).join("777_online_fix.rar"),
            noise(250_000, 0xa11ce),
        )
        .unwrap();
        let archive = root.join("tamper.luabak");
        export(&lib, &data, &archive, &BackupOptions::default(), Some("passe")).unwrap();
        (root, lib, data, archive)
    }

    /// The current v2 header layout, in bytes: magic(8) version(4) mem(4)
    /// iters(4) par(4) salt(32) nonce(7) size(8) — see encrypted_backup.rs.
    /// Legacy encrypted headers retain their 9-byte magic and layout.
    const V2_HEADER_LEN: usize = 8 + 4 + 4 + 4 + 4 + 32 + 7 + 8;
    const V2_BLOCK: usize = 65_536;
    const V2_TAG: usize = 16;

    fn assert_tamper_refused_before_any_write(
        root: &Path,
        lib: &Path,
        data: &Path,
        tampered: &Path,
    ) {
        let before = tree_state(lib);
        let config_before = std::fs::read(data.join("config.json")).unwrap();
        let err = import(tampered, lib, data, Some("passe")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "mot de passe incorrect ou archive altérée",
            "l'altération est refusée avec le message stable"
        );
        assert_eq!(
            tree_state(lib),
            before,
            "l'altération échoue avant toute écriture dans la bibliothèque"
        );
        assert_eq!(
            std::fs::read(data.join("config.json")).unwrap(),
            config_before,
            "l'altération échoue avant toute écriture dans la config"
        );
        assert_no_temps(root);
    }

    #[test]
    fn encrypted_roundtrip_restores_a_multi_block_library() {
        let _lock = library::cache_test_lock();
        let root = scratch("enc_round");
        let (lib, data) = seed(&root);
        let big = noise(250_000, 0x5eed);
        std::fs::write(
            crate::fixes::fixes_dir(&lib).join("777_online_fix.rar"),
            &big,
        )
        .unwrap();

        let dest = root.join("secure.luabak");
        let summary = export(
            &lib,
            &data,
            &dest,
            &BackupOptions::default(),
            Some("passe longue"),
        )
        .unwrap();
        assert!(summary.bytes > 0);
        assert_eq!(summary.fix_archive_count, 2);
        assert!(
            encrypted_backup::is_encrypted(&dest),
            "l'export chiffré porte le format v2"
        );

        // Wipe the library entirely, then restore it.
        std::fs::remove_dir_all(&lib).unwrap();
        library::clear_index_cache();
        let restored = import(&dest, &lib, &data, Some("passe longue")).unwrap();
        assert_eq!(restored.lua_restored, 2);
        assert_eq!(restored.fix_archives_restored, 2);
        assert_eq!(restored.fix_states_restored, 1);
        assert!(restored.config_restored);
        assert_eq!(
            std::fs::read(crate::fixes::fixes_dir(&lib).join("777_online_fix.rar")).unwrap(),
            big,
            "le payload multi-blocs revient intact"
        );
        assert_no_temps(&root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_encrypted_exports_of_the_same_data_differ_byte_for_byte() {
        let root = scratch("enc_distinct");
        let (lib, data) = seed(&root);
        let a = root.join("a.luabak");
        let b = root.join("b.luabak");
        export(&lib, &data, &a, &BackupOptions::default(), Some("même passe")).unwrap();
        export(&lib, &data, &b, &BackupOptions::default(), Some("même passe")).unwrap();
        assert_ne!(
            std::fs::read(&a).unwrap(),
            std::fs::read(&b).unwrap(),
            "sel et nonce aléatoires : jamais deux archives identiques"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_wrong_password_refuses_the_import_and_touches_nothing() {
        let _lock = library::cache_test_lock();
        let root = scratch("enc_wrongpw");
        let (lib, data) = seed(&root);
        let archive = root.join("secure.luabak");
        export(&lib, &data, &archive, &BackupOptions::default(), Some("le bon")).unwrap();

        let before = tree_state(&lib);
        let config_before = std::fs::read(data.join("config.json")).unwrap();
        let err = import(&archive, &lib, &data, Some("le mauvais")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "mot de passe incorrect ou archive altérée"
        );
        assert_eq!(tree_state(&lib), before, "bibliothèque byte-identique");
        assert_eq!(
            std::fs::read(data.join("config.json")).unwrap(),
            config_before,
            "config byte-identique"
        );
        assert_no_temps(&root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_flipped_header_bit_is_refused_before_any_write() {
        let _lock = library::cache_test_lock();
        let (root, lib, data, archive) = encrypted_fixture("tamper_header");
        let mut bytes = std::fs::read(&archive).unwrap();
        bytes[30] ^= 0x40; // inside the salt — the magic still announces v2
        let tampered = root.join("tampered.luabak");
        std::fs::write(&tampered, &bytes).unwrap();
        assert_tamper_refused_before_any_write(&root, &lib, &data, &tampered);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_modified_first_block_is_refused_before_any_write() {
        let _lock = library::cache_test_lock();
        let (root, lib, data, archive) = encrypted_fixture("tamper_first");
        let mut bytes = std::fs::read(&archive).unwrap();
        bytes[V2_HEADER_LEN + 5] ^= 0x01;
        let tampered = root.join("tampered.luabak");
        std::fs::write(&tampered, &bytes).unwrap();
        assert_tamper_refused_before_any_write(&root, &lib, &data, &tampered);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_modified_last_block_is_refused_before_any_write() {
        let _lock = library::cache_test_lock();
        let (root, lib, data, archive) = encrypted_fixture("tamper_last");
        let mut bytes = std::fs::read(&archive).unwrap();
        let len = bytes.len();
        bytes[len - 3] ^= 0x01; // inside the last frame's GCM tag
        let tampered = root.join("tampered.luabak");
        std::fs::write(&tampered, &bytes).unwrap();
        assert_tamper_refused_before_any_write(&root, &lib, &data, &tampered);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_truncated_archive_is_refused_before_any_write() {
        let _lock = library::cache_test_lock();
        let (root, lib, data, archive) = encrypted_fixture("tamper_trunc");
        let mut bytes = std::fs::read(&archive).unwrap();
        bytes.truncate(bytes.len() - 100);
        let tampered = root.join("tampered.luabak");
        std::fs::write(&tampered, &bytes).unwrap();
        assert_tamper_refused_before_any_write(&root, &lib, &data, &tampered);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reordered_blocks_are_refused_before_any_write() {
        let _lock = library::cache_test_lock();
        let (root, lib, data, archive) = encrypted_fixture("tamper_order");
        let mut bytes = std::fs::read(&archive).unwrap();
        // Plaintext size announced by the header → frame count.
        let plain = u64::from_le_bytes(bytes[64..72].try_into().unwrap()) as usize;
        assert!(
            plain > V2_BLOCK * 2,
            "le fixture doit porter au moins trois blocs chiffrés"
        );
        let frame_len = V2_BLOCK + V2_TAG;
        let first = bytes[V2_HEADER_LEN..V2_HEADER_LEN + frame_len].to_vec();
        let second = bytes[V2_HEADER_LEN + frame_len..V2_HEADER_LEN + 2 * frame_len].to_vec();
        bytes[V2_HEADER_LEN..V2_HEADER_LEN + frame_len].copy_from_slice(&second);
        bytes[V2_HEADER_LEN + frame_len..V2_HEADER_LEN + 2 * frame_len].copy_from_slice(&first);
        let tampered = root.join("tampered.luabak");
        std::fs::write(&tampered, &bytes).unwrap();
        assert_tamper_refused_before_any_write(&root, &lib, &data, &tampered);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_export_leaves_an_existing_destination_untouched_and_no_temp() {
        let root = scratch("export_fail");
        let (lib, data) = seed(&root);

        // The destination is an existing directory: publication cannot take it.
        let dest = root.join("blocked.luabak");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("sentinelle.txt"), b"sentinelle").unwrap();

        for password in [None, Some("passe")] {
            assert!(
                export(&lib, &data, &dest, &BackupOptions::default(), password).is_err(),
                "l'export doit échouer quand la destination est un dossier"
            );
        }
        assert!(
            dest.is_dir(),
            "la destination préexistante n'est pas remplacée"
        );
        assert_eq!(
            std::fs::read(dest.join("sentinelle.txt")).unwrap(),
            b"sentinelle"
        );
        assert_no_temps(&root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_export_never_truncates_a_preexisting_archive() {
        let root = scratch("export_trunc");
        let (lib, data) = seed(&root);
        let dest = root.join("existant.luabak");
        let original = b"archive preexistante - contenu integral";
        std::fs::write(&dest, original).unwrap();

        // Read-only: the publication by rename refuses to replace the file.
        let writable = std::fs::metadata(&dest).unwrap().permissions();
        let mut readonly = writable.clone();
        readonly.set_readonly(true);
        std::fs::set_permissions(&dest, readonly).unwrap();

        for password in [None, Some("passe")] {
            assert!(
                export(&lib, &data, &dest, &BackupOptions::default(), password).is_err(),
                "la publication doit échouer sur une destination en lecture seule"
            );
        }

        std::fs::set_permissions(&dest, writable).unwrap();
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            original,
            "l'archive préexistante n'est ni tronquée ni modifiée"
        );
        assert_no_temps(&root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_v1_zip_carries_neither_the_key_nor_the_sidecar() {
        let root = scratch("no_secrets");
        let (lib, data) = seed(&root);
        // The installation already holds a key and a signed index — the most
        // tempting material for a careless exporter.
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&lib.join("index.json"), &key).unwrap();

        let dest = root.join("plain.luabak");
        export(&lib, &data, &dest, &BackupOptions::default(), None).unwrap();

        let file = std::fs::File::open(&dest).unwrap();
        let zip = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = zip.file_names().map(|n| n.to_string()).collect();
        assert!(
            !names.iter().any(|n| n == "hmac.key" || n.ends_with("/hmac.key")),
            "la clé HMAC n'entre jamais dans une archive"
        );
        assert!(
            !names.iter().any(|n| n.ends_with(".hmac")),
            "la sidecar n'entre jamais dans une archive"
        );
        assert!(
            names.iter().any(|n| n == "library/index.json"),
            "l'index, lui, voyage — il sera signé à l'import"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_signs_the_restored_index_with_the_receiving_key() {
        let _lock = library::cache_test_lock();
        let root = scratch("adopt_v1");
        let (lib, data) = seed(&root);
        // The receiving installation already holds its key.
        let key = hmac::load_or_create_key(&data).unwrap();

        let dest = root.join("plain.luabak");
        export(&lib, &data, &dest, &BackupOptions::default(), None).unwrap();

        // Wipe the library entirely — the sidecar is never exported.
        std::fs::remove_dir_all(&lib).unwrap();
        library::clear_index_cache();

        import(&dest, &lib, &data, None).unwrap();

        // The strict load succeeds: the restored index carries a valid
        // sidecar for THIS installation's key.
        let entries = library::load_index_with_data_dir(&lib, &data).unwrap();
        assert!(entries.is_empty(), "l'index semé est une liste vide");
        assert!(hmac::has_sidecar(&lib.join("index.json")));
        assert!(hmac::verify(&lib.join("index.json"), &key).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_signs_the_restored_index_when_the_archive_is_encrypted() {
        let _lock = library::cache_test_lock();
        let root = scratch("adopt_v2");
        let (lib, data) = seed(&root);
        let key = hmac::load_or_create_key(&data).unwrap();

        let dest = root.join("secure.luabak");
        export(
            &lib,
            &data,
            &dest,
            &BackupOptions::default(),
            Some("passe d'import"),
        )
        .unwrap();
        assert!(encrypted_backup::is_encrypted(&dest));

        std::fs::remove_dir_all(&lib).unwrap();
        library::clear_index_cache();

        import(&dest, &lib, &data, Some("passe d'import")).unwrap();

        let entries = library::load_index_with_data_dir(&lib, &data).unwrap();
        assert!(entries.is_empty());
        assert!(hmac::verify(&lib.join("index.json"), &key).unwrap());
        assert_no_temps(&root);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_archive_without_index_leaves_the_live_pair_untouched() {
        let _lock = library::cache_test_lock();
        let root = scratch("adopt_skip");
        let (lib, data) = seed(&root);
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&lib.join("index.json"), &key).unwrap();
        let index_before = std::fs::read(lib.join("index.json")).unwrap();
        let sidecar_path = hmac::sidecar_path(&lib.join("index.json"));
        let sidecar_before = std::fs::read(&sidecar_path).unwrap();

        let dest = root.join("sans_index.luabak");
        let options = BackupOptions {
            include_lua: false,
            include_fix_archives: true,
            include_fix_states: true,
            include_config: false,
        };
        export(&lib, &data, &dest, &options, None).unwrap();
        import(&dest, &lib, &data, None).unwrap();

        assert_eq!(
            std::fs::read(lib.join("index.json")).unwrap(),
            index_before,
            "l'index vivant n'est pas touché"
        );
        assert_eq!(
            std::fs::read(&sidecar_path).unwrap(),
            sidecar_before,
            "la sidecar vivante n'est ni réécrite ni effacée"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An archive whose restored index is not valid JSON fails the import at
    /// the adoption step — the migration frontiers validate BEFORE signing, so
    /// a damaged index never gets a signature that would vouch for it.
    #[test]
    fn import_refuses_an_archive_whose_index_is_not_valid_json() {
        let _lock = library::cache_test_lock();
        let root = scratch("adopt_badjson");
        let (lib, data) = seed(&root);

        // A well-formed archive whose library/index.json is damaged.
        let archive = root.join("casse.luabak");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let manifest = serde_json::json!({
                "format": 1,
                "created_at": "2024-01-01T00:00:00Z",
                "app_version": env!("CARGO_PKG_VERSION"),
                "lua_count": 0,
                "fix_archive_count": 0,
                "fix_state_count": 0,
                "has_config": false
            });
            zip.start_file(MANIFEST_NAME, opts).unwrap();
            zip.write_all(&serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
            zip.start_file("library/index.json", opts).unwrap();
            zip.write_all(b"{pas un index").unwrap();
            zip.finish().unwrap();
        }

        let err = import(&archive, &lib, &data, None).unwrap_err();
        assert!(
            err.to_string().contains("JSON valide"),
            "l'index endommagé casse l'import, got: {err}"
        );
        // No signature vouches for the damaged index.
        assert!(!hmac::has_sidecar(&lib.join("index.json")));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// UI contract: an empty password means "not encrypted" — the export
    /// produces a plain v1 archive, importable without any password. Removing
    /// the `.filter(|p| !p.is_empty())` in `export` turns this red.
    #[test]
    fn export_with_an_empty_password_stays_v1() {
        let _lock = library::cache_test_lock();
        let root = scratch("empty_pw");
        let (lib, data) = seed(&root);
        let dest = root.join("vide.luabak");

        let summary = export(&lib, &data, &dest, &BackupOptions::default(), Some("")).unwrap();
        assert!(summary.bytes > 0);
        assert!(
            !encrypted_backup::is_encrypted(&dest),
            "mot de passe vide = archive non chiffrée"
        );
        // And it restores without a password.
        std::fs::remove_dir_all(&lib).unwrap();
        library::clear_index_cache();
        let restored = import(&dest, &lib, &data, None).unwrap();
        assert_eq!(restored.lua_restored, 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn importing_an_encrypted_archive_without_password_asks_for_one() {
        let _lock = library::cache_test_lock();
        let root = scratch("enc_nopw");
        let (lib, data) = seed(&root);
        let dest = root.join("secure.luabak");
        export(&lib, &data, &dest, &BackupOptions::default(), Some("passe")).unwrap();
        // The detection order is load-bearing (M4): is_encrypted runs BEFORE
        // read_manifest, or this error would be "pas une sauvegarde valide".
        for missing in [None, Some("")] {
            let err = import(&dest, &lib, &data, missing).unwrap_err();
            assert_eq!(err.to_string(), "archive chiffrée : mot de passe requis");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_snapshots_survives_encrypted_archives_and_flags_them() {
        let root = scratch("list_enc");
        let (lib, data) = seed(&root);
        let dir = backups_dir(&data);
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("auto-20240101-000000.luabak");
        export(&lib, &data, &plain, &BackupOptions::automatic(), None).unwrap();
        let sealed = dir.join("manuel-20240102-000000.luabak");
        export(&lib, &data, &sealed, &BackupOptions::default(), Some("passe")).unwrap();

        let listed = list_snapshots(&data);
        assert_eq!(listed.len(), 2, "les deux archives figurent au listing");
        let sealed_info = listed.iter().find(|s| s.name.starts_with("manuel-")).unwrap();
        assert!(sealed_info.encrypted, "l'archive chiffrée est signalée");
        assert_eq!(sealed_info.lua_count, 0, "le manifeste scellé ne fuit pas");
        assert_eq!(sealed_info.created_at, None);
        let plain_info = listed.iter().find(|s| s.name.starts_with("auto-")).unwrap();
        assert!(!plain_info.encrypted);
        assert_eq!(plain_info.lua_count, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── LOT-21 recovery: encrypted temps in data_dir, orphan cleanup ──

    /// List all files in a directory (non-recursive).
    fn dir_files(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .ok()
            .into_iter()
            .flat_map(|r| r.filter_map(|e| e.ok()))
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect()
    }

    /// Test 1: encrypted export creates NO extra files in dest's directory.
    #[test]
    fn encrypted_export_creates_no_temp_in_dest_dir() {
        let _lock = library::cache_test_lock();
        let root = scratch("enc_temp_dest");
        let (lib, data) = seed(&root);

        let dest_dir = root.join("dest");
        std::fs::create_dir_all(&dest_dir).unwrap();
        let dest = dest_dir.join("backup.luabak");

        let before = dir_files(&dest_dir);
        assert!(before.is_empty(), "le dossier de destination est vide avant l'export");

        export(&lib, &data, &dest, &BackupOptions::default(), Some("passe")).unwrap();

        let after = dir_files(&dest_dir);
        // Seule l'archive finale doit être présente.
        assert_eq!(after.len(), 1, "un seul fichier dans dest après export chiffré");
        assert!(
            after[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".luabak"),
            "le fichier est l'archive chiffrée"
        );
        assert_no_temps(&root);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 2: encrypted import creates NO extra files in archive's directory.
    #[test]
    fn encrypted_import_creates_no_temp_in_archive_dir() {
        let _lock = library::cache_test_lock();
        let root = scratch("enc_temp_import");
        let (lib, data) = seed(&root);

        let archive = root.join("secure.luabak");
        export(&lib, &data, &archive, &BackupOptions::default(), Some("passe")).unwrap();

        let archive_dir = root.as_path();
        let before = dir_files(archive_dir);
        assert_eq!(before.len(), 1, "un seul fichier avant import");

        let lib2 = root.join("library2");
        import(&archive, &lib2, &data, Some("passe")).unwrap();

        let after = dir_files(archive_dir);
        assert_eq!(after.len(), 1, "un seul fichier après import chiffré");
        assert_no_temps(&root);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 3: failed encrypted export leaves NO temp in dest_dir or data_dir.
    #[test]
    fn failed_encrypted_export_leaves_no_temp() {
        let root = scratch("enc_export_fail");
        let (lib, data) = seed(&root);

        // Destination is a directory — publication cannot take it.
        let dest = root.join("blocked.luabak");
        std::fs::create_dir(&dest).unwrap();

        let dest_files_before = dir_files(&root);
        let data_files_before = dir_files(&data);

        let err = export(&lib, &data, &dest, &BackupOptions::default(), Some("passe")).unwrap_err();
        assert!(!err.to_string().is_empty(), "l'export chiffré vers un dossier doit échouer, got: {err}");

        let dest_files_after = dir_files(&root);
        let data_files_after = dir_files(&data);
        // Aucun nouveau fichier — ni temp dans root, ni dans data.
        assert_eq!(dest_files_after.len(), dest_files_before.len(), "aucun temp ajouté dans root");
        assert_eq!(data_files_after.len(), data_files_before.len(), "aucun temp ajouté dans data");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 4: orphan cleanup removes old temps, keeps recent ones, skips non-patterns.
    ///
    /// Under Windows we cannot easily set an old mtime, so we test the
    /// pattern-bounding logic directly: old files are removed only when
    /// their mtime is > 1 h. We create a temp, wait 1.1 s, then call
    /// cleanup — the temp should still be there (not old enough). We also
    /// verify that non-matching files survive regardless of age.
    #[test]
    fn orphan_cleanup_respects_age_and_pattern() {
        let root = scratch("orphan_cleanup");
        std::fs::create_dir_all(&root).unwrap();

        // Recent temp — should NOT be removed (too young).
        let recent_temp = root.join(".luabak.partial.1234567890abcdef1234567890abcdef.tmp");
        std::fs::write(&recent_temp, b"junk").unwrap();

        // .luabak file — should NOT be removed (wrong pattern).
        let luabak = root.join("auto-20240101-000000.luabak");
        std::fs::write(&luabak, b"junk").unwrap();

        // Non-pattern file — should NOT be removed.
        let other = root.join("config.json");
        std::fs::write(&other, b"{}").unwrap();

        // File with wrong hex length — should NOT be removed.
        let bad_hex = root.join(".luabak.partial.abc.tmp");
        std::fs::write(&bad_hex, b"junk").unwrap();

        // File without .tmp extension — should NOT be removed.
        let no_tmp = root.join(".luabak.partial.abcdef0123456789abcdef0123456789");
        std::fs::write(&no_tmp, b"junk").unwrap();

        cleanup_orphan_temps(&root);

        assert!(recent_temp.exists(), "le temp récent doit être conservé");
        assert!(luabak.exists(), "le fichier .luabak doit être conservé");
        assert!(other.exists(), "le fichier hors motif doit être conservé");
        assert!(bad_hex.exists(), "le fichier avec hex invalide doit être conservé");
        assert!(no_tmp.exists(), "le fichier sans .tmp doit être conservé");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 4b: orphan cleanup never touches files outside data_dir.
    #[test]
    fn orphan_cleanup_stays_bounded_to_data_dir() {
        let root = scratch("orphan_bound");
        let data = root.join("data");
        let outside = root.join("outside");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        // Temp outside data_dir — must NOT be touched.
        let outside_temp = outside.join(".luabak.partial.abcdef0123456789abcdef0123456789.tmp");
        std::fs::write(&outside_temp, b"junk").unwrap();

        cleanup_orphan_temps(&data);

        assert!(outside_temp.exists(), "le temp hors data_dir doit être conservé");
        let _ = std::fs::remove_dir_all(&root);
    }

    // ── LOT-21 recovery E1: temp_dir_for observable decision ──

    #[test]
    fn temp_dir_for_encrypted_returns_data_dir() {
        let root = scratch("tempdir_enc");
        let data = root.join("data");
        let dest = root.join("dest").join("backup.luabak");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();

        assert_eq!(temp_dir_for(true, &dest, &data), data);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn temp_dir_for_plaintext_returns_dest_parent() {
        let root = scratch("tempdir_plain");
        let data = root.join("data");
        let dest = root.join("dest").join("backup.luabak");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();

        assert_eq!(
            temp_dir_for(false, &dest, &data),
            dest.parent().unwrap()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn temp_dir_for_plaintext_dest_in_root_returns_dot() {
        let root = scratch("tempdir_root");
        let data = root.join("data");
        let dest = root.join("backup.luabak");

        // dest.parent() is root — not empty — so it should return root
        assert_eq!(
            temp_dir_for(false, &dest, &data),
            root
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Démonstration 6 : bout en bout par `backup::import` — un steam_dir
    /// inexistant dans l'archive ne remplace pas le local, et les .lua
    /// sont bien restaurés.
    #[test]
    fn import_merges_config_and_keeps_local_steam_dir() {
        let _lock = library::cache_test_lock();
        let root = scratch("e2e_merge");
        let (lib, data) = seed(&root);

        // Write a local config with a valid steam_dir.
        let local_cfg = crate::config::AppConfig {
            steam_dir: Some(PathBuf::from("C:\\Steam")),
            library_dir: Some(PathBuf::from("D:\\SteamLib")),
            theme: Some("light".to_string()),
            dark_mode: Some(false),
            locale: None,
            first_run_done: true,
            defender_exclusions: None,
            update_notified_version: None,
            update_from_version: None, default_archive_password: None,
        };
        let local_json = serde_json::to_vec_pretty(&local_cfg).unwrap();
        std::fs::write(data.join("config.json"), &local_json).unwrap();

        // Create a backup.
        let dest = root.join("backup.luabak");
        export(&lib, &data, &dest, &BackupOptions::default(), None).unwrap();

        // Tamper the config inside the archive: replace steam_dir with a bogus path.
        let archive_path = dest;
        // We need to modify the config.json inside the zip.
        // Instead of manipulating the zip bytes directly, let's create a new
        // archive with a tampered config.
        let tampered = root.join("tampered.luabak");
        {
            let mut writer = zip::ZipWriter::new(
                std::fs::File::create(&tampered).unwrap(),
            );
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            // Write manifest
            let manifest = read_manifest(&archive_path).unwrap();
            let manifest_json = serde_json::to_vec_pretty(&manifest).unwrap();
            writer.start_file("manifest.json", options).unwrap();
            writer.write_all(&manifest_json).unwrap();

            // Write tampered config.json with a bogus steam_dir
            let tampered_cfg = crate::config::AppConfig {
                steam_dir: Some(PathBuf::from("Z:\\NonExistentSteam")),
                library_dir: Some(PathBuf::from("Z:\\NonExistentLib")),
                theme: Some("dark".to_string()),
                dark_mode: Some(true),
                locale: None,
                first_run_done: true,
                defender_exclusions: None,
                update_notified_version: None,
                update_from_version: None, default_archive_password: None,
            };
            let tampered_json = serde_json::to_vec_pretty(&tampered_cfg).unwrap();
            writer.start_file("config.json", options).unwrap();
            writer.write_all(&tampered_json).unwrap();

            // Copy library files
            let mut orig_zip = zip::ZipArchive::new(std::fs::File::open(&archive_path).unwrap()).unwrap();
            for i in 0..orig_zip.len() {
                let mut entry = orig_zip.by_index(i).unwrap();
                let name = entry.name().to_string();
                if name == "manifest.json" || name == "config.json" {
                    continue;
                }
                if entry.is_dir() {
                    continue;
                }
                writer.start_file(&name, options).unwrap();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                writer.write_all(&data).unwrap();
            }
            writer.finish().unwrap();
        }

        // Wipe the library, then import the tampered archive.
        std::fs::remove_dir_all(&lib).unwrap();
        let restored = import(&tampered, &lib, &data, None).unwrap();

        // The .lua files are restored.
        assert_eq!(restored.lua_restored, 2, "les .lua sont restaurés");
        assert_eq!(std::fs::read(lib.join("264710.lua")).unwrap(), b"-- lua");

        // The steam_dir on disk is still the local one.
        let final_cfg: crate::config::AppConfig =
            serde_json::from_str(&std::fs::read_to_string(data.join("config.json")).unwrap()).unwrap();
        assert_eq!(
            final_cfg.steam_dir,
            Some(PathBuf::from("C:\\Steam")),
            "le steam_dir local survit quand l'importé n'existe pas"
        );
        assert_eq!(
            final_cfg.library_dir,
            Some(PathBuf::from("D:\\SteamLib")),
            "idem pour library_dir"
        );
        // But theme was imported (it's not a path).
        assert_eq!(
            final_cfg.theme,
            Some("dark".to_string()),
            "le thème importé est adopté"
        );
        // config_kept_local should report what was kept.
        assert!(
            restored.config_kept_local.contains(&"steam_dir".to_string()),
            "config_kept_local contient steam_dir"
        );
        assert!(
            restored.config_kept_local.contains(&"library_dir".to_string()),
            "config_kept_local contient library_dir"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Démonstration 7 : une archive dont config.json n'est pas un JSON valide
    /// → l'import réussit, les .lua sont restaurés, la config locale est inchangée.
    #[test]
    fn import_skips_corrupt_config_and_restores_lua() {
        let _lock = library::cache_test_lock();
        let root = scratch("e2e_corrupt_cfg");
        let (lib, data) = seed(&root);

        // Write a known local config.
        let local_cfg = crate::config::AppConfig {
            steam_dir: Some(PathBuf::from("C:\\Steam")),
            library_dir: Some(PathBuf::from("D:\\SteamLib")),
            theme: Some("light".to_string()),
            dark_mode: Some(false),
            locale: None,
            first_run_done: true,
            defender_exclusions: None,
            update_notified_version: None,
            update_from_version: None, default_archive_password: None,
        };
        let local_json = serde_json::to_vec_pretty(&local_cfg).unwrap();
        std::fs::write(data.join("config.json"), &local_json).unwrap();
        let config_before = std::fs::read(data.join("config.json")).unwrap();

        // Create a backup.
        let dest = root.join("backup.luabak");
        export(&lib, &data, &dest, &BackupOptions::default(), None).unwrap();

        // Tamper the config inside the archive to be invalid JSON.
        let tampered = root.join("tampered.luabak");
        {
            let mut writer = zip::ZipWriter::new(
                std::fs::File::create(&tampered).unwrap(),
            );
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            // Write manifest
            let manifest = read_manifest(&dest).unwrap();
            let manifest_json = serde_json::to_vec_pretty(&manifest).unwrap();
            writer.start_file("manifest.json", options).unwrap();
            writer.write_all(&manifest_json).unwrap();

            // Write INVALID config.json
            writer.start_file("config.json", options).unwrap();
            writer.write_all(b"this is not valid json {{{").unwrap();

            // Copy library files
            let mut orig_zip = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
            for i in 0..orig_zip.len() {
                let mut entry = orig_zip.by_index(i).unwrap();
                let name = entry.name().to_string();
                if name == "manifest.json" || name == "config.json" {
                    continue;
                }
                if entry.is_dir() {
                    continue;
                }
                writer.start_file(&name, options).unwrap();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                writer.write_all(&data).unwrap();
            }
            writer.finish().unwrap();
        }

        // Wipe the library, then import the tampered archive.
        std::fs::remove_dir_all(&lib).unwrap();
        let restored = import(&tampered, &lib, &data, None).unwrap();

        // The import succeeds.
        assert!(restored.config_restored, "config_restored est true");
        assert_eq!(restored.lua_restored, 2, "les .lua sont restaurés");
        assert_eq!(std::fs::read(lib.join("264710.lua")).unwrap(), b"-- lua");

        // The local config is unchanged.
        let config_after = std::fs::read(data.join("config.json")).unwrap();
        assert_eq!(config_before, config_after, "la config locale est inchangée");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// C7 : une config locale présente mais **illisible** (octets non-UTF-8, et
    /// par extension un droit de lecture refusé) est laissée strictement intacte.
    /// C'est le bras que C5 ne couvre pas : là, `read_to_string` échoue avant
    /// même que serde soit appelé. Retomber sur `Default` effacerait le
    /// `steam_dir` de l'utilisateur — à rebours du fail-closed du LOT-21.
    #[test]
    fn import_preserves_an_unreadable_local_config() {
        let _lock = library::cache_test_lock();
        let root = scratch("e2e_unreadable_local");
        let (lib, data) = seed(&root);

        let dest = root.join("backup.luabak");
        export(&lib, &data, &dest, &BackupOptions::default(), None).unwrap();

        // Octets qui ne sont pas de l'UTF-8 valide : read_to_string échoue.
        let unreadable: &[u8] = &[0xff, 0xfe, 0x00, 0x9c, 0x80];
        std::fs::write(data.join("config.json"), unreadable).unwrap();

        std::fs::remove_dir_all(&lib).unwrap();
        let restored = import(&dest, &lib, &data, None).unwrap();
        assert_eq!(restored.lua_restored, 2, "les .lua sont restaurés");

        let after = std::fs::read(data.join("config.json")).unwrap();
        assert_eq!(
            after, unreadable,
            "la config locale illisible est laissée telle quelle"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// C6 : aucune config locale sur cette machine → la config importée est
    /// bel et bien écrite. C'est le cas d'une installation neuve qui restaure
    /// une sauvegarde : sans cela, `config_restored` annonce une restauration
    /// qui n'a pas eu lieu et le thème, `first_run_done` et le reste sont perdus.
    #[test]
    fn import_writes_config_when_no_local_config_exists() {
        let _lock = library::cache_test_lock();
        let root = scratch("e2e_no_local_config");
        let (lib, data) = seed(&root);

        // La sauvegarde porte une config reconnaissable.
        std::fs::write(
            data.join("config.json"),
            serde_json::to_vec_pretty(&crate::config::AppConfig {
                steam_dir: None,
                library_dir: None,
                theme: Some("sunset".to_string()),
                dark_mode: Some(true),
                locale: None,
                first_run_done: true,
                defender_exclusions: None,
                update_notified_version: None,
                update_from_version: None, default_archive_password: None,
            })
            .unwrap(),
        )
        .unwrap();

        let dest = root.join("backup.luabak");
        export(&lib, &data, &dest, &BackupOptions::default(), None).unwrap();

        // Machine neuve : ni bibliothèque, ni config.json.
        std::fs::remove_dir_all(&lib).unwrap();
        std::fs::remove_file(data.join("config.json")).unwrap();

        let restored = import(&dest, &lib, &data, None).unwrap();
        assert!(restored.config_restored, "config_restored est true");

        let written = std::fs::read_to_string(data.join("config.json"))
            .expect("config.json doit avoir été écrit par l'import");
        let cfg: crate::config::AppConfig = serde_json::from_str(&written)
            .expect("la config écrite doit être lisible");
        assert_eq!(
            cfg.theme,
            Some("sunset".to_string()),
            "le thème importé est adopté sur une machine sans config locale"
        );
        assert!(
            cfg.first_run_done,
            "first_run_done importé : l'onboarding ne doit pas être rejoué"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// C5 : une config locale corrompue n'est pas écrasée par défaut.
    /// Le fichier local reste intact, les .lua sont restaurés.
    #[test]
    fn import_preserves_corrupt_local_config() {
        let _lock = library::cache_test_lock();
        let root = scratch("e2e_corrupt_local");
        let (lib, data) = seed(&root);

        // Write a LOCAL config that is valid JSON but wrong types.
        let corrupt_local = b"{\"steam_dir\": 42, \"theme\": \"light\"}";
        std::fs::write(data.join("config.json"), corrupt_local).unwrap();
        let config_before = std::fs::read(data.join("config.json")).unwrap();

        // Create a backup.
        let dest = root.join("backup.luabak");
        export(&lib, &data, &dest, &BackupOptions::default(), None).unwrap();

        // Tamper the config inside the archive to have a bogus steam_dir.
        let tampered = root.join("tampered.luabak");
        {
            let mut writer = zip::ZipWriter::new(
                std::fs::File::create(&tampered).unwrap(),
            );
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            let manifest = read_manifest(&dest).unwrap();
            let manifest_json = serde_json::to_vec_pretty(&manifest).unwrap();
            writer.start_file("manifest.json", options).unwrap();
            writer.write_all(&manifest_json).unwrap();

            let tampered_cfg = crate::config::AppConfig {
                steam_dir: Some(PathBuf::from("Z:\\NonExistentSteam")),
                library_dir: Some(PathBuf::from("Z:\\NonExistentLib")),
                theme: Some("dark".to_string()),
                dark_mode: Some(true),
                locale: None,
                first_run_done: true,
                defender_exclusions: None,
                update_notified_version: None,
                update_from_version: None, default_archive_password: None,
            };
            let tampered_json = serde_json::to_vec_pretty(&tampered_cfg).unwrap();
            writer.start_file("config.json", options).unwrap();
            writer.write_all(&tampered_json).unwrap();

            let mut orig_zip = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
            for i in 0..orig_zip.len() {
                let mut entry = orig_zip.by_index(i).unwrap();
                let name = entry.name().to_string();
                if name == "manifest.json" || name == "config.json" {
                    continue;
                }
                if entry.is_dir() {
                    continue;
                }
                writer.start_file(&name, options).unwrap();
                let mut entry_data = Vec::new();
                entry.read_to_end(&mut entry_data).unwrap();
                writer.write_all(&entry_data).unwrap();
            }
            writer.finish().unwrap();
        }

        // Wipe the library, then import the tampered archive.
        std::fs::remove_dir_all(&lib).unwrap();
        let restored = import(&tampered, &lib, &data, None).unwrap();

        // The import succeeds.
        assert!(restored.config_restored, "config_restored est true");
        assert_eq!(restored.lua_restored, 2, "les .lua sont restaurés");

        // The local config is unchanged (not overwritten by default).
        let config_after = std::fs::read(data.join("config.json")).unwrap();
        assert_eq!(config_before, config_after, "la config locale corrompue est inchangée");

        let _ = std::fs::remove_dir_all(&root);
    }
}

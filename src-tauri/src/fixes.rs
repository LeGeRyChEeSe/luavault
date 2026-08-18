//! Online-fix lifecycle: install into the game folder, verify integrity,
//! and roll back to the pre-patch state from a compressed backup.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::archive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixFile {
    /// Path relative to the game root.
    pub rel: String,
    pub sha256: String,
    pub size: u64,
}

/// What we wrote where, so a later uninstall can undo exactly this and nothing else.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixState {
    pub app_id: String,
    pub game_dir: String,
    pub installed_at: String,
    pub files: Vec<FixFile>,
    /// Zip holding the original version of every file we overwrote.
    pub backup_zip: Option<String>,
    /// Relative paths that already existed before the fix was applied.
    pub backed_up: Vec<String>,
    /// Directories the install created, deepest first (for clean removal).
    pub created_dirs: Vec<String>,
}

/// Health of the installed fix, as shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixHealth {
    /// No fix has ever been installed for this game.
    NotInstalled,
    /// Every recorded file is present with the expected content.
    Healthy,
    /// Files are missing or were replaced (e.g. a Steam update reverted them).
    Damaged,
    /// The fix was installed elsewhere — the game moved or was reinstalled.
    GameMoved,
}

#[derive(Debug, Clone, Serialize)]
pub struct FixReport {
    pub app_id: String,
    pub health: FixHealth,
    pub installed_at: Option<String>,
    pub game_dir: Option<String>,
    pub file_count: usize,
    pub missing: Vec<String>,
    pub modified: Vec<String>,
    pub has_backup: bool,
    /// Online-fix files found in the game folder that *we* never installed —
    /// someone patched this game before the app got involved.
    #[serde(default)]
    pub foreign: Vec<String>,
}

impl FixReport {
    fn empty(app_id: &str) -> Self {
        FixReport {
            app_id: app_id.to_string(),
            health: FixHealth::NotInstalled,
            installed_at: None,
            game_dir: None,
            file_count: 0,
            missing: Vec::new(),
            modified: Vec::new(),
            has_backup: false,
            foreign: Vec::new(),
        }
    }
}

/// Files no vanilla Steam build ships — their presence means an online fix is
/// already applied. Deliberately narrow: generic proxy DLLs (`winmm.dll`,
/// `version.dll`) ship with plenty of legitimate mods, so they don't qualify.
const FOREIGN_MARKERS: [&str; 5] = [
    "OnlineFix64.dll",
    "OnlineFix.dll",
    "OnlineFix.ini",
    "OnlineFix.url",
    "dlllist.txt",
];

/// Detect an online fix applied outside the app, so an adopted game doesn't
/// look unpatched when it is. Searches the game root and its direct children.
pub fn detect_foreign(game_dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut roots = vec![game_dir.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(game_dir) {
        roots.extend(
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .take(64),
        );
    }
    for root in roots {
        for marker in FOREIGN_MARKERS {
            let path = root.join(marker);
            if path.is_file() {
                let rel = path
                    .strip_prefix(game_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                if !found.contains(&rel) {
                    found.push(rel);
                }
            }
        }
    }
    found.sort();
    found
}

pub fn fixes_dir(lib: &Path) -> PathBuf {
    lib.join("fixes")
}

pub fn archive_path(lib: &Path, app_id: &str) -> PathBuf {
    fixes_dir(lib).join(format!("{app_id}_online_fix.rar"))
}

pub fn state_path(lib: &Path, app_id: &str) -> PathBuf {
    fixes_dir(lib).join(format!("{app_id}.state.json"))
}

fn backup_path(lib: &Path, app_id: &str) -> PathBuf {
    fixes_dir(lib)
        .join("backups")
        .join(format!("{app_id}_pre-fix.zip"))
}

/// The tag a parked backup's name carries. Two parts: eight hex digits of
/// SHA-256 of the exact path — what makes the tag **injective**, two
/// libraries differing only by their drive letter still hash apart — and a
/// readable tail so a human can tell the folders apart. Injectivity rests on
/// the hash alone: which end of the tail survives truncation changes nothing
/// about it.
fn folder_tag(game_dir: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(game_dir.as_bytes());
    let short: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    // Only ASCII alphanumerics and lone underscores survive, so the name is
    // valid on every filesystem the library can live on.
    let mut tail: String = game_dir
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while tail.contains("__") {
        tail = tail.replace("__", "_");
    }
    let mut tail = tail.trim_matches('_').to_string();
    // Byte-safe: the mapping above only ever emits ASCII.
    if tail.len() > 40 {
        tail = tail[tail.len() - 40..].to_string();
    }
    if tail.is_empty() {
        short
    } else {
        format!("{short}_{tail}")
    }
}

/// A backup renamed after the folder it protects. When the game moves, the
/// zip at [`backup_path`] belongs to the OLD folder: it is moved aside under
/// this name instead of being overwritten — the old folder may still exist
/// with patched files, and that zip is the only way to give it back its
/// originals. The name carries an injective tag ([`folder_tag`]), but the
/// name alone never serves as proof of which folder a zip protects — that is
/// the companion file's job.
fn moved_backup_path(lib: &Path, app_id: &str, game_dir: &str) -> PathBuf {
    fixes_dir(lib)
        .join("backups")
        .join(format!("{app_id}_pre-fix_{}.zip", folder_tag(game_dir)))
}

/// Beside every parked zip: the exact `game_dir` it protects, byte for byte.
/// A filename cannot carry this guarantee (tags can collide in principle and
/// files can be renamed by hand); a promotion verifies the companion, never
/// the name.
fn companion_path(parked_zip: &Path) -> PathBuf {
    parked_zip.with_extension("dir")
}

fn write_companion(parked_zip: &Path, game_dir: &str) -> Result<()> {
    std::fs::write(companion_path(parked_zip), game_dir.as_bytes())
        .context("écriture du fichier compagnon de la sauvegarde déplacée")
}

fn companion_matches(parked_zip: &Path, game_dir: &str) -> bool {
    std::fs::read(companion_path(parked_zip))
        .map(|bytes| bytes == game_dir.as_bytes())
        .unwrap_or(false)
}

/// The backup parked for `game_dir`, whichever `_N` variant it fell into.
/// The companion file decides — a zip whose companion is missing or names
/// another folder is never promoted, so a collision or a manual rename can't
/// pass off someone else's originals for this folder's.
fn find_parked_backup(lib: &Path, app_id: &str, game_dir: &str) -> Option<PathBuf> {
    let base = moved_backup_path(lib, app_id, game_dir);
    let stem = base.file_stem()?.to_string_lossy().to_string();
    let mut candidate = base;
    let mut suffix = 2u32;
    while candidate.is_file() {
        if companion_matches(&candidate, game_dir) {
            return Some(candidate);
        }
        candidate = candidate.with_file_name(format!("{stem}_{suffix}.zip"));
        suffix += 1;
    }
    None
}

/// The first free name carrying `game_dir`, so two backups of two folders are
/// never merged and an existing backup is never overwritten.
fn next_free_moved_backup(lib: &Path, app_id: &str, game_dir: &str) -> PathBuf {
    let base = moved_backup_path(lib, app_id, game_dir);
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut target = base;
    let mut suffix = 2;
    while target.is_file() {
        target = target.with_file_name(format!("{stem}_{suffix}.zip"));
        suffix += 1;
    }
    target
}

pub fn load_state(lib: &Path, app_id: &str) -> Option<FixState> {
    let raw = std::fs::read_to_string(state_path(lib, app_id)).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_state(lib: &Path, state: &FixState) -> Result<()> {
    std::fs::create_dir_all(fixes_dir(lib)).context("création du dossier fixes")?;
    let raw = serde_json::to_vec_pretty(state).context("sérialisation de l'état du fix")?;
    std::fs::write(state_path(lib, &state.app_id), raw).context("écriture de l'état du fix")?;
    Ok(())
}

/// Directories that `files` require under `game_dir` but that don't exist yet,
/// deepest first so they can be removed in that order later.
fn missing_dirs(game_dir: &Path, files: &[PathBuf]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    for rel in files {
        let Some(parent) = rel.parent() else {
            continue;
        };
        let mut current = PathBuf::new();
        for part in parent.components() {
            current.push(part);
            let as_string = current.to_string_lossy().to_string();
            if as_string.is_empty() {
                continue;
            }
            if !game_dir.join(&current).exists() && !dirs.contains(&as_string) {
                dirs.push(as_string);
            }
        }
    }
    // Deepest first.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.matches(['\\', '/']).count()));
    dirs
}

/// Install (or repair) the downloaded online fix into `game_dir`.
///
/// Files already present are backed up once into a zip before being
/// overwritten, so [`uninstall`] can restore the game to its pre-patch
/// state. That first backup is never overwritten by a repair: when the game
/// moved since the last install, the previous folder's backup is parked
/// under an injective name carrying that folder, with a companion file
/// recording the exact path — and a game that returns to a folder it was
/// patched in before reuses that folder's backup, verified against the
/// companion (never on the name alone), instead of recording patched files
/// as the originals. At every point where this function returns early, the
/// state on disk points only at backups that exist.
pub fn install(lib: &Path, app_id: &str, game_dir: &Path, password: Option<&str>) -> Result<FixReport> {
    let archive_file = archive_path(lib, app_id);
    if !archive_file.is_file() {
        bail!("archive du patch introuvable — téléchargez-le d'abord");
    }
    if !game_dir.is_dir() {
        bail!("dossier du jeu introuvable — installez le jeu via Steam avant d'appliquer le patch");
    }

    // Unique per call: two installs must never share a staging folder. The
    // staging lives *inside* the game folder on purpose: that folder sits under
    // `steamapps\common`, which the user excludes from Defender once. Extracting
    // here (instead of %TEMP%) means the patch files never land in a folder the
    // antivirus still scans, so no second exclusion is needed — and the staging
    // dir is removed afterwards, leaving the game folder clean.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ticket = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = game_dir.join(format!(".lv_fix_staging_{}_{ticket}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    archive::extract(&archive_file, &temp, password)?;

    let root = archive::payload_root(&temp);
    let files = archive::list_files(&root)?;
    if files.is_empty() {
        let _ = std::fs::remove_dir_all(&temp);
        bail!("le patch ne contient aucun fichier exploitable");
    }

    let previous = load_state(lib, app_id);
    let backup = backup_path(lib, app_id);
    let dir_str = game_dir.display().to_string();
    let moved = previous
        .as_ref()
        .is_some_and(|s| s.game_dir != dir_str);

    if let Some(prev) = previous.as_ref().filter(|_| moved) {
        // The game changed folders. The zip at `backup` protects the OLD
        // one: park it under a name that says which, instead of letting the
        // new install overwrite it — the old folder may still hold patched
        // files, and the zip is the only way to give it back its originals.
        // The state is written BEFORE the rename: should the install fail
        // anywhere after this (new backup zip, file copies, hashing), what
        // is on disk still points at a backup that exists, and an uninstall
        // still restores the old folder instead of silently restoring
        // nothing while the parked zip turns orphan.
        if backup.is_file() {
            let target = next_free_moved_backup(lib, app_id, &prev.game_dir);
            let mut parked = prev.clone();
            parked.backup_zip = Some(target.display().to_string());
            save_state(lib, &parked)?;
            write_companion(&target, &prev.game_dir)?;
            std::fs::rename(&backup, &target)
                .context("conservation de la sauvegarde de l'ancien dossier du jeu")?;
        }
    }

    // A game that returns to a folder it was patched in before finds its own
    // backup parked there: promote it back to the active name. Backing up
    // the current files instead would record patched files as the originals.
    // [`find_parked_backup`] verifies the companion — a zip whose companion
    // is missing or names another folder is never promoted, so a collision
    // or a manual rename cannot substitute another folder's originals.
    let mut promoted = false;
    if !backup.is_file() {
        if let Some(parked) = find_parked_backup(lib, app_id, &dir_str) {
            std::fs::rename(&parked, &backup)
                .context("récupération de la sauvegarde du dossier du jeu")?;
            let _ = std::fs::remove_file(companion_path(&parked));
            promoted = true;
        }
    }

    // Keep the very first backup for this folder: it alone holds the
    // untouched originals.
    let (backup_zip, backed_up) = if promoted {
        // The folder was patched before, and its originals are already inside.
        (Some(backup.display().to_string()), Vec::new())
    } else if let Some(state) = previous
        .as_ref()
        .filter(|s| s.game_dir == dir_str && backup.is_file())
    {
        (state.backup_zip.clone(), state.backed_up.clone())
    } else if backup.is_file() {
        // A backup sits in the active slot that no state claims: adopt it
        // rather than overwriting the only copy of some folder's originals.
        (Some(backup.display().to_string()), Vec::new())
    } else {
        let existing: Vec<PathBuf> = files
            .iter()
            .filter(|rel| game_dir.join(rel).is_file())
            .cloned()
            .collect();
        let count = archive::zip_files(game_dir, &existing, &backup)?;
        let names = existing
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        if count == 0 {
            let _ = std::fs::remove_file(&backup);
            (None, names)
        } else {
            (Some(backup.display().to_string()), names)
        }
    };

    let created_dirs = previous
        .as_ref()
        .filter(|s| s.game_dir == game_dir.display().to_string())
        .map(|s| s.created_dirs.clone())
        .unwrap_or_else(|| missing_dirs(game_dir, &files));

    let mut records = Vec::with_capacity(files.len());
    for rel in &files {
        let src = root.join(rel);
        let dst = game_dir.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).context("création d'un dossier dans le jeu")?;
        }
        std::fs::copy(&src, &dst)
            .with_context(|| format!("copie de {} vers le jeu", rel.display()))?;
        records.push(FixFile {
            rel: rel.to_string_lossy().to_string(),
            sha256: archive::sha256_file(&dst)?,
            size: std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0),
        });
    }
    let _ = std::fs::remove_dir_all(&temp);

    let state = FixState {
        app_id: app_id.to_string(),
        game_dir: game_dir.display().to_string(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        files: records,
        backup_zip,
        backed_up,
        created_dirs,
    };
    save_state(lib, &state)?;
    Ok(verify(lib, app_id, Some(game_dir)))
}

/// Check that every file the fix installed is still present and unmodified.
pub fn verify(lib: &Path, app_id: &str, current_game_dir: Option<&Path>) -> FixReport {
    let Some(state) = load_state(lib, app_id) else {
        let mut report = FixReport::empty(app_id);
        // Nothing of ours is installed — but the game may already be patched.
        if let Some(dir) = current_game_dir.filter(|d| d.is_dir()) {
            report.foreign = detect_foreign(dir);
            report.game_dir = Some(dir.display().to_string());
        }
        return report;
    };
    let recorded_dir = PathBuf::from(&state.game_dir);
    let mut report = FixReport {
        app_id: app_id.to_string(),
        health: FixHealth::Healthy,
        installed_at: Some(state.installed_at.clone()),
        game_dir: Some(state.game_dir.clone()),
        file_count: state.files.len(),
        missing: Vec::new(),
        modified: Vec::new(),
        has_backup: state.backup_zip.as_deref().map(|p| Path::new(p).is_file()) == Some(true),
        foreign: Vec::new(),
    };

    if let Some(current) = current_game_dir {
        if current != recorded_dir {
            report.health = FixHealth::GameMoved;
            return report;
        }
    }
    if !recorded_dir.is_dir() {
        report.health = FixHealth::GameMoved;
        return report;
    }

    // Parallel SHA-256 verification, preserving original file order.
    let files = &state.files;
    if files.is_empty() {
        // No files to check — nothing to hash.
    } else if files.len() == 1 {
        // Single file: no thread creation needed.
        let file = &files[0];
        let path = recorded_dir.join(&file.rel);
        if !path.is_file() {
            report.missing.push(file.rel.clone());
        } else if archive::sha256_file(&path).ok().as_deref() != Some(file.sha256.as_str()) {
            report.modified.push(file.rel.clone());
        }
    } else {
        // Distribute files across available cores (capped at 8), preserving order.
        let n_threads = std::thread::available_parallelism()
            .map(|n| std::cmp::min(n.get(), 8))
            .unwrap_or(1);

        // Split into chunks by index ranges.
        let chunk_size = files.len().div_ceil(n_threads);
        let mut chunks: Vec<Vec<&FixFile>> = Vec::with_capacity(n_threads);
        for chunk in files.chunks(chunk_size) {
            // chunk is &[FixFile], we need Vec<&FixFile>
            chunks.push(chunk.iter().collect());
        }

        // Each thread returns (original_index, status) so we can reassemble
        // by original file order — immune to duplicate rel paths.
        let mut results: Vec<(usize, &str)> = Vec::with_capacity(files.len());

        std::thread::scope(|s| {
            let handles: Vec<_> = chunks
                .into_iter()
                .enumerate()
                .map(|(ci, chunk)| {
                    let game_dir = recorded_dir.clone();
                    s.spawn(move || {
                        let mut local = Vec::new();
                        for (i, file) in chunk.iter().enumerate() {
                            let idx = ci * chunk_size + i;
                            let path = game_dir.join(&file.rel);
                            if !path.is_file() {
                                local.push((idx, "missing"));
                            } else if archive::sha256_file(&path).ok().as_deref()
                                != Some(file.sha256.as_str())
                            {
                                local.push((idx, "modified"));
                            }
                        }
                        local
                    })
                })
                .collect();
            for handle in handles {
                results.extend(handle.join().unwrap());
            }
        });

        // Sort by original index to restore file order.
        results.sort_by_key(|(idx, _)| *idx);
        for (idx, status) in results {
            match status {
                "missing" => report.missing.push(files[idx].rel.clone()),
                "modified" => report.modified.push(files[idx].rel.clone()),
                _ => {}
            }
        }
    }
    if !report.missing.is_empty() || !report.modified.is_empty() {
        report.health = FixHealth::Damaged;
    }
    report
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallReport {
    pub removed: usize,
    pub restored: usize,
    pub game_dir: String,
}

/// Remove every file the fix installed and put the originals back.
pub fn uninstall(lib: &Path, app_id: &str) -> Result<UninstallReport> {
    let Some(state) = load_state(lib, app_id) else {
        bail!("aucun patch installé n'est enregistré pour ce jeu");
    };
    let game_dir = PathBuf::from(&state.game_dir);
    if !game_dir.is_dir() {
        bail!(
            "dossier du jeu introuvable ({}) — impossible de désinstaller proprement",
            state.game_dir
        );
    }

    let mut removed = 0usize;
    for file in &state.files {
        let path = game_dir.join(&file.rel);
        if path.is_file() && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }

    // Put back the originals we overwrote.
    let mut restored = 0usize;
    let consumed_backup = state.backup_zip.clone();
    if let Some(zip) = consumed_backup.as_deref().map(Path::new) {
        if zip.is_file() {
            restored = archive::unzip_all(zip, &game_dir)
                .context("restauration de la sauvegarde d'avant patch")?
                .len();
        }
    }

    // Drop folders the fix created, deepest first, and only while empty.
    for dir in &state.created_dirs {
        let path = game_dir.join(dir);
        let is_empty = std::fs::read_dir(&path).map(|mut d| d.next().is_none());
        if path.is_dir() && is_empty.unwrap_or(false) {
            let _ = std::fs::remove_dir(&path);
        }
    }

    let _ = std::fs::remove_file(state_path(lib, app_id));
    // Remove exactly the backup that was just restored — usually the active
    // one, but it can be a parked zip (`<appid>_pre-fix_<tag>.zip`), which
    // dies with its companion so no orphan outlives the state that pointed
    // at it. Whatever sits in the active slot WITHOUT being claimed by the
    // state is left alone: it may be an interrupted install's fresh backup
    // for another folder, and the next install adopts it instead of
    // recording patched files as originals.
    if let Some(zip) = consumed_backup.as_deref().map(Path::new) {
        let _ = std::fs::remove_file(zip);
        let _ = std::fs::remove_file(companion_path(zip));
    }

    Ok(UninstallReport {
        removed,
        restored,
        game_dir: state.game_dir,
    })
}

/// Forget a fix without touching the game (used by wipes when the game is gone).
pub fn forget(lib: &Path, app_id: &str) {
    let _ = std::fs::remove_file(state_path(lib, app_id));
    let _ = std::fs::remove_file(backup_path(lib, app_id));
}

/// AppIDs that currently have a recorded fix installation.
pub fn installed_app_ids(lib: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(fixes_dir(lib)) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_suffix(".state.json")
                .map(str::to_string)
        })
        .collect()
}

/// Build a password-protected zip that stands in for a real online-fix .rar.
/// Crate-visible so the bulk-pass tests in `commands.rs` build the same fixture.
#[cfg(test)]
pub(crate) fn fake_fix_archive(path: &Path, entries: &[(&str, &[u8])]) {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .with_aes_encryption(zip::AesMode::Aes256, "testpass");
    for (name, data) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ast_fix_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Each test gets its own scratch tree — they run concurrently.
    fn setup(tag: &str) -> (PathBuf, PathBuf) {
        let root = scratch(tag);
        let lib = root.join("lib");
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        fake_fix_archive(
            &archive_path(&lib, "42"),
            &[
                ("steam_api64.dll", b"patched"),
                ("OnlineFix/OnlineFix64.dll", b"newdll"),
            ],
        );
        (lib, game)
    }

    /// Extract one entry of a zip so a test can check what a backup holds.
    fn zipped_content(zip: &Path, dest: &Path, rel: &str) -> Vec<u8> {
        let extracted = archive::unzip_all(zip, dest).unwrap();
        assert!(
            extracted.iter().any(|p| p.to_string_lossy() == rel),
            "{} should contain {rel}, got {extracted:?}",
            zip.display()
        );
        std::fs::read(dest.join(rel)).unwrap()
    }

    /// Two Steam libraries that differ **only by their drive letter** — the
    /// dominant `fix_game_moved` case (a game moved from one disk to the
    /// other) and exactly the pair a tag truncated to its tail cannot tell
    /// apart: the heads differ, the tails are identical.
    fn two_drive_libraries(root: &Path) -> (PathBuf, PathBuf) {
        let common = |drive: &str| {
            root.join(drive)
                .join("SteamLibrary")
                .join("steamapps")
                .join("common")
                .join("The Elder Scrolls V Skyrim Special Edition")
        };
        let (drive_d, drive_e) = (common("D"), common("E"));
        std::fs::create_dir_all(&drive_d).unwrap();
        std::fs::create_dir_all(&drive_e).unwrap();
        (drive_d, drive_e)
    }

    /// The names parked backups end up under, without their extension.
    fn parked_survivors(lib: &Path) -> Vec<String> {
        std::fs::read_dir(fixes_dir(lib).join("backups"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("42_pre-fix_") && n.ends_with(".zip"))
            .collect()
    }

    #[test]
    fn install_backs_up_originals_and_uninstall_restores_them() {
        let (lib, game) = setup("install_undo");
        std::fs::write(game.join("steam_api64.dll"), b"original").unwrap();

        let report = install(&lib, "42", &game, Some("testpass")).unwrap();
        assert_eq!(report.health, FixHealth::Healthy);
        assert_eq!(report.file_count, 2);
        assert!(report.has_backup);
        assert_eq!(std::fs::read(game.join("steam_api64.dll")).unwrap(), b"patched");
        assert!(game.join("OnlineFix").join("OnlineFix64.dll").is_file());

        let undo = uninstall(&lib, "42").unwrap();
        assert_eq!(undo.removed, 2);
        assert_eq!(undo.restored, 1);
        // Overwritten file is back to its original content…
        assert_eq!(std::fs::read(game.join("steam_api64.dll")).unwrap(), b"original");
        // …and the folder the fix created is gone.
        assert!(!game.join("OnlineFix").exists());
        assert_eq!(verify(&lib, "42", Some(&game)).health, FixHealth::NotInstalled);

        let _ = std::fs::remove_dir_all(game.parent().unwrap());
    }

    #[test]
    fn verify_reports_missing_and_modified_files() {
        let (lib, game) = setup("verify_damage");
        install(&lib, "42", &game, Some("testpass")).unwrap();

        std::fs::remove_file(game.join("steam_api64.dll")).unwrap();
        std::fs::write(game.join("OnlineFix").join("OnlineFix64.dll"), b"tampered").unwrap();

        let report = verify(&lib, "42", Some(&game));
        assert_eq!(report.health, FixHealth::Damaged);
        assert_eq!(report.missing, vec!["steam_api64.dll".to_string()]);
        assert_eq!(report.modified.len(), 1);

        // Re-installing repairs it without losing the original backup.
        let repaired = install(&lib, "42", &game, Some("testpass")).unwrap();
        assert_eq!(repaired.health, FixHealth::Healthy);

        let _ = std::fs::remove_dir_all(game.parent().unwrap());
    }

    /// The LOT-15 trap: repairing a `fix_game_moved` used to write the new
    /// folder's backup over the old folder's, at the same path. When the old
    /// folder still exists with patched files, that silently destroyed the
    /// only way to restore it — and a bulk repair did it to several games at
    /// once. The old backup must survive under a name carrying its folder.
    ///
    /// The two folders here differ **only by their drive letter**, the
    /// dominant move case: with a tag truncated to its tail they hash to the
    /// same name, A's parked zip is mis-promoted as B's own, and B ends up
    /// restored from **A's** originals.
    #[test]
    fn repairing_a_moved_game_keeps_the_old_folders_backup() {
        let (lib, _setup_game) = setup("moved_backup");
        let root = lib.parent().unwrap().to_path_buf();
        let (game_a, game_b) = two_drive_libraries(&root);

        // The parked names must not collide — the whole flow falls over if they do.
        assert_ne!(
            moved_backup_path(&lib, "42", &game_a.display().to_string()),
            moved_backup_path(&lib, "42", &game_b.display().to_string()),
            "deux bibliothèques qui ne diffèrent que par la lettre de lecteur ne doivent pas partager le même nom de sauvegarde"
        );

        std::fs::write(game_a.join("steam_api64.dll"), b"original-A").unwrap();

        // First install in folder A: A's originals fill <appid>_pre-fix.zip.
        install(&lib, "42", &game_a, Some("testpass")).unwrap();

        // The game "moves": Steam reinstalls it on the other disk while
        // folder A stays behind with its patched files.
        std::fs::write(game_b.join("steam_api64.dll"), b"original-B").unwrap();

        let report = install(&lib, "42", &game_b, Some("testpass")).unwrap();
        assert_eq!(report.health, FixHealth::Healthy);

        // The active backup now holds B's originals — not A's mis-promoted ones.
        let dest_new = root.join("unzip-new");
        assert_eq!(
            zipped_content(&backup_path(&lib, "42"), &dest_new, "steam_api64.dll"),
            b"original-B",
            "la sauvegarde active doit contenir les originaux de B, pas ceux de A"
        );

        // …and A's backup survived under a name of its own — overwriting it
        // would have destroyed the only way to restore folder A.
        let backups = fixes_dir(&lib).join("backups");
        let survivors = parked_survivors(&lib);
        assert_eq!(
            survivors.len(),
            1,
            "l'ancienne sauvegarde doit survivre sous un nom qui porte son dossier"
        );
        assert_eq!(
            zipped_content(&backups.join(&survivors[0]), &root.join("unzip-old"), "steam_api64.dll"),
            b"original-A"
        );
        // The companion names folder A exactly — that is what a later
        // promotion checks before trusting the zip.
        assert_eq!(
            std::fs::read(companion_path(&backups.join(&survivors[0]))).unwrap(),
            game_a.display().to_string().as_bytes()
        );

        // Uninstall restores B's originals from the active backup — never
        // A's, which would leave B broken and A without a restore path.
        let undo = uninstall(&lib, "42").unwrap();
        assert_eq!(undo.restored, 1);
        assert_eq!(
            std::fs::read(game_b.join("steam_api64.dll")).unwrap(),
            b"original-B",
            "B doit recevoir SES originaux, pas ceux de A"
        );

        // B's uninstall consumed B's backup but left A's parked zip intact:
        // folder A still holds patched files and keeps its only restore path.
        assert_eq!(parked_survivors(&lib).len(), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A game that moves back to a folder it was patched in before must not
    /// record the (still patched) current files as the originals: that
    /// folder's own backup is promoted back and reused — the promotion
    /// decides on the companion file, never on the name alone.
    #[test]
    fn moving_back_reuses_the_folders_own_backup() {
        let (lib, _setup_game) = setup("moved_back");
        let root = lib.parent().unwrap().to_path_buf();
        let (game_a, game_b) = two_drive_libraries(&root);

        std::fs::write(game_a.join("steam_api64.dll"), b"original-A").unwrap();
        install(&lib, "42", &game_a, Some("testpass")).unwrap();

        // Move to the other disk, repair there…
        std::fs::write(game_b.join("steam_api64.dll"), b"original-B").unwrap();
        install(&lib, "42", &game_b, Some("testpass")).unwrap();

        // …then the game comes back to A, whose files are still patched.
        let report = install(&lib, "42", &game_a, Some("testpass")).unwrap();
        assert_eq!(report.health, FixHealth::Healthy);

        assert_eq!(
            zipped_content(&backup_path(&lib, "42"), &root.join("unzip-back"), "steam_api64.dll"),
            b"original-A",
            "la sauvegarde du dossier A doit être réutilisée, pas remplacée par des fichiers déjà patchés"
        );
        // The promotion consumed A's parked zip and its companion.
        let parked_a = moved_backup_path(&lib, "42", &game_a.display().to_string());
        assert!(!parked_a.is_file());
        assert!(!companion_path(&parked_a).exists());

        // B's backup survived the return move, parked under B's name with
        // B's exact path in its companion.
        let backups = fixes_dir(&lib).join("backups");
        let survivors = parked_survivors(&lib);
        assert_eq!(survivors.len(), 1);
        assert_eq!(
            std::fs::read(companion_path(&backups.join(&survivors[0]))).unwrap(),
            game_b.display().to_string().as_bytes()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The anti-collision loop is the whole point of "next free": a parked
    /// backup must never be overwritten, only worked around.
    #[test]
    fn next_free_moved_backup_never_overwrites_a_parked_backup() {
        let (lib, _setup_game) = setup("next_free");
        let root = lib.parent().unwrap().to_path_buf();
        let (game_a, _) = two_drive_libraries(&root);
        let dir_a = game_a.display().to_string();

        let base = moved_backup_path(&lib, "42", &dir_a);
        std::fs::create_dir_all(base.parent().unwrap()).unwrap();
        std::fs::write(&base, b"first parked backup").unwrap();

        // The base name is taken: the next free one is the `_2` variant…
        let target = next_free_moved_backup(&lib, "42", &dir_a);
        assert_ne!(target, base);
        assert!(
            target
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("_2.zip"),
            "la variante doit porter le suffixe _2, got {}",
            target.display()
        );
        // …it does not exist yet (creating it would be the overwrite)…
        assert!(!target.exists());
        // …and the existing backup is untouched.
        assert_eq!(std::fs::read(&base).unwrap(), b"first parked backup");

        // One level down the same holds.
        std::fs::write(&target, b"second parked backup").unwrap();
        let third = next_free_moved_backup(&lib, "42", &dir_a);
        assert!(
            third
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("_3.zip")
        );
        assert!(!third.exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A backup whose base name is squatted by an impostor (missing or
    /// lying companion) still gets found in its `_2` variant: the
    /// promotion follows the companion, not the base name.
    #[test]
    fn promotion_finds_a_parked_backup_that_fell_to_a_variant() {
        let (lib, _setup_game) = setup("promote_variant");
        let root = lib.parent().unwrap().to_path_buf();
        let (game_a, game_b) = two_drive_libraries(&root);
        let dir_b = game_b.display().to_string();

        let base = moved_backup_path(&lib, "42", &dir_b);
        std::fs::create_dir_all(base.parent().unwrap()).unwrap();

        // Squatter on the base name: a zip whose companion names ANOTHER folder.
        let src_impostor = root.join("src-impostor");
        std::fs::create_dir_all(&src_impostor).unwrap();
        std::fs::write(src_impostor.join("steam_api64.dll"), b"impostor").unwrap();
        archive::zip_files(
            &src_impostor,
            &[PathBuf::from("steam_api64.dll")],
            &base,
        )
        .unwrap();
        write_companion(&base, &game_a.display().to_string()).unwrap();

        // The real backup for B fell to the `_2` variant, companion correct.
        let variant = base.with_file_name(format!(
            "{}_2.zip",
            base.file_stem().unwrap().to_string_lossy()
        ));
        let src_real = root.join("src-real");
        std::fs::create_dir_all(&src_real).unwrap();
        std::fs::write(src_real.join("steam_api64.dll"), b"original-B").unwrap();
        archive::zip_files(&src_real, &[PathBuf::from("steam_api64.dll")], &variant).unwrap();
        write_companion(&variant, &dir_b).unwrap();

        // B's folder is patched by hand; installing must promote the variant
        // (never the squatter) instead of recording patched files as originals.
        std::fs::write(game_b.join("steam_api64.dll"), b"patched-by-hand").unwrap();
        let report = install(&lib, "42", &game_b, Some("testpass")).unwrap();
        assert_eq!(report.health, FixHealth::Healthy);

        assert_eq!(
            zipped_content(&backup_path(&lib, "42"), &root.join("unzip-variant"), "steam_api64.dll"),
            b"original-B",
            "c'est la variante accompagnée du bon dossier qui doit être promue"
        );
        // The squatter was left alone, the variant consumed.
        assert!(base.is_file());
        assert!(!variant.is_file());

        // Uninstall gives B back the variant's originals.
        let undo = uninstall(&lib, "42").unwrap();
        assert_eq!(undo.restored, 1);
        assert_eq!(
            std::fs::read(game_b.join("steam_api64.dll")).unwrap(),
            b"original-B"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A companion that lies must never be trusted: a zip whose companion
    /// names another folder is not promoted, even when it sits under this
    /// folder's own name.
    #[test]
    fn promotion_refuses_a_zip_whose_companion_names_another_folder() {
        let (lib, _setup_game) = setup("refuse_promotion");
        let root = lib.parent().unwrap().to_path_buf();
        let (game_a, game_b) = two_drive_libraries(&root);
        let dir_b = game_b.display().to_string();

        // A parked zip under B's name, but its companion says A.
        let squat = moved_backup_path(&lib, "42", &dir_b);
        std::fs::create_dir_all(squat.parent().unwrap()).unwrap();
        let src = root.join("src-squat");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("steam_api64.dll"), b"original-A").unwrap();
        archive::zip_files(&src, &[PathBuf::from("steam_api64.dll")], &squat).unwrap();
        write_companion(&squat, &game_a.display().to_string()).unwrap();

        std::fs::write(game_b.join("steam_api64.dll"), b"virgin-B").unwrap();
        install(&lib, "42", &game_b, Some("testpass")).unwrap();

        // No promotion: the active backup holds B's own pre-install files…
        assert_eq!(
            zipped_content(&backup_path(&lib, "42"), &root.join("unzip-refused"), "steam_api64.dll"),
            b"virgin-B",
            "une sauvegarde au compagnon menteur ne doit jamais être promue"
        );
        // …and the lying zip stayed parked where it was.
        assert!(squat.is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The window between parking the old backup and the final `save_state`
    /// used to span the new backup zip, every file copy and every hash. A
    /// failure in there left `state.backup_zip` pointing at a path that no
    /// longer existed: uninstall then silently restored zero file and the
    /// parked zip turned orphan. The state is now written before the rename,
    /// so any failure after the park leaves a restorable state.
    #[test]
    fn a_failure_after_parking_leaves_a_restorable_state() {
        let (lib, _setup_game) = setup("fail_window");
        let root = lib.parent().unwrap().to_path_buf();
        let (game_a, game_b) = two_drive_libraries(&root);

        std::fs::write(game_a.join("steam_api64.dll"), b"original-A").unwrap();
        install(&lib, "42", &game_a, Some("testpass")).unwrap();

        // Folder B is rigged so the install fails AFTER the park: a file
        // squatting where the fix needs to create a directory.
        std::fs::write(game_b.join("steam_api64.dll"), b"original-B").unwrap();
        std::fs::write(game_b.join("OnlineFix"), b"squatter").unwrap();
        install(&lib, "42", &game_b, Some("testpass")).unwrap_err();

        // The state on disk points at a backup that exists.
        let state = load_state(&lib, "42").expect("l'état doit survivre à l'échec");
        let zip = PathBuf::from(
            state
                .backup_zip
                .expect("l'état doit pointer vers la sauvegarde parquée"),
        );
        assert!(
            zip.is_file(),
            "backup_zip doit pointer vers un fichier existant, pas vers un chemin renommé : {}",
            zip.display()
        );

        // Uninstall restores folder A exactly — no silent zero-restore.
        let undo = uninstall(&lib, "42").unwrap();
        assert_eq!(undo.restored, 1, "la restauration silencieuse de zéro fichier est le bug refermé ici");
        assert_eq!(
            std::fs::read(game_a.join("steam_api64.dll")).unwrap(),
            b"original-A"
        );

        // No orphan: the consumed parked zip is gone with the state that
        // pointed at it…
        assert!(parked_survivors(&lib).is_empty(), "aucun zip taggé orphelin ne doit survivre à l'état");
        // …and B's unclaimed fresh backup survived, so a retry of install(B)
        // adopts B's true originals instead of recording patched files.
        assert_eq!(
            zipped_content(&backup_path(&lib, "42"), &root.join("unzip-leftover"), "steam_api64.dll"),
            b"original-B"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn verify_flags_a_moved_game() {
        let (lib, game) = setup("verify_moved");
        install(&lib, "42", &game, Some("testpass")).unwrap();
        let elsewhere = game.parent().unwrap().join("moved");
        std::fs::create_dir_all(&elsewhere).unwrap();
        assert_eq!(
            verify(&lib, "42", Some(&elsewhere)).health,
            FixHealth::GameMoved
        );
        let _ = std::fs::remove_dir_all(game.parent().unwrap());
    }

    #[test]
    fn install_refuses_when_the_game_is_not_installed() {
        let (lib, game) = setup("no_game");
        let absent = game.parent().unwrap().join("nope");
        let err = install(&lib, "42", &absent, Some("testpass")).unwrap_err();
        assert!(err.to_string().contains("installez le jeu via Steam"));
        let _ = std::fs::remove_dir_all(game.parent().unwrap());
    }

    #[test]
    fn verify_flags_a_fix_applied_outside_the_app() {
        let root = scratch("foreign");
        let lib = root.join("lib");
        let game = root.join("game");
        std::fs::create_dir_all(game.join("OnlineFix")).unwrap();
        std::fs::write(game.join("OnlineFix").join("OnlineFix64.dll"), b"x").unwrap();
        std::fs::write(game.join("OnlineFix.ini"), b"x").unwrap();

        let report = verify(&lib, "42", Some(&game));
        // We installed nothing, yet the game is patched — say so.
        assert_eq!(report.health, FixHealth::NotInstalled);
        assert_eq!(
            report.foreign,
            vec!["OnlineFix.ini".to_string(), "OnlineFix\\OnlineFix64.dll".to_string()]
        );

        // A clean game folder reports nothing.
        let clean = root.join("clean");
        std::fs::create_dir_all(&clean).unwrap();
        assert!(verify(&lib, "42", Some(&clean)).foreign.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_refuses_without_a_downloaded_archive() {
        let root = scratch("noarchive");
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        let err = install(&root.join("lib"), "7", &game, Some("testpass")).unwrap_err();
        assert!(err.to_string().contains("téléchargez-le d'abord"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Parallel hashing with 16 files spread across multiple threads,
    /// including missing and modified files. Verifies that the output
    /// preserves original file order and contains exactly the right names.
    #[test]
    fn verify_parallel_hash_many_files() {
        let root = scratch("verify_parallel");
        let lib = root.join("lib");
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();

        // Build 16 files: 4 missing, 4 modified, 8 healthy.
        let mut expected_missing = Vec::new();
        let mut expected_modified = Vec::new();

        // Helper: compute SHA-256 of a byte slice (same logic as archive::sha256_file).
        fn sha256_bytes(data: &[u8]) -> String {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(data);
            format!("{:x}", hasher.finalize())
        }

        for i in 0..16 {
            let rel = format!("file_{:02}.dat", i);
            let content = format!("content-{i:04x}");
            let content_bytes = content.as_bytes();

            if i < 4 {
                // Missing files (don't write them).
                expected_missing.push(rel.clone());
            } else if i < 8 {
                // Modified files (write wrong content).
                expected_modified.push(rel.clone());
                std::fs::write(game.join(&rel), b"wrong").unwrap();
            } else {
                // Healthy files.
                std::fs::write(game.join(&rel), content_bytes).unwrap();
            }
        }

        // Build a state.json that records all 16 files with their correct hashes.
        let files: Vec<FixFile> = (0..16)
            .map(|i| {
                let rel = format!("file_{:02}.dat", i);
                let content = format!("content-{i:04x}");
                let content_bytes = content.as_bytes();
                FixFile {
                    rel,
                    sha256: sha256_bytes(content_bytes),
                    size: content_bytes.len() as u64,
                }
            })
            .collect();

        let state = FixState {
            app_id: "42".to_string(),
            game_dir: game.display().to_string(),
            installed_at: chrono::Utc::now().to_rfc3339(),
            files,
            backup_zip: None,
            backed_up: Vec::new(),
            created_dirs: Vec::new(),
        };

        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(fixes_dir(&lib)).unwrap();
        let state_path = state_path(&lib, "42");
        std::fs::write(
            &state_path,
            serde_json::to_string_pretty(&state).unwrap(),
        )
        .unwrap();

        let report = verify(&lib, "42", Some(&game));

        assert_eq!(report.health, FixHealth::Damaged);
        assert_eq!(report.file_count, 16);

        // Check missing — must be in original order.
        assert_eq!(report.missing.len(), 4);
        for (i, expected) in expected_missing.iter().enumerate() {
            assert_eq!(&report.missing[i], expected);
        }

        // Check modified — must be in original order.
        assert_eq!(report.modified.len(), 4);
        for (i, expected) in expected_modified.iter().enumerate() {
            assert_eq!(&report.modified[i], expected);
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}

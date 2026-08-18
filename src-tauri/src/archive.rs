//! Archive extraction for online-fix payloads (RAR/ZIP, password protected).

use anyhow::{anyhow, bail, Context, Result};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Rar,
    Zip,
    SevenZ,
    Unknown,
}

/// Identify the archive by magic bytes.
pub fn detect_kind(path: &Path) -> ArchiveKind {
    let mut head = [0u8; 8];
    let Ok(mut file) = std::fs::File::open(path) else {
        return ArchiveKind::Unknown;
    };
    let Ok(read) = file.read(&mut head) else {
        return ArchiveKind::Unknown;
    };
    let head = &head[..read];

    if head.starts_with(b"Rar!\x1a\x07") {
        ArchiveKind::Rar
    } else if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        ArchiveKind::Zip
    } else if head.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        ArchiveKind::SevenZ
    } else {
        ArchiveKind::Unknown
    }
}

/// Check if the archive entries require a password.
pub fn needs_password(archive: &Path) -> bool {
    match detect_kind(archive) {
        ArchiveKind::Zip => {
            let Ok(file) = std::fs::File::open(archive) else { return false; };
            let Ok(mut zip) = zip::ZipArchive::new(file) else { return false; };
            for i in 0..zip.len() {
                if let Ok(entry) = zip.by_index_raw(i) {
                    if entry.encrypted() {
                        return true;
                    }
                }
            }
            false
        }
        ArchiveKind::Rar => {
            let Ok(mut open) = unrar::Archive::new(archive).open_for_processing() else { return false; };
            while let Ok(Some(header)) = open.read_header() {
                if header.entry().is_encrypted() {
                    return true;
                }
                open = match header.skip() {
                    Ok(op) => op,
                    Err(_) => break,
                };
            }
            false
        }
        _ => false,
    }
}

/// Extract `archive` into `dest`, transparently handling RAR and ZIP.
/// Returns the relative paths of every extracted file.
pub fn extract(archive: &Path, dest: &Path, password: Option<&str>) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dest).context("création du dossier d'extraction")?;
    let pwd = password.unwrap_or("");
    let res = match detect_kind(archive) {
        ArchiveKind::Rar => extract_rar(archive, dest, pwd),
        ArchiveKind::Zip => extract_zip(archive, dest, pwd),
        ArchiveKind::SevenZ => bail!(
            "archive 7-Zip non prise en charge — extrayez-la manuellement puis réessayez"
        ),
        ArchiveKind::Unknown => bail!(
            "format d'archive non reconnu — le fichier est peut-être corrompu, retéléchargez-le"
        ),
    };
    if let Err(e) = res {
        let msg = e.to_string();
        if needs_password(archive)
            || msg.contains("PasswordIncorrect")
            || msg.contains("mot de passe")
            || msg.contains("déchiffrée")
            || msg.contains("decrypt")
        {
            bail!("PasswordIncorrect: {}", msg);
        }
        return Err(e);
    }
    let files = list_files(dest)?;
    if files.is_empty() {
        bail!("PasswordIncorrect: l'archive est vide ou n'a pas pu être déchiffrée");
    }
    Ok(files)
}

fn extract_rar(archive: &Path, dest: &Path, password: &str) -> Result<()> {
    let mut open = unrar::Archive::with_password(archive, password)
        .open_for_processing()
        .map_err(|e| anyhow!("PasswordIncorrect: ouverture de l'archive RAR: {e}"))?;

    while let Some(header) = open
        .read_header()
        .map_err(|e| anyhow!("PasswordIncorrect: lecture de l'archive RAR (mot de passe incorrect ?): {e}"))?
    {
        let is_file = header.entry().is_file();
        open = if is_file {
            header
                .extract_with_base(dest)
                .map_err(|e| anyhow!("PasswordIncorrect: extraction RAR: {e}"))?
        } else {
            header.skip().map_err(|e| anyhow!("PasswordIncorrect: extraction RAR: {e}"))?
        };
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path, password: &str) -> Result<()> {
    let file = std::fs::File::open(archive).context("ouverture de l'archive ZIP")?;
    let mut zip = zip::ZipArchive::new(file).context("lecture de l'archive ZIP")?;

    for i in 0..zip.len() {
        let is_encrypted = zip.by_index_raw(i).map(|e| e.encrypted()).unwrap_or(false);
        let mut entry = if is_encrypted {
            zip.by_index_decrypt(i, password.as_bytes())
                .with_context(|| format!("PasswordIncorrect: entrée ZIP #{i} illisible (mot de passe incorrect ?)"))?
        } else {
            zip.by_index(i)
                .with_context(|| format!("entrée ZIP #{i} illisible"))?
        };
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).context("création d'un dossier extrait")?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).context("création d'un dossier extrait")?;
        }
        let mut sink = std::fs::File::create(&out)
            .with_context(|| format!("écriture de {}", out.display()))?;
        std::io::copy(&mut entry, &mut sink).context("PasswordIncorrect: décompression d'une entrée ZIP")?;
    }
    Ok(())
}

/// Relative paths of every regular file below `root`, sorted for stable output.
pub fn list_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.context("parcours du dossier extrait")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .context("chemin relatif")?
            .to_path_buf();
        if rel.components().any(|c| matches!(c, Component::ParentDir)) {
            continue;
        }
        files.push(rel);
    }
    files.sort();
    Ok(files)
}

/// Online fixes are inconsistently packed: some put files at the archive root,
/// others wrap everything in a single folder. Peel wrapper folders so callers
/// always get the directory whose contents belong at the game's root.
pub fn payload_root(extracted: &Path) -> PathBuf {
    let mut root = extracted.to_path_buf();
    for _ in 0..4 {
        let Ok(entries) = std::fs::read_dir(&root) else {
            break;
        };
        let children: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        match children.as_slice() {
            [only] if only.path().is_dir() => root = only.path(),
            _ => break,
        }
    }
    root
}

/// Zip `rels` (relative to `base`) into `out`, preserving the relative layout.
/// Missing sources are skipped. Returns the number of files written.
pub fn zip_files(base: &Path, rels: &[PathBuf], out: &Path) -> Result<usize> {
    use std::io::Write;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).context("création du dossier de sauvegarde")?;
    }
    let file = std::fs::File::create(out)
        .with_context(|| format!("création de {}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
        .large_file(true);

    let mut written = 0usize;
    for rel in rels {
        let src = base.join(rel);
        if !src.is_file() {
            continue;
        }
        let name = rel.to_string_lossy().replace('\\', "/");
        zip.start_file(name, options).context("entrée d'archive")?;
        let bytes = std::fs::read(&src).with_context(|| format!("lecture de {}", src.display()))?;
        zip.write_all(&bytes).context("écriture dans l'archive")?;
        written += 1;
    }
    zip.finish().context("finalisation de l'archive")?;
    Ok(written)
}

/// Extract every entry of a plain (unencrypted) zip into `dest`.
/// Returns the relative paths restored.
pub fn unzip_all(archive: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let file = std::fs::File::open(archive)
        .with_context(|| format!("ouverture de {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("lecture de l'archive")?;
    let mut restored = Vec::new();

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("entrée d'archive illisible")?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).context("création d'un dossier restauré")?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).context("création d'un dossier restauré")?;
        }
        let mut sink =
            std::fs::File::create(&out).with_context(|| format!("écriture de {}", out.display()))?;
        std::io::copy(&mut entry, &mut sink).context("restauration d'un fichier")?;
        restored.push(rel);
    }
    Ok(restored)
}

/// Hex SHA-256 of a file — used to tell "fix intact" from "fix altered".
pub fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("lecture de {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf).context("lecture pour empreinte")?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ast_arch_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])], password: Option<&str>) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let mut options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        if let Some(pw) = password {
            options = options.with_aes_encryption(zip::AesMode::Aes256, pw);
        }
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn detects_zip_and_extracts_encrypted_entries() {
        let dir = scratch("zip");
        let archive = dir.join("fix.rar"); // deliberately mislabelled extension
        write_zip(
            &archive,
            &[("OnlineFix64.dll", b"dll"), ("steam_api64.dll", b"api")],
            Some("testpass"),
        );
        assert_eq!(detect_kind(&archive), ArchiveKind::Zip);
        assert!(needs_password(&archive));

        let out = dir.join("out");
        let files = extract(&archive, &out, Some("testpass")).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(std::fs::read(out.join("OnlineFix64.dll")).unwrap(), b"dll");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_password_is_reported_as_password_incorrect() {
        let dir = scratch("wrong-password");
        let archive = dir.join("fix.zip");
        write_zip(&archive, &[("OnlineFix64.dll", b"dll")], Some("testpass"));

        let err = extract(&archive, &dir.join("out"), Some("incorrect")).unwrap_err();
        assert!(
            err.to_string().contains("PasswordIncorrect:"),
            "l'erreur de déchiffrement doit rester distinguable : {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn payload_root_peels_single_wrapper_folder() {
        let dir = scratch("root");
        let nested = dir.join("Online Fix").join("Game");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("a.dll"), b"x").unwrap();
        std::fs::write(nested.join("b.dll"), b"x").unwrap();
        assert_eq!(payload_root(&dir), nested);

        // Two entries at the top level: nothing to peel.
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();
        assert_eq!(payload_root(&dir), dir);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zip_and_unzip_roundtrip_preserves_layout() {
        let dir = scratch("roundtrip");
        let base = dir.join("game");
        std::fs::create_dir_all(base.join("Engine")).unwrap();
        std::fs::write(base.join("steam_api64.dll"), b"original").unwrap();
        std::fs::write(base.join("Engine").join("cfg.ini"), b"conf").unwrap();

        let rels = vec![
            PathBuf::from("steam_api64.dll"),
            PathBuf::from("Engine").join("cfg.ini"),
            PathBuf::from("absent.dll"),
        ];
        let backup = dir.join("backup.zip");
        assert_eq!(zip_files(&base, &rels, &backup).unwrap(), 2);

        std::fs::write(base.join("steam_api64.dll"), b"overwritten").unwrap();
        let restored = unzip_all(&backup, &base).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(std::fs::read(base.join("steam_api64.dll")).unwrap(), b"original");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_detects_content_changes() {
        let dir = scratch("sha");
        let file = dir.join("a.bin");
        std::fs::write(&file, b"one").unwrap();
        let first = sha256_file(&file).unwrap();
        std::fs::write(&file, b"two").unwrap();
        assert_ne!(first, sha256_file(&file).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_format_is_rejected_with_a_clear_message() {
        let dir = scratch("bad");
        let archive = dir.join("broken.rar");
        std::fs::write(&archive, b"not an archive at all").unwrap();
        let err = extract(&archive, &dir.join("out"), None).unwrap_err();
        assert!(err.to_string().contains("non reconnu"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

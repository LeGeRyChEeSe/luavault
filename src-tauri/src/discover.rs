//! Auto-discovery: adopt `.lua` files that are already sitting in
//! `{Steam}\config\lua` but that our library has never heard of.
//!
//! Users routinely arrive with a Steam folder that SteamTools has been feeding
//! by hand. Those games are invisible to every state we derive — so we import
//! them, then let the normal `stage` pipeline verify them like any other entry.

use crate::{config, detect, i18n_log, library};
use anyhow::{Context, Result};
use std::path::Path;

/// Steam's store header — the exact URL LuaVault serves as an entry `icon`,
/// so an adopted game looks identical to a downloaded one.
pub fn header_image(app_id: &str) -> String {
    format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{app_id}/header.jpg")
}

/// AppIDs whose `.lua` currently lives in `{Steam}\config\lua`.
pub fn steam_lua_ids(steam: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(detect::lua_dir(steam)) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let stem = name.strip_suffix(".lua")?;
            // Only numeric stems are AppIDs; SteamTools also drops helper scripts there.
            (!stem.is_empty() && stem.chars().all(|c| c.is_ascii_digit())).then(|| stem.to_string())
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// `.lua` files present in Steam that the library index doesn't know about.
pub fn orphans(lib: &Path, steam: &Path) -> Vec<String> {
    orphans_with_data_dir(lib, &config::data_dir(), steam)
}

/// `orphans` with injected `data_dir` (for tests).
pub fn orphans_with_data_dir(lib: &Path, data_dir: &Path, steam: &Path) -> Vec<String> {
    let Some(known) = known_entries(lib, data_dir) else {
        return Vec::new();
    };
    steam_lua_ids(steam)
        .into_iter()
        .filter(|id| !known.iter().any(|e| &e.app_id == id))
        .collect()
}

/// Numeric-stem `.lua` files sitting in the library directory itself that the
/// index has never recorded — i.e. files the user dropped in by hand.
pub fn library_orphans(lib: &Path) -> Vec<String> {
    library_orphans_with_data_dir(lib, &config::data_dir())
}

/// `library_orphans` with injected `data_dir` (for tests).
pub fn library_orphans_with_data_dir(lib: &Path, data_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(lib) else {
        return Vec::new();
    };
    let Some(known) = known_entries(lib, data_dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let stem = name.strip_suffix(".lua")?;
            (!stem.is_empty() && stem.chars().all(|c| c.is_ascii_digit())).then(|| stem.to_string())
        })
        .filter(|id| !known.iter().any(|e| &e.app_id == id))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Best-effort index read for the orphan scans.
///
/// Returns `Some(entries)` when the index is readable (even if empty),
/// and `None` when the index is unreadable or damaged.
/// In the latter case the caller must not propose any orphan — proposing
/// unknown files on a broken index would rewrite the library from scratch.
fn known_entries(lib: &Path, data_dir: &Path) -> Option<Vec<library::LibraryEntry>> {
    match library::load_index_with_data_dir(lib, data_dir) {
        Ok(entries) => Some(entries),
        Err(e) => {
            log::warn!("{}", i18n_log::i18n_log(format!("discover: index de la bibliothèque ignoré: {e:#}"), "logs.discover.library-index-ignored", &[("error", serde_json::json!(format!("{e:#}")))]));
            None
        }
    }
}

/// Index a `.lua` that already lives in the library directory. Unlike
/// [`adopt`], there is no Steam copy to read — the file is right where it
/// needs to be, so we only need to create the index entry.
pub fn adopt_local(
    lib: &Path,
    app_id: &str,
    name: &str,
    icon: Option<&str>,
) -> Result<library::LibraryEntry> {
    adopt_local_with_data_dir(lib, &config::data_dir(), app_id, name, icon)
}

/// `adopt_local` with injected `data_dir` (for tests).
pub fn adopt_local_with_data_dir(
    lib: &Path,
    data_dir: &Path,
    app_id: &str,
    name: &str,
    icon: Option<&str>,
) -> Result<library::LibraryEntry> {
    let src = lib.join(library::lua_file_name(app_id));
    let bytes = std::fs::read(&src)
        .with_context(|| format!("lecture de {}", src.display()))?;
    library::upsert_with_data_dir(lib, data_dir, app_id, name, icon, &bytes)
}

/// Copy an orphan `.lua` from Steam into the library and index it.
///
/// The Steam copy is the source of truth here — it is what SteamTools actually
/// loads, so adopting it must never overwrite it with something else.
pub fn adopt(
    lib: &Path,
    steam: &Path,
    app_id: &str,
    name: &str,
    icon: Option<&str>,
) -> Result<library::LibraryEntry> {
    adopt_with_data_dir(lib, &config::data_dir(), steam, app_id, name, icon)
}

/// `adopt` with injected `data_dir` (for tests).
pub fn adopt_with_data_dir(
    lib: &Path,
    data_dir: &Path,
    steam: &Path,
    app_id: &str,
    name: &str,
    icon: Option<&str>,
) -> Result<library::LibraryEntry> {
    let src = detect::lua_dir(steam).join(library::lua_file_name(app_id));
    let bytes = std::fs::read(&src)
        .with_context(|| format!("lecture de {}", src.display()))?;
    library::upsert_with_data_dir(lib, data_dir, app_id, name, icon, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ast_discover_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn lists_only_numeric_lua_stems() {
        let steam = scratch("scan");
        let lua = detect::lua_dir(&steam);
        std::fs::create_dir_all(&lua).unwrap();
        std::fs::write(lua.join("264710.lua"), b"x").unwrap();
        std::fs::write(lua.join("848450.lua"), b"x").unwrap();
        std::fs::write(lua.join("helper.lua"), b"x").unwrap();
        std::fs::write(lua.join("readme.txt"), b"x").unwrap();

        assert_eq!(steam_lua_ids(&steam), vec!["264710", "848450"]);
        let _ = std::fs::remove_dir_all(&steam);
    }

    #[test]
    fn orphans_excludes_games_already_indexed_and_adopt_imports_them() {
        let _lock = library::cache_test_lock();
        let root = scratch("adopt");
        let data = root.join("data");
        let lib = root.join("lib");
        let steam = root.join("steam");
        let lua = detect::lua_dir(&steam);
        std::fs::create_dir_all(&lua).unwrap();
        std::fs::write(lua.join("264710.lua"), b"-- from steam").unwrap();
        std::fs::write(lua.join("848450.lua"), b"-- also").unwrap();
        library::upsert_with_data_dir(&lib, &data, "848450", "Below Zero", None, b"-- ours").unwrap();

        assert_eq!(orphans_with_data_dir(&lib, &data, &steam), vec!["264710"]);

        let entry = adopt_with_data_dir(&lib, &data, &steam, "264710", "Subnautica", Some("http://i")).unwrap();
        assert_eq!(entry.name, "Subnautica");
        // The bytes come from Steam, untouched.
        assert_eq!(
            std::fs::read(lib.join("264710.lua")).unwrap(),
            b"-- from steam"
        );
        assert!(orphans_with_data_dir(&lib, &data, &steam).is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn header_image_points_at_the_steam_cdn() {
        assert!(header_image("264710").ends_with("/steam/apps/264710/header.jpg"));
    }

    #[test]
    fn library_orphans_finds_hand_dropped_files() {
        let _lock = library::cache_test_lock();
        let root = scratch("lib_orphans");
        let data = root.join("data");
        let lib = root.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        // Already indexed — must NOT appear.
        library::upsert_with_data_dir(&lib, &data, "848450", "Below Zero", None, b"-- known").unwrap();
        // Hand-dropped, numeric stem — must appear.
        std::fs::write(lib.join("264710.lua"), b"-- dropped").unwrap();
        std::fs::write(lib.join("553850.lua"), b"-- dropped too").unwrap();
        // Non-numeric stem — ignored.
        std::fs::write(lib.join("helper.lua"), b"-- script").unwrap();
        // Not a .lua — ignored.
        std::fs::write(lib.join("notes.txt"), b"hi").unwrap();

        assert_eq!(library_orphans_with_data_dir(&lib, &data), vec!["264710", "553850"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn adopt_local_indexes_a_dropped_file() {
        let _lock = library::cache_test_lock();
        let root = scratch("lib_adopt_local");
        let data = root.join("data");
        let lib = root.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("264710.lua"), b"-- hand-dropped").unwrap();

        assert_eq!(library_orphans_with_data_dir(&lib, &data), vec!["264710"]);

        let entry = adopt_local_with_data_dir(&lib, &data, "264710", "Subnautica", Some("http://icon")).unwrap();
        assert_eq!(entry.app_id, "264710");
        assert_eq!(entry.name, "Subnautica");
        // The file content is preserved.
        assert_eq!(
            std::fs::read(lib.join("264710.lua")).unwrap(),
            b"-- hand-dropped"
        );
        // No longer an orphan once indexed.
        assert!(library_orphans_with_data_dir(&lib, &data).is_empty());

        // Adopting a file that doesn't exist is an error, not a panic.
        assert!(adopt_local_with_data_dir(&lib, &data, "999999", "Ghost", None).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 1 : index illisible (sidecar corrompue) → aucun orphelin Steam.
    /// Avant la correction : la fonction renvoyait le `.lua` comme orphelin.
    #[test]
    fn orphans_unreadable_index_returns_empty() {
        let _lock = library::cache_test_lock();
        let root = scratch("orphans_bad_idx");
        let data = root.join("data");
        let lib = root.join("lib");
        let steam = root.join("steam");
        let lua = detect::lua_dir(&steam);
        std::fs::create_dir_all(&lua).unwrap();
        std::fs::create_dir_all(&lib).unwrap();

        // 1) Crée un index valide et le signe.
        library::upsert_with_data_dir(&lib, &data, "848450", "Below Zero", None, b"-- known").unwrap();

        // 2) Corrompt la sidecar HMAC pour que load_index_with_data_dir échoue.
        //    On écrit directement un tag hex invalide dans le fichier sidecar.
        let sidecar_file = lib.join("index.hmac");
        std::fs::write(&sidecar_file, "LV-HMAC-v1:ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ").unwrap();

        // Vérifie que load_index_with_data_dir échoue bien sur ce montage.
        assert!(library::load_index_with_data_dir(&lib, &data).is_err(), "load_index_with_data_dir doit échouer sur sidecar corrompue");

        // 3) Place un `.lua` inconnu dans Steam.
        std::fs::write(lua.join("264710.lua"), b"-- steam").unwrap();

        // 4) orphans_with_data_dir doit renvoyer une liste vide.
        assert_eq!(
            orphans_with_data_dir(&lib, &data, &steam),
            Vec::<String>::new(),
            "index illisible → aucun orphelin Steam"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 2 : index illisible (sidecar corrompue) → aucun orphelin local.
    #[test]
    fn library_orphans_unreadable_index_returns_empty() {
        let _lock = library::cache_test_lock();
        let root = scratch("lib_orphans_bad_idx");
        let data = root.join("data");
        let lib = root.join("lib");
        std::fs::create_dir_all(&lib).unwrap();

        // 1) Crée un index valide et le signe.
        library::upsert_with_data_dir(&lib, &data, "848450", "Below Zero", None, b"-- known").unwrap();

        // 2) Corrompt la sidecar HMAC.
        let sidecar_file = lib.join("index.hmac");
        std::fs::write(&sidecar_file, "LV-HMAC-v1:ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ").unwrap();

        // Vérifie que load_index_with_data_dir échoue.
        assert!(library::load_index_with_data_dir(&lib, &data).is_err(), "load_index_with_data_dir doit échouer sur sidecar corrompue");

        // 3) Place un `.lua` numérique dans le dossier bibliothèque.
        std::fs::write(lib.join("264710.lua"), b"-- dropped").unwrap();

        // 4) library_orphans_with_data_dir doit renvoyer une liste vide.
        assert_eq!(
            library_orphans_with_data_dir(&lib, &data),
            Vec::<String>::new(),
            "index illisible → aucun orphelin local"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 3 : index valide et correctement signé → les orphelins sont bien rapportés.
    /// Garde le chemin nominal intact.
    #[test]
    fn orphans_valid_index_still_finds_orphans() {
        let _lock = library::cache_test_lock();
        let root = scratch("orphans_valid");
        let data = root.join("data");
        let lib = root.join("lib");
        let steam = root.join("steam");
        let lua = detect::lua_dir(&steam);
        std::fs::create_dir_all(&lua).unwrap();

        // Index valide avec un jeu connu.
        library::upsert_with_data_dir(&lib, &data, "848450", "Below Zero", None, b"-- known").unwrap();

        // Un `.lua` inconnu dans Steam.
        std::fs::write(lua.join("264710.lua"), b"-- steam").unwrap();

        // Doit être rapporté comme orphelin.
        assert_eq!(
            orphans_with_data_dir(&lib, &data, &steam),
            vec!["264710"],
            "index valide → l'orphelin est détecté"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

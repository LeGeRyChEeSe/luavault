use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PORTABLE_MARKER: &str = "LuaVault.portable";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    /// User-overridden Steam directory (None = auto-detect).
    pub steam_dir: Option<PathBuf>,
    /// User-overridden library directory (None = <data_dir>/library).
    pub library_dir: Option<PathBuf>,
    /// Set once the first-run onboarding has been completed.
    pub first_run_done: bool,
    /// Selected colour theme id (see `lib/themes.ts`). None = the default one.
    pub theme: Option<String>,
    /// Dark mode, independent of the theme. None = follow the system.
    pub dark_mode: Option<bool>,
    /// UI locale. None = the interface infers one from the system language.
    pub locale: Option<String>,
    /// Defender exclusion choice: None = not asked yet, Some(true) = added,
    /// Some(false) = the user declined. Asked once after onboarding.
    pub defender_exclusions: Option<bool>,
    /// Last remote version the user was notified about (toast shown once per version).
    #[serde(default)]
    pub update_notified_version: Option<String>,
    /// Version we were running when an update was launched. Set just before the
    /// installer starts, read once at the next startup, then cleared.
    #[serde(default)]
    pub update_from_version: Option<String>,
    /// Optional default password for extracting online fix archives.
    #[serde(default)]
    pub default_archive_password: Option<String>,
}

/// Result of merging an imported config into a local one.
#[derive(Debug, Clone, Default)]
pub struct ConfigMerge {
    /// The merged configuration to write back.
    pub merged: AppConfig,
    /// Field names for which the local value was kept
    /// ("steam_dir", "library_dir") — only when a local `Some` value
    /// survived because the imported path did not exist.
    /// Empty = everything was imported or no local value existed.
    pub kept_local: Vec<String>,
}

/// Merge an imported [`AppConfig`] into the local one.
///
/// `steam_dir` and `library_dir` are adopted from the import **only if**
/// `exists(path)` returns true for the imported path. Otherwise the local
/// value is kept and the field name is recorded in `kept_local`.
///
/// A `None` imported value never overwrites a `Some` local value — a
/// missing import is not a user decision.
///
/// All other fields are imported as-is.
pub fn merge_imported(
    local: &AppConfig,
    imported: &AppConfig,
    exists: impl Fn(&Path) -> bool,
) -> ConfigMerge {
    let mut kept_local = Vec::new();

    let steam_dir = match (&local.steam_dir, &imported.steam_dir) {
        (Some(local_path), Some(imp_path)) => {
            if exists(imp_path) {
                Some(imp_path.clone())
            } else {
                kept_local.push("steam_dir".to_string());
                Some(local_path.clone())
            }
        }
        (Some(local_path), None) => Some(local_path.clone()),
        (None, Some(imp_path)) => {
            if exists(imp_path) {
                Some(imp_path.clone())
            } else {
                None
            }
        }
        (None, None) => None,
    };

    let library_dir = match (&local.library_dir, &imported.library_dir) {
        (Some(local_path), Some(imp_path)) => {
            if exists(imp_path) {
                Some(imp_path.clone())
            } else {
                kept_local.push("library_dir".to_string());
                Some(local_path.clone())
            }
        }
        (Some(local_path), None) => Some(local_path.clone()),
        (None, Some(imp_path)) => {
            if exists(imp_path) {
                Some(imp_path.clone())
            } else {
                None
            }
        }
        (None, None) => None,
    };

    ConfigMerge {
        merged: AppConfig {
            steam_dir,
            library_dir,
            first_run_done: imported.first_run_done,
            theme: imported.theme.clone(),
            dark_mode: imported.dark_mode,
            locale: imported.locale.clone(),
            defender_exclusions: imported.defender_exclusions,
            update_notified_version: imported.update_notified_version.clone(),
            update_from_version: imported.update_from_version.clone(),
            default_archive_password: imported
                .default_archive_password
                .clone()
                .or_else(|| local.default_archive_password.clone()),
        },
        kept_local,
    }
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if path.exists() {
            let raw = std::fs::read_to_string(&path).context("read config.json")?;
            let cfg: AppConfig = serde_json::from_str(&raw).context("parse config.json")?;
            Ok(cfg)
        } else {
            let cfg = AppConfig::default();
            cfg.save().ok();
            Ok(cfg)
        }
    }

    pub fn save(&self) -> Result<()> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir).context("create data dir")?;
        self.save_to(&config_path())?;
        Ok(())
    }

    /// Serialize this configuration to an explicit path.
    ///
    /// `save` remains the application entry point; this variant keeps disk
    /// persistence testable without writing into the developer's data folder.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let raw = serde_json::to_vec_pretty(self).context("serialize config")?;
        std::fs::write(path, raw).context("write config.json")?;
        Ok(())
    }

    pub fn resolved_library_dir(&self) -> PathBuf {
        self.library_dir
            .clone()
            .unwrap_or_else(|| data_dir().join("library"))
    }
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn is_portable() -> bool {
    exe_dir().join(PORTABLE_MARKER).exists()
}

/// Portable edition: everything lives next to the executable.
/// Installable edition: %LocalAppData%\LuaVault.
pub fn data_dir() -> PathBuf {
    if is_portable() {
        exe_dir()
    } else {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("LuaVault")
    }
}

fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn local_cfg() -> AppConfig {
        AppConfig {
            steam_dir: Some(PathBuf::from("C:\\Steam")),
            library_dir: Some(PathBuf::from("D:\\SteamLib")),
            theme: Some("light".to_string()),
            dark_mode: Some(false),
            locale: None,
            first_run_done: true,
            defender_exclusions: None,
            update_notified_version: None,
            update_from_version: None,
            default_archive_password: None,
        }
    }

    /// MAJ-D-1 : une config écrite par une version antérieure (sans le nouveau
    /// champ) se lit encore grâce à `#[serde(default)]`.
    #[test]
    fn config_deserializes_without_update_from_version() {
        // JSON écrit par une version qui ne connaît pas le champ.
        let json = r#"{
            "steam_dir": "C:\\Steam",
            "first_run_done": true,
            "update_notified_version": null
        }"#;
        let cfg: AppConfig = serde_json::from_str(json).expect(
            "une config sans update_from_version doit se lire (#[serde(default)])",
        );
        assert_eq!(cfg.steam_dir, Some(PathBuf::from("C:\\Steam")));
        assert_eq!(cfg.update_from_version, None);
    }

    fn exists_true(p: &Path) -> bool {
        let _ = p;
        true
    }

    fn exists_false(p: &Path) -> bool {
        let _ = p;
        false
    }

    /// Prédicat discriminant : ne retourne true que pour le chemin passé en argument.
    fn exists_only(path: &Path) -> impl Fn(&Path) -> bool + '_ {
        move |p: &Path| p == path
    }

    /// Démonstration 1 : steam_dir importé inexistant → la valeur locale survit
    /// et kept_local contient "steam_dir".
    #[test]
    fn merge_imported_keeps_local_steam_dir_when_imported_does_not_exist() {
        let local = local_cfg();
        let imported = AppConfig {
            steam_dir: Some(PathBuf::from("Z:\\NonExistent")),
            ..Default::default()
        };
        let result = merge_imported(&local, &imported, exists_false);
        assert_eq!(
            result.merged.steam_dir,
            Some(PathBuf::from("C:\\Steam")),
            "la valeur locale survit quand le chemin importé n'existe pas"
        );
        assert!(
            result.kept_local.contains(&"steam_dir".to_string()),
            "kept_local doit contenir \"steam_dir\""
        );
    }

    /// Démonstration 2 : steam_dir importé existant → il est adopté
    #[test]
    fn merge_imported_adopts_steam_dir_when_imported_exists() {
        let local = local_cfg();
        let imported = AppConfig {
            steam_dir: Some(PathBuf::from("E:\\SteamNew")),
            ..Default::default()
        };
        let result = merge_imported(&local, &imported, exists_true);
        assert_eq!(
            result.merged.steam_dir,
            Some(PathBuf::from("E:\\SteamNew")),
            "le chemin importé est adopté quand il existe"
        );
        assert!(
            !result.kept_local.contains(&"steam_dir".to_string()),
            "kept_local ne doit pas contenir \"steam_dir\""
        );
    }

    /// Démonstration 3 : library_dir suit exactement la même règle
    #[test]
    fn merge_imported_keeps_local_library_dir_when_imported_does_not_exist() {
        let local = local_cfg();
        let imported = AppConfig {
            library_dir: Some(PathBuf::from("F:\\NonExistentLib")),
            ..Default::default()
        };
        let result = merge_imported(&local, &imported, exists_false);
        assert_eq!(
            result.merged.library_dir,
            Some(PathBuf::from("D:\\SteamLib")),
            "la valeur locale library_dir survit"
        );
        assert!(
            result.kept_local.contains(&"library_dir".to_string()),
            "kept_local doit contenir \"library_dir\""
        );
    }

    /// Démonstration 4 : un champ non-chemin (theme) est importé dans tous les cas
    #[test]
    fn merge_imported_always_imports_non_path_fields() {
        let local = local_cfg();
        let imported = AppConfig {
            steam_dir: Some(PathBuf::from("Z:\\NonExistent")),
            theme: Some("dark".to_string()),
            ..Default::default()
        };
        let result = merge_imported(&local, &imported, exists_false);
        assert_eq!(
            result.merged.theme,
            Some("dark".to_string()),
            "le thème importé est toujours adopté"
        );
        assert_eq!(
            result.merged.steam_dir,
            Some(PathBuf::from("C:\\Steam")),
            "mais le chemin inexistant est conservé localement"
        );
    }

    /// Démonstration 5 : un None importé ne remplace pas un Some local
    #[test]
    fn merge_imported_none_imported_does_not_overwrite_local_some() {
        let local = local_cfg();
        let imported = AppConfig::default(); // tous les chemins à None
        let result = merge_imported(&local, &imported, exists_true);
        assert_eq!(
            result.merged.steam_dir,
            Some(PathBuf::from("C:\\Steam")),
            "un None importé ne remplace pas un Some local"
        );
        assert_eq!(
            result.merged.library_dir,
            Some(PathBuf::from("D:\\SteamLib")),
            "idem pour library_dir"
        );
        // Un None importé n'est pas une décision de l'utilisateur :
        // il n'entre pas dans kept_local.
        assert!(
            result.kept_local.is_empty(),
            "kept_local est vide car l'importé est None (absence, pas conflit)"
        );
    }

    #[test]
    fn test_merge_imported_preserves_default_archive_password() {
        let local = AppConfig {
            default_archive_password: Some("mot-de-passe-local".to_string()),
            ..Default::default()
        };
        let imported = AppConfig::default();

        let result = merge_imported(&local, &imported, exists_true);

        assert_eq!(
            result.merged.default_archive_password,
            Some("mot-de-passe-local".to_string()),
            "l'absence de mot de passe dans l'import ne doit pas effacer celui enregistré localement"
        );
    }

    /// C1 : library_dir importé existant → il est adopté (cas manquant)
    #[test]
    fn merge_imported_adopts_library_dir_when_imported_exists() {
        let local = local_cfg();
        let imported = AppConfig {
            library_dir: Some(PathBuf::from("E:\\SteamNewLib")),
            ..Default::default()
        };
        let result = merge_imported(&local, &imported, exists_true);
        assert_eq!(
            result.merged.library_dir,
            Some(PathBuf::from("E:\\SteamNewLib")),
            "le chemin importé library_dir est adopté quand il existe"
        );
        assert!(
            !result.kept_local.contains(&"library_dir".to_string()),
            "kept_local ne doit pas contenir \"library_dir\""
        );
    }

    /// C2 : le prédicat `exists` est appelé avec le bon chemin.
    /// Si on sonde le chemin local au lieu de l'importé, le résultat change.
    #[test]
    fn merge_imported_exists_predicate_checks_imported_path() {
        let local = local_cfg();
        let imported = AppConfig {
            steam_dir: Some(PathBuf::from("E:\\SteamNew")),
            ..Default::default()
        };
        // Le chemin importé existe, le local n'existe pas.
        let pred = exists_only(Path::new("E:\\SteamNew"));
        let result = merge_imported(&local, &imported, pred);
        assert_eq!(
            result.merged.steam_dir,
            Some(PathBuf::from("E:\\SteamNew")),
            "l'importé est adopté car son chemin existe"
        );
        assert!(
            !result.kept_local.contains(&"steam_dir".to_string()),
            "kept_local ne contient pas steam_dir"
        );
        // Si on sonde le chemin local au lieu de l'importé, le résultat change.
        let pred_local = exists_only(Path::new("C:\\Steam"));
        let result2 = merge_imported(&local, &imported, pred_local);
        assert_eq!(
            result2.merged.steam_dir,
            Some(PathBuf::from("C:\\Steam")),
            "le chemin local survit quand c'est lui qui est sondé"
        );
        assert!(
            result2.kept_local.contains(&"steam_dir".to_string()),
            "kept_local contient steam_dir car le chemin importé n'existe pas"
        );
    }

    /// C3 : (None, Some(existant)) → adopté, kept_local vide.
    #[test]
    fn merge_imported_none_local_some_imported_exists() {
        let local = AppConfig::default();
        let imported = AppConfig {
            steam_dir: Some(PathBuf::from("E:\\SteamNew")),
            library_dir: Some(PathBuf::from("E:\\SteamLibNew")),
            ..Default::default()
        };
        let result = merge_imported(&local, &imported, exists_true);
        assert_eq!(
            result.merged.steam_dir,
            Some(PathBuf::from("E:\\SteamNew")),
            "steam_dir adopté quand il existe"
        );
        assert_eq!(
            result.merged.library_dir,
            Some(PathBuf::from("E:\\SteamLibNew")),
            "library_dir adopté quand il existe"
        );
        assert!(
            result.kept_local.is_empty(),
            "kept_local vide : pas de valeur locale à conserver"
        );
    }

    /// C3 : (None, Some(inexistant)) → None, kept_local vide.
    #[test]
    fn merge_imported_none_local_some_imported_does_not_exist() {
        let local = AppConfig::default();
        let imported = AppConfig {
            steam_dir: Some(PathBuf::from("Z:\\NonExistent")),
            library_dir: Some(PathBuf::from("Z:\\NonExistentLib")),
            ..Default::default()
        };
        let result = merge_imported(&local, &imported, exists_false);
        assert_eq!(
            result.merged.steam_dir,
            None,
            "steam_dir = None car inexistant et pas de local"
        );
        assert_eq!(
            result.merged.library_dir,
            None,
            "library_dir = None car inexistant et pas de local"
        );
        assert!(
            result.kept_local.is_empty(),
            "kept_local vide : aucune valeur locale n'a été préservée"
        );
    }

    /// C4 : les 6 champs "importés tels quels" sont tous vérifiés.
    #[test]
    fn merge_imported_all_non_path_fields_imported_as_is() {
        let local = AppConfig {
            steam_dir: Some(PathBuf::from("C:\\Steam")),
            library_dir: Some(PathBuf::from("D:\\SteamLib")),
            first_run_done: true,
            theme: Some("light".to_string()),
            dark_mode: Some(false),
            locale: None,
            defender_exclusions: Some(true),
            update_notified_version: Some("1.0.0".to_string()),
            update_from_version: None,
            default_archive_password: None,
        };
        let imported = AppConfig {
            steam_dir: Some(PathBuf::from("Z:\\NonExistent")),
            library_dir: Some(PathBuf::from("Z:\\NonExistentLib")),
            first_run_done: false,
            theme: Some("dark".to_string()),
            dark_mode: Some(true),
            locale: None,
            defender_exclusions: Some(false),
            update_notified_version: Some("2.0.0".to_string()),
            update_from_version: None,
            default_archive_password: Some("secret".to_string()),
        };
        let result = merge_imported(&local, &imported, exists_false);
        // Les chemins locaux survivent.
        assert_eq!(
            result.merged.steam_dir,
            Some(PathBuf::from("C:\\Steam")),
            "steam_dir local survit"
        );
        assert_eq!(
            result.merged.library_dir,
            Some(PathBuf::from("D:\\SteamLib")),
            "library_dir local survit"
        );
        // Les champs non-chemins sont importés.
        assert!(
            !result.merged.first_run_done,
            "first_run_done importé (le local true est écrasé)"
        );
        assert_eq!(
            result.merged.theme,
            Some("dark".to_string()),
            "theme importé"
        );
        assert_eq!(
            result.merged.dark_mode,
            Some(true),
            "dark_mode importé"
        );
        assert_eq!(
            result.merged.defender_exclusions,
            Some(false),
            "defender_exclusions importé"
        );
        assert_eq!(
            result.merged.update_notified_version,
            Some("2.0.0".to_string()),
            "update_notified_version importé"
        );
        assert_eq!(
            result.merged.default_archive_password,
            Some("secret".to_string()),
            "default_archive_password importé"
        );
    }
}

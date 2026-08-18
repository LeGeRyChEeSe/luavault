use serde::Serialize;
use std::path::{Path, PathBuf};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::{RegKey, HKEY};

#[derive(Debug, Clone, Serialize, Default)]
pub struct SteamStatus {
    pub path: PathBuf,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SteamToolsStatus {
    pub steam_path: PathBuf,
    /// OpenSteamTool.dll + xinput1_4.dll both present.
    pub installed: bool,
    pub has_open_steam_tool: bool,
    pub has_xinput: bool,
    pub has_dwmapi: bool,
    pub lua_dir_exists: bool,
    /// Legacy config\stplug-in directory still present.
    pub legacy_plugin_dir: bool,
    /// Conflicting SteamProof/MFX artifacts that fix-st.ps1 would remove.
    pub conflicts: Vec<String>,
}

/// The registry — HKCU `SteamPath` in particular — can store the Steam path in
/// lowercase and/or with forward slashes (`e:/applications/steam`). Canonicalize
/// to the real casing with backslashes so every downstream consumer (UI, game
/// dirs, Defender exclusions) sees a clean path.
fn canonicalize_steam_dir(path: &Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(canonical) => {
            let as_string = canonical.display().to_string();
            PathBuf::from(as_string.strip_prefix(r"\\?\").unwrap_or(&as_string))
        }
        Err(_) => path.to_path_buf(),
    }
}

/// Auto-detect the Steam installation: registry first, then common paths.
pub fn detect_steam() -> Option<SteamStatus> {
    let candidates: [(HKEY, &str, &str); 3] = [
        (HKEY_CURRENT_USER, "Software\\Valve\\Steam", "HKCU"),
        (HKEY_LOCAL_MACHINE, "Software\\Valve\\Steam", "HKLM"),
        (
            HKEY_LOCAL_MACHINE,
            "Software\\WOW6432Node\\Valve\\Steam",
            "HKLM\\WOW6432Node",
        ),
    ];
    for (root, subkey, label) in candidates {
        if let Ok(key) = RegKey::predef(root).open_subkey(subkey) {
            for value_name in ["InstallPath", "SteamPath"] {
                if let Ok(raw) = key.get_value::<String, _>(value_name) {
                    let path = PathBuf::from(raw);
                    if path.join("steam.exe").exists() {
                        return Some(SteamStatus {
                            path: canonicalize_steam_dir(&path),
                            source: format!("registre {label} ({value_name})"),
                        });
                    }
                }
            }
        }
    }

    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(base) = std::env::var(var) {
            let path = PathBuf::from(base).join("Steam");
            if path.join("steam.exe").exists() {
                return Some(SteamStatus {
                    path: canonicalize_steam_dir(&path),
                    source: "dossier par défaut".to_string(),
                });
            }
        }
    }
    None
}

pub fn detect_steam_path() -> Option<PathBuf> {
    detect_steam().map(|s| s.path)
}

/// Validate a user-picked Steam folder.
pub fn looks_like_steam_dir(path: &Path) -> bool {
    path.join("steam.exe").exists()
}

pub fn lua_dir(steam: &Path) -> PathBuf {
    steam.join("config").join("lua")
}

pub fn inspect_steamtools(steam: &Path) -> SteamToolsStatus {
    let has = |rel: &str| steam.join(rel).exists();

    let has_open_steam_tool = has("OpenSteamTool.dll");
    let has_xinput = has("xinput1_4.dll");
    let has_dwmapi = has("dwmapi.dll");

    let conflict_files = [
        "wtsapi32.dll",
        "version.dll",
        "config\\manifests.dll",
        "config\\.mfx_init",
        "config\\.stfix_init",
    ];
    let conflicts = conflict_files
        .iter()
        .filter(|f| has(f))
        .map(|f| f.to_string())
        .collect();

    SteamToolsStatus {
        steam_path: steam.to_path_buf(),
        installed: has_open_steam_tool && has_xinput,
        has_open_steam_tool,
        has_xinput,
        has_dwmapi,
        lua_dir_exists: lua_dir(steam).is_dir(),
        legacy_plugin_dir: steam.join("config").join("stplug-in").is_dir(),
        conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_markers_conflicts_and_legacy_dir() {
        let steam = std::env::temp_dir().join(format!("ast_detect_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&steam);
        std::fs::create_dir_all(steam.join("config").join("lua")).unwrap();
        std::fs::create_dir_all(steam.join("config").join("stplug-in")).unwrap();
        std::fs::write(steam.join("OpenSteamTool.dll"), b"x").unwrap();
        std::fs::write(steam.join("xinput1_4.dll"), b"x").unwrap();
        std::fs::write(steam.join("wtsapi32.dll"), b"conflict").unwrap();
        std::fs::write(steam.join("config").join(".mfx_init"), b"conflict").unwrap();

        let status = inspect_steamtools(&steam);
        assert!(status.installed);
        assert!(status.has_open_steam_tool);
        assert!(status.has_xinput);
        assert!(!status.has_dwmapi);
        assert!(status.lua_dir_exists);
        assert!(status.legacy_plugin_dir);
        assert_eq!(
            status.conflicts,
            vec!["wtsapi32.dll".to_string(), "config\\.mfx_init".to_string()]
        );
        assert!(!looks_like_steam_dir(&steam));

        let _ = std::fs::remove_dir_all(&steam);
    }

    #[test]
    fn not_installed_when_markers_missing() {
        let steam = std::env::temp_dir().join(format!("ast_detect_empty_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&steam);
        std::fs::create_dir_all(&steam).unwrap();

        let status = inspect_steamtools(&steam);
        assert!(!status.installed);
        assert!(!status.lua_dir_exists);
        assert!(status.conflicts.is_empty());

        let _ = std::fs::remove_dir_all(&steam);
    }
}

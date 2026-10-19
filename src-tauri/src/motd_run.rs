//! Running a message's action — the one place, whether the click came from the
//! window or from a notification with the app closed.
//!
//! The set is closed (`motd::ResolvedAction`) and the caller has already looked the
//! action up in a message whose signature it verified. What is added here is a
//! guard both paths share: **nothing that closes Steam runs while a game is
//! running.** A confirmation in the window protects against a mis-click; a toast
//! button pressed while the user is mid-game would protect against nothing, and
//! "Steam closed during a boss fight" is the failure nobody forgives.

use std::path::PathBuf;

use crate::motd::ResolvedAction;
use crate::update::UpdateClient;
use crate::{install, steam_downgrade};

/// Does this action close, relaunch or reinstall Steam?
pub fn closes_steam(action: &ResolvedAction) -> bool {
    // Changing the manifest source is read live by SteamTools: Steam stays open.
    !matches!(action, ResolvedAction::OpenUrl(_) | ResolvedAction::SteamToolsSource(_))
}

/// Steam keeps the AppID of the game it is running in `RunningAppID`; `0` or
/// absent means none.
pub fn game_running(running_app_id: Option<u32>) -> bool {
    running_app_id.is_some_and(|id| id != 0)
}

#[cfg(windows)]
pub fn running_app_id() -> Option<u32> {
    use winreg::enums::HKEY_CURRENT_USER;
    winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .ok()?
        .get_value::<u32, _>("RunningAppID")
        .ok()
}

#[cfg(not(windows))]
pub fn running_app_id() -> Option<u32> {
    None
}

/// The refusal, in the same French as every command's error.
pub const GAME_RUNNING_MESSAGE: &str =
    "Un jeu est en cours dans Steam : fermez-le d'abord, puis recommencez. Rien n'a été modifié.";

/// Decide before doing: `Some(reason)` when the action must not run now.
pub fn refusal(action: &ResolvedAction, running_app_id: Option<u32>) -> Option<&'static str> {
    (closes_steam(action) && game_running(running_app_id)).then_some(GAME_RUNNING_MESSAGE)
}

/// Run `action`. The returned sentence is what the window or the toast says.
pub async fn execute(
    http: &UpdateClient,
    steam_dir: Option<PathBuf>,
    action: ResolvedAction,
) -> Result<String, String> {
    if let Some(reason) = refusal(&action, running_app_id()) {
        return Err(reason.to_string());
    }
    let steam = || steam_dir.clone().ok_or("Steam introuvable : indiquez son dossier dans Réglages".to_string());
    match action {
        ResolvedAction::SteamDowngrade(pin) => {
            let version = steam_downgrade::apply(http, &steam()?, &pin).await.map_err(|e| e.to_string())?;
            Ok(format!(
                "Steam est revenu à la version {version} et ne se mettra plus à jour tout seul. Il redémarre : patientez quelques secondes."
            ))
        }
        ResolvedAction::SteamUndo => {
            steam_downgrade::undo(&steam()?).await.map_err(|e| e.to_string())?;
            Ok("Les mises à jour de Steam sont rétablies. Steam redémarre et se mettra à jour.".to_string())
        }
        ResolvedAction::RestartSteam => {
            let exe = steam()?.join("steam.exe");
            if !exe.is_file() {
                return Err("steam.exe introuvable dans le dossier Steam".to_string());
            }
            tauri::async_runtime::spawn_blocking(move || install::restart_steam(&exe))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            Ok("Steam redémarre : patientez quelques secondes.".to_string())
        }
        ResolvedAction::RepairTools => {
            let (program, args) = install::steamtools_command();
            install::run_elevated(program, &args).map_err(|e| e.to_string())?;
            Ok("Script SteamTools lancé dans une fenêtre PowerShell élevée (Steam sera fermé par le script).".to_string())
        }
        ResolvedAction::SteamToolsSource(source) => {
            let dir = steam()?;
            if !crate::detect::inspect_steamtools(&dir).installed {
                return Err("SteamTools est introuvable dans le dossier de Steam : rien n'a été modifié.".to_string());
            }
            let name = source.clone();
            tauri::async_runtime::spawn_blocking(move || crate::steamtools_source::apply(&dir, &name))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "La source des manifestes de SteamTools est maintenant « {source} ». Steam n'a pas besoin d'être redémarré : relancez l'installation du jeu."
            ))
        }
        ResolvedAction::OpenUrl(url) => {
            tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| e.to_string())?;
            Ok(String::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam_downgrade::Pin;

    fn all_actions() -> Vec<ResolvedAction> {
        vec![
            ResolvedAction::SteamDowngrade(Pin { snapshot: "20261006145653".into(), sha256: "0".repeat(64) }),
            ResolvedAction::SteamUndo,
            ResolvedAction::RestartSteam,
            ResolvedAction::RepairTools,
            ResolvedAction::OpenUrl("https://example.org/".into()),
            ResolvedAction::SteamToolsSource("wudrm".into()),
        ]
    }

    #[test]
    fn only_opening_a_page_or_editing_a_config_leaves_steam_alone() {
        let closers: Vec<bool> = all_actions().iter().map(closes_steam).collect();
        assert_eq!(closers, [true, true, true, true, false, false]);
    }

    #[test]
    fn a_running_game_blocks_everything_that_closes_steam() {
        for a in all_actions() {
            let blocked = refusal(&a, Some(1_245_620)).is_some();
            assert_eq!(blocked, closes_steam(&a), "{a:?}");
        }
    }

    #[test]
    fn no_game_means_no_refusal() {
        for a in all_actions() {
            assert!(refusal(&a, None).is_none(), "absent value: {a:?}");
            assert!(refusal(&a, Some(0)).is_none(), "RunningAppID 0 is no game: {a:?}");
        }
    }

    #[test]
    fn the_refusal_says_nothing_was_changed() {
        assert!(GAME_RUNNING_MESSAGE.contains("Rien n'a été modifié"));
    }

    #[tokio::test]
    async fn without_a_steam_folder_the_steam_actions_say_so_and_do_nothing() {
        let http = UpdateClient::new();
        for a in [ResolvedAction::SteamUndo, ResolvedAction::RestartSteam] {
            let err = execute(&http, None, a).await.unwrap_err();
            // Either "no Steam folder" or, on a machine with a game running, the guard.
            assert!(err.contains("Steam introuvable") || err == GAME_RUNNING_MESSAGE, "{err}");
        }
    }
}

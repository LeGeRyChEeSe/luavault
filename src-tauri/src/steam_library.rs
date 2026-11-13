//! LuaVault on the play bar of the Steam client's library (experimental edition).
//!
//! The main window's script (`scripts/steam-client-pill.js`) reads which game the library shows
//! (the `overview.appid` the client hands its play button) and asks `lib:<appid>`. The answer is a
//! [`LibCard`]: what LuaVault knows of that game, in one line, and the gestures it allows, decided
//! here from the very `GameStatus` the window's cards use (`commands::build_status`, `derive_stage`).
//! A game that is not in LuaVault's library gets no card, so nothing is drawn.
//!
//! A gesture comes back as `act:<action>:<appid>` under a closed set ([`LibAction`]); the page never
//! names a file, a path or a kind. What runs is what the window's buttons run: the same install of a
//! patch already in the library (`commands::install_fix_inner`), the same uninstall
//! (`fixes::uninstall`), the same removal from Steam (`library::remove_from_steam`). Nothing is ever
//! downloaded from here: « Trouver un correctif sur le site » opens the site's home page
//! (`steam_store::find`), the person downloads there, and the downloads follow-up takes the file.

use std::collections::BTreeMap;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::GameStatus;
use crate::fixes::{self, FixHealth};
use crate::{library, motd_run, steam_client, steam_store};

/// What the play bar may ask, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibAction {
    /// Install the patch archive the library already holds.
    Apply,
    /// Open the site's home page to look for a patch (nothing is downloaded by LuaVault).
    Find,
    /// Install again a patch that is damaged or was left behind by a moved game.
    Repair,
    /// Take the patch off, originals back.
    Unpatch,
    /// Take the `.lua` out of Steam (the library keeps it).
    RemoveLua,
    /// Steam's own uninstall dialog.
    Uninstall,
    /// Show this game in LuaVault.
    Open,
}

impl LibAction {
    pub const ALL: [LibAction; 7] =
        [LibAction::Apply, LibAction::Find, LibAction::Repair, LibAction::Unpatch, LibAction::RemoveLua, LibAction::Uninstall, LibAction::Open];

    pub fn parse(word: &str) -> Option<LibAction> {
        LibAction::ALL.into_iter().find(|a| a.id() == word)
    }

    pub fn id(self) -> &'static str {
        match self {
            LibAction::Apply => "apply",
            LibAction::Find => "find",
            LibAction::Repair => "repair",
            LibAction::Unpatch => "unpatch",
            LibAction::RemoveLua => "remove_lua",
            LibAction::Uninstall => "uninstall",
            LibAction::Open => "open",
        }
    }
}

/// One entry of the play bar's menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibButton {
    pub id: &'static str,
    pub label: String,
    /// The second click's label, for a gesture that removes something.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
}

/// What the play bar shows for one game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibCard {
    pub app_id: String,
    /// `ok`, `warn`, `error`, `muted` or `busy` (the script's closed set).
    pub tone: &'static str,
    pub line: String,
    pub actions: Vec<LibButton>,
}

fn broken(health: FixHealth) -> bool {
    matches!(health, FixHealth::Damaged | FixHealth::GameMoved)
}

/// Pure: the card of a game LuaVault holds, `None` for any other.
pub fn card_for(status: &GameStatus, lang: &str) -> Option<LibCard> {
    if !status.in_library {
        return None;
    }
    let fr = lang == "fr";
    let t = |a: &str, b: &str| if fr { a.to_string() } else { b.to_string() };
    let (tone, line) = match status.stage {
        "lua_not_in_steam" => ("muted", t("Retiré de Steam", "Taken out of Steam")),
        "needs_steam_install" => ("muted", t("Dans LuaVault, pas encore installé", "In LuaVault, not installed yet")),
        "installing" => ("busy", t("Installation par Steam en cours", "Steam is installing it")),
        "fix_downloaded" => ("warn", t("Correctif prêt à appliquer", "Patch ready to apply")),
        "fix_damaged" => ("error", t("Correctif abîmé", "Patch damaged")),
        "fix_game_moved" => ("error", t("Jeu déplacé : correctif à reposer", "Game moved: patch to apply again")),
        "fix_installed" => ("ok", t("Correctif actif", "Patch active")),
        "fix_external" => ("muted", t("Patché hors de LuaVault", "Patched outside LuaVault")),
        _ => ("ok", t("Prêt", "Ready")),
    };
    let ready = status.game.installed && status.game.fully_installed;
    let mut actions = Vec::new();
    let mut push = |action: LibAction, label: String, confirm: Option<String>| actions.push(LibButton { id: action.id(), label, confirm });
    if ready {
        let has_fix = status.fix.health != FixHealth::NotInstalled;
        if broken(status.fix.health) && status.fix_downloaded {
            push(LibAction::Repair, t("Réparer le correctif", "Repair the patch"), None);
        }
        if !has_fix && status.fix.foreign.is_empty() {
            if status.fix_downloaded {
                push(LibAction::Apply, t("Appliquer le correctif", "Apply the patch"), None);
            } else {
                push(LibAction::Find, t("Trouver un correctif sur le site", "Find a patch on the site"), None);
            }
        }
        if has_fix {
            push(
                LibAction::Unpatch,
                t("Retirer le correctif", "Remove the patch"),
                Some(t("Confirmer : remettre les fichiers d'origine", "Confirm: restore the original files")),
            );
        }
    }
    push(LibAction::Open, t("Ouvrir dans LuaVault", "Open in LuaVault"), None);
    if status.lua_in_steam {
        push(
            LibAction::RemoveLua,
            t("Retirer le jeu de Steam (.lua)", "Take the game out of Steam (.lua)"),
            Some(t("Confirmer : Steam ne verra plus ce jeu", "Confirm: Steam will no longer see this game")),
        );
    }
    if status.game.installed {
        push(LibAction::Uninstall, t("Désinstaller le jeu", "Uninstall the game"), None);
    }
    Some(LibCard { app_id: status.app_id.clone(), tone, line, actions })
}

/// The card of the game Steam shows, read now (blocking: it verifies the patch's files).
pub fn card_of(cfg: &crate::config::AppConfig, app_id: &str, lang: &str) -> Option<LibCard> {
    let lib = cfg.resolved_library_dir();
    let entries = library::load_index(&lib);
    let entry = entries.iter().find(|e| e.app_id == app_id)?;
    let steam = crate::commands::resolve_steam(cfg);
    let status = crate::commands::build_status(&lib, steam.as_deref(), app_id, Some(entry));
    card_for(&status, lang)
}

/// One dot of the library's game list: the colour and the line of the game's card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dot {
    pub tone: &'static str,
    pub line: String,
}

/// Every game of LuaVault's library with its dot, from the same `card_for` as the play bar's block.
/// Blocking file reads: called on a blocking thread.
pub fn dots_of(cfg: &crate::config::AppConfig, lang: &str) -> BTreeMap<String, Dot> {
    let mut out = BTreeMap::new();
    if cfg.steam_library == Some(false) {
        return out;
    }
    let lib = cfg.resolved_library_dir();
    let steam = crate::commands::resolve_steam(cfg);
    for entry in library::load_index(&lib) {
        let status = crate::commands::build_status(&lib, steam.as_deref(), &entry.app_id, Some(&entry));
        if let Some(card) = card_for(&status, lang) {
            out.insert(entry.app_id.clone(), Dot { tone: card.tone, line: card.line });
        }
    }
    out
}

/// What the page's `lvLibDots` receives: JSON from serde, every text stays a string.
pub fn dots_call(dots: &BTreeMap<String, Dot>) -> String {
    let json = serde_json::to_string(dots).unwrap_or_else(|_| "{}".to_string());
    format!("window.lvLibDots && window.lvLibDots({json})")
}

fn lang(app: &AppHandle) -> &'static str {
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    steam_store::lang_of(&cfg)
}

/// Read the dots on a blocking thread and send them to the main window.
pub async fn send_dots(app: &AppHandle) {
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    let lang = lang(app);
    let dots = tauri::async_runtime::spawn_blocking(move || dots_of(&cfg, lang)).await.unwrap_or_default();
    steam_client::to_window(dots_call(&dots));
}

/// What the page's `lvLibCard` receives: the game asked about, and its card or nothing.
pub fn card_call(app_id: &str, card: Option<&LibCard>) -> String {
    let payload = serde_json::json!({ "appId": app_id, "card": card });
    format!("window.lvLibCard && window.lvLibCard({payload})")
}

/// Read the card on a blocking thread and send it to the main window.
pub async fn send_card(app: &AppHandle, app_id: &str, note: Option<(&'static str, String)>) {
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    let lang = lang(app);
    let id = app_id.to_string();
    // Switched off in the panel inside Steam: every game reads as « not ours », nothing is drawn.
    let card = if cfg.steam_library != Some(false) {
        tauri::async_runtime::spawn_blocking(move || card_of(&cfg, &id, lang)).await.ok().flatten()
    } else {
        None
    };
    let card = match (card, note) {
        (Some(mut card), Some((tone, line))) => {
            card.tone = tone;
            card.line = line;
            Some(card)
        }
        (card, _) => card,
    };
    steam_client::to_window(card_call(app_id, card.as_ref()));
    // The game's state may have moved: its dot in the list follows.
    send_dots(app).await;
}

fn busy_card(app_id: &str, line: String) -> LibCard {
    LibCard { app_id: app_id.to_string(), tone: "busy", line, actions: Vec::new() }
}

/// Told to the window: show this game in the library.
pub const OPEN_GAME: &str = "steam://open-game";

/// Run one gesture of the play bar, then send the game's card again. The line says how it went.
pub async fn act(app: &AppHandle, action: LibAction, app_id: &str) {
    let fr = lang(app) == "fr";
    let t = |a: &str, b: &str| if fr { a.to_string() } else { b.to_string() };
    match action {
        LibAction::Open => {
            let name = library::load_index(&app.state::<crate::AppState>().config.lock().unwrap().resolved_library_dir())
                .into_iter()
                .find(|e| e.app_id == app_id)
                .map(|e| e.name)
                .unwrap_or_default();
            let _ = app.emit(OPEN_GAME, serde_json::json!({ "app_id": app_id, "name": name }));
            crate::guard_service::show_main(app);
            return;
        }
        LibAction::Uninstall => {
            let _ = crate::install::open_steam_uri(&format!("steam://uninstall/{app_id}"));
            return;
        }
        LibAction::Find => {
            steam_store::find(app, app_id, steam_store::Wanted::PatchOnly).await;
            let line = steam_client::job_of(app_id).map(|j| j.line).unwrap_or_default();
            let notice = steam_client::Notice {
                title: t("Trouver un correctif sur le site", "Find a patch on the site"),
                line: line.clone(),
                tone: "info",
                app_id: Some(app_id.to_string()),
                open: false,
            };
            steam_client::notify_in_steam(&notice);
            send_card(app, app_id, Some(("busy", t("Site ouvert : téléchargez le correctif, LuaVault le reprendra.", "Site opened: download the patch, LuaVault will pick it up.")))).await;
            return;
        }
        _ => {}
    }
    if motd_run::game_running(motd_run::running_app_id()) {
        let line = t("Un jeu est en cours : fermez-le d'abord.", "A game is running: close it first.");
        send_card(app, app_id, Some(("error", line))).await;
        return;
    }
    let busy = match action {
        LibAction::Apply | LibAction::Repair => t("Pose en cours...", "Applying..."),
        LibAction::Unpatch => t("Retrait du correctif...", "Removing the patch..."),
        _ => t("En cours...", "Working..."),
    };
    steam_client::to_window(card_call(app_id, Some(&busy_card(app_id, busy))));
    let note = match run(app, action, app_id, fr).await {
        Ok(Some(line)) => Some(("ok", line)),
        Ok(None) => None,
        Err(e) => {
            log::warn!("LuaVault dans Steam : {} pour {app_id} : {e}", action.id());
            Some(("error", crate::notify::plain_text(&e, 140)))
        }
    };
    send_card(app, app_id, note).await;
    let _ = app.emit(crate::steam_followup::LIBRARY_CHANGED, ());
    steam_client::refresh();
}

async fn run(app: &AppHandle, action: LibAction, app_id: &str, fr: bool) -> Result<Option<String>, String> {
    let cfg = app.state::<crate::AppState>().config.lock().unwrap().clone();
    let lib = cfg.resolved_library_dir();
    let say = |a: &str, b: &str| if fr { a.to_string() } else { b.to_string() };
    match action {
        LibAction::Apply | LibAction::Repair => {
            if !fixes::archive_path(&lib, app_id).is_file() {
                return Err(say("Aucun correctif dans LuaVault pour ce jeu.", "No patch in LuaVault for this game."));
            }
            let report = crate::commands::install_fix_inner(&cfg, app_id, None).await?;
            Ok(Some(say(&format!("Correctif appliqué ({} fichier(s)).", report.file_count), &format!("Patch applied ({} file(s)).", report.file_count))))
        }
        LibAction::Unpatch => {
            let id = app_id.to_string();
            tauri::async_runtime::spawn_blocking(move || fixes::uninstall(&lib, &id).map_err(|e| format!("{e:#}")))
                .await
                .map_err(|e| e.to_string())??;
            Ok(Some(say("Correctif retiré, fichiers d'origine remis.", "Patch removed, original files restored.")))
        }
        LibAction::RemoveLua => {
            let steam = crate::commands::resolve_steam(&cfg).ok_or_else(|| say("Steam introuvable", "Steam not found"))?;
            library::remove_from_steam(app_id, &steam).map_err(|e| format!("{e:#}"))?;
            Ok(Some(say("Retiré de Steam : la bibliothèque de LuaVault le garde.", "Taken out of Steam: LuaVault's library keeps it.")))
        }
        LibAction::Open | LibAction::Uninstall | LibAction::Find => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vdf::GameInstall;

    fn status(stage: &'static str) -> GameStatus {
        GameStatus {
            app_id: "413150".into(),
            name: "Stardew Valley".into(),
            icon: None,
            updated_at: None,
            added_at: None,
            in_library: true,
            lua_in_steam: true,
            fix_downloaded: false,
            hidden: false,
            tags: Vec::new(),
            game: GameInstall { installed: true, fully_installed: true, ..Default::default() },
            playtime_minutes: None,
            last_played: None,
            fix: fixes::FixReport::empty("413150"),
            stage,
        }
    }

    fn ids(card: &LibCard) -> Vec<&'static str> {
        card.actions.iter().map(|a| a.id).collect()
    }

    #[test]
    fn steam_library_a_game_luavault_does_not_hold_gets_nothing() {
        let mut s = status("no_lua");
        s.in_library = false;
        assert_eq!(card_for(&s, "fr"), None);
    }

    #[test]
    fn steam_library_each_state_offers_the_gestures_it_allows() {
        // No patch in the library: the site, never a download.
        let card = card_for(&status("ready"), "fr").unwrap();
        assert_eq!(ids(&card), vec!["find", "open", "remove_lua", "uninstall"]);

        let mut s = status("fix_downloaded");
        s.fix_downloaded = true;
        let card = card_for(&s, "fr").unwrap();
        assert_eq!((card.tone, ids(&card)[0]), ("warn", "apply"));

        let mut s = status("fix_installed");
        s.fix_downloaded = true;
        s.fix.health = FixHealth::Healthy;
        let card = card_for(&s, "fr").unwrap();
        assert_eq!((card.tone, card.line.as_str()), ("ok", "Correctif actif"));
        assert_eq!(ids(&card), vec!["unpatch", "open", "remove_lua", "uninstall"], "no second install of what is in place");
        assert!(card.actions[0].confirm.is_some(), "removing asks twice");

        let mut s = status("fix_damaged");
        s.fix_downloaded = true;
        s.fix.health = FixHealth::Damaged;
        assert_eq!(ids(&card_for(&s, "fr").unwrap())[..2], ["repair", "unpatch"]);

        let mut s = status("fix_external");
        s.fix.foreign = vec!["OnlineFix.ini".into()];
        let ext = ids(&card_for(&s, "fr").unwrap());
        assert!(!ext.contains(&"find") && !ext.contains(&"apply"), "never over someone else's patch");

        // Not installed: nothing to patch, nothing to uninstall.
        let mut s = status("needs_steam_install");
        s.fix_downloaded = true;
        s.game = GameInstall::default();
        assert_eq!(ids(&card_for(&s, "fr").unwrap()), vec!["open", "remove_lua"]);
    }

    #[test]
    fn steam_library_the_page_gets_json_it_cannot_be_broken_by() {
        let card = card_for(&status("ready"), "fr").unwrap();
        let call = card_call("413150", Some(&card));
        assert!(call.starts_with("window.lvLibCard && window.lvLibCard({"), "{call}");
        let json = call.trim_start_matches("window.lvLibCard && window.lvLibCard(").trim_end_matches(')');
        let back: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(back["appId"], "413150");
        assert_eq!(back["card"]["tone"], "ok");
        assert_eq!(card_call("1", None), "window.lvLibCard && window.lvLibCard({\"appId\":\"1\",\"card\":null})");
        let mut dots = BTreeMap::new();
        dots.insert("413150".to_string(), Dot { tone: "error", line: "abîmé \"</script>".to_string() });
        let call = dots_call(&dots);
        assert!(call.starts_with("window.lvLibDots && window.lvLibDots({\"413150\":{\"tone\":\"error\""), "{call}");
        assert!(call.contains("\\\"</script>"), "a quote stays inside its string: {call}");
    }

    #[test]
    fn steam_library_actions_are_a_closed_set() {
        for a in LibAction::ALL {
            assert_eq!(LibAction::parse(a.id()), Some(a));
        }
        for w in ["", "Apply", "apply ", "patch", "bypass", "delete", "remove_lua;"] {
            assert_eq!(LibAction::parse(w), None, "{w}");
        }
    }

    #[test]
    fn steam_library_texts_carry_no_dash() {
        for stage in ["lua_not_in_steam", "needs_steam_install", "installing", "fix_downloaded", "fix_damaged", "fix_game_moved", "fix_installed", "fix_external", "ready"] {
            for lang in ["fr", "en"] {
                let mut s = status(stage);
                s.fix_downloaded = true;
                s.fix.health = FixHealth::Damaged;
                let card = card_for(&s, lang).unwrap();
                for text in std::iter::once(card.line.clone()).chain(card.actions.iter().flat_map(|a| [Some(a.label.clone()), a.confirm.clone()]).flatten()) {
                    assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'), "{text}");
                }
            }
        }
    }
}

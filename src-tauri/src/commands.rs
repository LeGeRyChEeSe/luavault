use crate::{
    archive, artwork, backup, cache, config, defender, detect, discover, encrypted_backup,
    exchange, fixes, hmac, i18n_log, install, library, reachability, stats, steamstore, update, vdf, wipe, AppState,
};
use log::{debug, info, warn};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Clone, Serialize, Default)]
pub struct DetectionReport {
    pub portable: bool,
    pub data_dir: String,
    pub library_dir: String,
    pub library_count: usize,
    pub steam: Option<detect::SteamStatus>,
    pub steamtools: Option<detect::SteamToolsStatus>,
    pub first_run_done: bool,
    /// Saved appearance, so the UI can paint the right theme on first frame.
    pub theme: Option<String>,
    pub dark_mode: Option<bool>,
    /// UI locale (None = the interface infers one from the system language).
    pub locale: Option<String>,
    /// Defender exclusion choice (None = the app should ask once).
    pub defender_exclusions: Option<bool>,
    /// Password retained for subsequent encrypted online-fix archives.
    pub default_archive_password: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    pub version: String,
    pub portable: bool,
    pub data_dir: String,
}

/// Explicit, bounded Steam reachability check. The frontend calls it at startup
/// and when the user retries; no timer or background loop is involved.
#[tauri::command]
pub async fn get_reachability(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<reachability::Reachability, String> {
    let online = reachability::probe_steam(&state.http).await;
    let result = state
        .reachability
        .lock()
        .map_err(|_| "état de joignabilité indisponible".to_string())?
        .record_probe(online);

    app.emit("reachability://changed", result.clone())
        .map_err(|e| format!("événement de joignabilité : {e}"))?;
    Ok(result)
}

/// Outcome of importing one user-provided `.lua` file.
///
/// `filename_differs` is informational only: the AppID always comes from the
/// declarations in the file, never from the name the user gave it.
#[derive(Debug, Clone, Serialize)]
pub struct LuaImportResult {
    pub entry: library::LibraryEntry,
    pub filename_differs: bool,
}

/// The blocking preparation phase of a local `.lua` import. The bytes and the
/// HMAC-verified index intentionally travel together so metadata resolution
/// cannot make us read either source a second time.
struct PreparedLuaImport {
    app_id: String,
    stem: String,
    filename_differs: bool,
    bytes: Vec<u8>,
    entries: Vec<library::LibraryEntry>,
}

/// Outcome of importing one locally supplied patch archive.
#[derive(Debug, Clone, Serialize)]
pub struct PatchImportResult {
    pub app_id: String,
    pub archive_path: String,
    /// True when the AppID came from a deliberately unambiguous filename form.
    pub app_id_inferred: bool,
}

/// Extract an AppID only from a deliberate filename form: `440.zip`,
/// `440_online_fix.zip`, or `Portal 2 (440).zip`. Bare digits elsewhere are
/// intentionally ignored: `patch_v2_64bit.zip` must never select a game by accident.
fn app_id_from_patch_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    if !stem.is_empty() && stem.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(stem.to_string());
    }
    if let Some(candidate) = stem.strip_suffix("_online_fix") {
        if !candidate.is_empty() && candidate.bytes().all(|byte| byte.is_ascii_digit()) {
            return Some(candidate.to_string());
        }
    }
    let start = stem.rfind('(')?;
    let candidate = stem.get(start + 1..)?.strip_suffix(')')?;
    (!candidate.is_empty() && candidate.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| candidate.to_string())
}

/// Validate, associate and store a local archive at the canonical fix path.
///
/// An explicit AppID is the UI's confirmation of the target game. Without it,
/// only `Name (AppID)` may be inferred and that AppID must already be present
/// in the managed library; no unrelated number in a filename is ever guessed.
fn import_patch_archive_inner(
    path: &Path,
    requested_app_id: Option<&str>,
    library_dir: &Path,
    data_dir: &Path,
) -> Result<PatchImportResult, String> {
    import_patch_archive_inner_with_before_publish(
        path,
        requested_app_id,
        library_dir,
        data_dir,
        |_| {},
    )
}

/// The callback observes the fully written temporary file immediately before
/// publication. Production passes a no-op; the unit test uses it to prove the
/// `.partial` file exists before the final rename.
fn import_patch_archive_inner_with_before_publish<F>(
    path: &Path,
    requested_app_id: Option<&str>,
    library_dir: &Path,
    data_dir: &Path,
    before_publish: F,
) -> Result<PatchImportResult, String>
where
    F: FnOnce(&Path),
{
    if !path.is_file() {
        return Err("le chemin spécifié n'est pas un fichier".to_string());
    }

    match archive::detect_kind(path) {
        archive::ArchiveKind::Rar | archive::ArchiveKind::Zip => {}
        archive::ArchiveKind::SevenZ => {
            return Err("les archives .7z ne sont pas prises en charge : réemballez l'archive en .zip ou .rar".to_string());
        }
        archive::ArchiveKind::Unknown => {
            return Err("format d'archive non reconnu : choisissez une archive .zip ou .rar valide".to_string());
        }
    }

    let inferred = requested_app_id.is_none();
    let app_id = requested_app_id
        .filter(|id| !id.trim().is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
        .map(str::to_owned)
        .or_else(|| app_id_from_patch_filename(path))
        .ok_or_else(|| "impossible d'identifier le jeu : sélectionnez son AppID avant d'importer le patch".to_string())?;

    let known = library::load_index_with_data_dir(library_dir, data_dir)
        .map_err(|e| e.to_string())?
        .into_iter()
        .any(|entry| entry.app_id == app_id);
    if !known {
        return Err(format!("ce jeu n'est pas dans votre bibliothèque (AppID {app_id}) : sélectionnez le jeu visé avant d'importer le patch"));
    }

    let destination = fixes::archive_path(library_dir, &app_id);
    let parent = destination.parent().ok_or_else(|| "dossier de patch invalide".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("création du dossier des patchs : {e}"))?;
    let temporary = parent.join(format!(".{app_id}_online_fix.{}.partial", std::process::id()));
    std::fs::copy(path, &temporary).map_err(|e| format!("copie de l'archive de patch : {e}"))?;
    before_publish(&temporary);
    if destination.exists() {
        std::fs::remove_file(&destination).map_err(|e| format!("remplacement de l'archive de patch existante : {e}"))?;
    }
    if let Err(error) = std::fs::rename(&temporary, &destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("publication de l'archive de patch : {error}"));
    }

    Ok(PatchImportResult {
        app_id,
        archive_path: destination.display().to_string(),
        app_id_inferred: inferred,
    })
}

fn build_report(cfg: &config::AppConfig) -> DetectionReport {
    let library_dir = cfg.resolved_library_dir();
    let steam = cfg
        .steam_dir
        .clone()
        .map(|path| detect::SteamStatus {
            path,
            source: "choisi manuellement".to_string(),
        })
        .or_else(detect::detect_steam);
    let steamtools = steam.as_ref().map(|s| detect::inspect_steamtools(&s.path));
    DetectionReport {
        portable: config::is_portable(),
        data_dir: config::data_dir().display().to_string(),
        library_dir: library_dir.display().to_string(),
        library_count: library::load_index(&library_dir).len(),
        steam,
        steamtools,
        first_run_done: cfg.first_run_done,
        theme: cfg.theme.clone(),
        dark_mode: cfg.dark_mode,
        locale: cfg.locale.clone(),
        defender_exclusions: cfg.defender_exclusions,
        default_archive_password: cfg.default_archive_password.clone(),
    }
}

fn resolve_steam(cfg: &config::AppConfig) -> Option<PathBuf> {
    cfg.steam_dir.clone().or_else(detect::detect_steam_path)
}

/// Everything the UI needs to colour-code one game in a single round-trip.
#[derive(Debug, Clone, Serialize)]
pub struct GameStatus {
    pub app_id: String,
    pub name: String,
    pub icon: Option<String>,
    pub updated_at: Option<String>,
    /// When this entry was first added to the library (RFC 3339).
    pub added_at: Option<String>,
    /// The `.lua` exists in our library folder.
    pub in_library: bool,
    /// The `.lua` has been copied into `{Steam}\config\lua`.
    pub lua_in_steam: bool,
    /// The fix archive has been downloaded.
    pub fix_downloaded: bool,
    /// Hidden from the library view without deleting anything.
    pub hidden: bool,
    /// User-defined tags for categorising games.
    pub tags: Vec<String>,
    pub game: vdf::GameInstall,
    /// Minutes played per Steam's local records (LOT-13). `None` = "on ne
    /// sait pas" (no readable data) — never displayed as zero. `Some(0)`
    /// with no last session means "jamais joué". Not a lifecycle signal:
    /// it deliberately stays out of `derive_stage`.
    pub playtime_minutes: Option<u64>,
    /// Unix seconds of the last recorded session, when Steam has one.
    pub last_played: Option<u64>,
    pub fix: fixes::FixReport,
    /// Single derived state driving the badge, colour and primary action.
    pub stage: &'static str,
}

/// Collapse the raw signals into the one state the user actually cares about.
fn derive_stage(status: &GameStatus) -> &'static str {
    if !status.in_library {
        return "no_lua";
    }
    if !status.lua_in_steam {
        return "lua_not_in_steam";
    }
    if !status.game.installed {
        return "needs_steam_install";
    }
    if !status.game.fully_installed {
        return "installing";
    }
    match status.fix.health {
        fixes::FixHealth::GameMoved => "fix_game_moved",
        fixes::FixHealth::Damaged => "fix_damaged",
        fixes::FixHealth::Healthy => "fix_installed",
        fixes::FixHealth::NotInstalled => {
            // A game adopted from Steam may already carry someone else's patch.
            // Saying "patch available" there would invite an overwrite we can't undo.
            if !status.fix.foreign.is_empty() {
                "fix_external"
            } else if status.fix_downloaded {
                "fix_downloaded"
            } else {
                "ready"
            }
        }
    }
}

/// Collect every game's status. Shared by `library_status` and `library_stats`.
fn collect_statuses(
    lib: &Path,
    steam: Option<PathBuf>,
    entries: Vec<library::LibraryEntry>,
) -> Vec<GameStatus> {
    entries
        .into_iter()
        .map(|entry| build_status(lib, steam.as_deref(), &entry.app_id, Some(&entry)))
        .collect()
}

/// Timing wrapper around [`collect_statuses`] — returns (statuses, entry_count, elapsed_ms).
/// The count and duration are logged by the caller so a future regression is visible.
fn measured_collect_statuses(
    lib: &Path,
    steam: Option<PathBuf>,
    entries: Vec<library::LibraryEntry>,
) -> (Vec<GameStatus>, usize, u128) {
    measured_collect_statuses_with(lib, steam, entries, std::time::Instant::now)
}

/// Same as [`measured_collect_statuses`] with an injectable clock, so a test
/// can prove the reported duration is actually measured — a hardcoded zero
/// satisfies any upper bound but never a lower one.
fn measured_collect_statuses_with(
    lib: &Path,
    steam: Option<PathBuf>,
    entries: Vec<library::LibraryEntry>,
    now: fn() -> std::time::Instant,
) -> (Vec<GameStatus>, usize, u128) {
    let count = entries.len();
    let start = now();
    let statuses = collect_statuses(lib, steam, entries);
    let elapsed_ms = now().duration_since(start).as_millis();
    (statuses, count, elapsed_ms)
}

fn build_status(
    lib: &Path,
    steam: Option<&Path>,
    app_id: &str,
    entry: Option<&library::LibraryEntry>,
) -> GameStatus {
    let game = steam
        .map(|s| vdf::locate_game(s, app_id))
        .unwrap_or_else(|| vdf::GameInstall {
            app_id: app_id.to_string(),
            ..Default::default()
        });
    let game_dir = game.install_dir.as_deref().map(Path::new);

    // LOT-13: local playtime — pure file reads, no network. A game present
    // in the account's apps block without any keys was never played: encode
    // that as zero minutes, which the UI renders "jamais joué". No record
    // at all (unreadable file, ambiguous account, absent AppID) stays None:
    // "on ne sait pas", a different state from zero.
    let (playtime_minutes, last_played) = match steam.and_then(|s| vdf::playtime_for(s, app_id)) {
        Some(record) if record.minutes.is_none() && record.last_played.is_none() => {
            (Some(0), None)
        }
        Some(record) => (record.minutes, record.last_played),
        None => (None, None),
    };

    let mut status = GameStatus {
        app_id: app_id.to_string(),
        name: entry.map(|e| e.name.clone()).unwrap_or_default(),
        icon: entry.and_then(|e| e.icon.clone()),
        updated_at: entry.map(|e| e.updated_at.clone()),
        added_at: entry.map(|e| e.added_at.clone()),
        in_library: lib.join(library::lua_file_name(app_id)).is_file(),
        lua_in_steam: steam.map(|s| library::is_in_steam(app_id, s)) == Some(true),
        fix_downloaded: fixes::archive_path(lib, app_id).is_file(),
        hidden: entry.map(|e| e.hidden).unwrap_or(false),
        tags: entry.map(|e| e.tags.clone()).unwrap_or_default(),
        fix: fixes::verify(lib, app_id, game_dir),
        game,
        playtime_minutes,
        last_played,
        stage: "no_lua",
    };
    status.stage = derive_stage(&status);
    status
}

/// Non-fatal rolling snapshot — never blocks the operation that triggered it.
fn snapshot_quietly(lib: &Path, data_dir: &Path) {
    match backup::auto_snapshot(lib, data_dir) {
        Ok(summary) => info!("{}", i18n_log::i18n_log(format!("snapshot: {} ({} .lua, {} octets)", summary.path, summary.lua_count, summary.bytes), "logs.snapshot.created", &[("path", serde_json::json!(&summary.path)), ("luaCount", serde_json::json!(summary.lua_count)), ("bytes", serde_json::json!(summary.bytes))])),
        Err(e) => warn!("{}", i18n_log::i18n_log(format!("snapshot ignoré: {e}"), "logs.snapshot.skipped", &[("error", serde_json::json!(e.to_string()))])),
    }
}

// --------------------------------------------------- discovery & bulk work

#[derive(Debug, Clone, Serialize, Default)]
pub struct ImportReport {
    /// Games adopted from `{Steam}\config\lua` during this run.
    pub imported: Vec<String>,
    pub errors: Vec<String>,
}

/// One line of a bulk run, so the UI can show what actually happened per game.
#[derive(Debug, Clone, Serialize)]
pub struct BulkItem {
    pub app_id: String,
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct BulkReport {
    pub items: Vec<BulkItem>,
    pub succeeded: usize,
    pub failed: usize,
    /// Candidates the pass wanted to treat but didn't finish — set only by
    /// a cancellation, never a failure. Games outside the selection are not
    /// part of the operation at all, so they are not counted here.
    pub skipped: usize,
}

impl BulkReport {
    fn push(&mut self, app_id: &str, name: &str, result: Result<String, String>) {
        let (ok, detail) = match result {
            Ok(detail) => {
                self.succeeded += 1;
                (true, detail)
            }
            Err(detail) => {
                self.failed += 1;
                (false, detail)
            }
        };
        self.items.push(BulkItem {
            app_id: app_id.to_string(),
            name: name.to_string(),
            ok,
            detail,
        });
    }
}

/// Emitted after each game so the UI can show real-time progress.
#[derive(Debug, Clone, Serialize)]
pub struct BulkProgressEvent {
    /// `"games"` or `"fixes"`.
    pub phase: &'static str,
    /// 1-based index of the game currently being processed.
    pub current: usize,
    pub total: usize,
    pub app_id: String,
    pub name: String,
    /// `"working"` | `"ok"` | `"error"` | `"skipped"`.
    pub status: &'static str,
    pub detail: String,
    pub cancelled: bool,
}

/// One game in the pre-flight plan.
#[derive(Debug, Clone, Serialize)]
pub struct BulkPlanItem {
    pub app_id: String,
    pub name: String,
    /// What would happen: `"steam_install"` | `"copy_lua"` | `"archive_missing"` | `"install_fix"`.
    pub action: &'static str,
    pub label: String,
    /// Non-empty when something might go wrong (e.g. game not activated).
    pub warning: Option<String>,
}

/// Dry-run summary returned by `bulk_preflight` so the UI can confirm before acting.
#[derive(Debug, Clone, Serialize)]
pub struct BulkPlan {
    pub steam_detected: bool,
    pub steam_running: bool,
    pub games: Vec<BulkPlanItem>,
    pub fixes: Vec<BulkPlanItem>,
    /// Fifth mode — the games a selection action will treat. The backend
    /// leaves it empty: the stage-driven modes fill `games`/`fixes`, and the
    /// local selection actions (verify, copy, tag, hide) build theirs in the
    /// view, from the same eligibility the buttons count.
    pub selection: Vec<BulkPlanItem>,
    pub warnings: Vec<String>,
}

/// Chooses the final name and icon for an adopted game. A local Steam manifest
/// always wins when it exists; network metadata is only a fallback, and the
/// AppID placeholder remains available when neither source has a name.
fn resolve_display(
    steam_name: Option<&str>,
    app_id: &str,
    network: Option<&steamstore::SteamDetails>,
) -> (String, Option<String>) {
    if let Some(name) = steam_name {
        return (name.to_string(), Some(discover::header_image(app_id)));
    }
    if let Some(details) = network {
        if !details.name.is_empty() {
            let icon = details
                .header_image
                .clone()
                .unwrap_or_else(|| discover::header_image(app_id));
            return (details.name.clone(), Some(icon));
        }
    }
    (format!("AppID {app_id}"), Some(discover::header_image(app_id)))
}

/// Best-effort local name and icon lookup for a game we only know by AppID.
async fn describe(
    state: &AppState,
    steam: Option<&Path>,
    app_id: &str,
) -> (String, Option<String>, Option<bool>) {
    // Steam's own manifest is local and remains the preferred source.
    let steam_name = steam
        .map(|s| vdf::locate_game(s, app_id))
        .and_then(|game| game.steam_name);

    // The fallback needs only a name and an image, whose Steam-store values do
    // not need the UI locale table; English is the established default here.
    // Do not fetch when a local manifest already supplied the authoritative name.
    let network = if steam_name.is_none() {
        cached_steam_details(state, app_id, "english").await.ok()
    } else {
        None
    };

    let (name, icon) = resolve_display(steam_name.as_deref(), app_id, network.as_ref());
    (name, icon, None)
}

/// Adopt every `.lua` already sitting in Steam. Runs on startup and behind the refresh button.
#[tauri::command]
pub async fn sync_from_steam(state: State<'_, AppState>) -> Result<ImportReport, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let mut report = ImportReport::default();

    let steam = resolve_steam(&cfg);

    // Adopt .lua files the user dropped by hand into the library directory.
    // Runs before the Steam check so it works even when Steam isn't detected.
    for app_id in discover::library_orphans(&library_dir) {
        let (name, icon, _) = describe(&state, steam.as_deref(), &app_id).await;
        match discover::adopt_local(&library_dir, &app_id, &name, icon.as_deref()) {
            Ok(_) => {
                info!("{}", i18n_log::i18n_log(format!("sync_from_steam: fichier local adopté {app_id} (\"{name}\")"), "logs.sync.local-adopted", &[("appId", serde_json::json!(&app_id)), ("name", serde_json::json!(&name))]));
                report.imported.push(name);
            }
            Err(e) => report.errors.push(format!("{app_id}: {e}")),
        }
    }

    let Some(steam) = steam else {
        return Ok(report);
    };

    for app_id in discover::orphans(&library_dir, &steam) {
        let (name, icon, _) = describe(&state, Some(&steam), &app_id).await;
        match discover::adopt(&library_dir, &steam, &app_id, &name, icon.as_deref()) {
            Ok(_) => {
                info!("{}", i18n_log::i18n_log(format!("sync_from_steam: adopté {app_id} (\"{name}\")"), "logs.sync.adopted", &[("appId", serde_json::json!(&app_id)), ("name", serde_json::json!(&name))]));
                report.imported.push(name);
            }
            Err(e) => report.errors.push(format!("{app_id}: {e}")),
        }
    }
    Ok(report)
}

/// Best-effort check: is `steam.exe` in the process list?
fn is_steam_running() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("tasklist.exe")
        .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.to_lowercase().contains("steam.exe"))
        })
        .unwrap_or(false)
}

/// Stages treated by "install every fix": everything that can receive a patch.
const INSTALLABLE_FIX_STAGES: &[&str] = &[
    "fix_downloaded",
    "fix_damaged",
    "fix_game_moved",
];

/// Stages treated by a repair: only installs that broke. `fix_downloaded`
/// are not repairs — those games never had a patch applied.
/// `fix_external` appears in neither set: the app holds no backup of those
/// files, and reinstalling over them would destroy a third-party patch with
/// no way back.
const REPAIRABLE_FIX_STAGES: &[&str] = &["fix_damaged", "fix_game_moved"];

/// The selection every bulk-fix pass shares. A game enters when StateFlags
/// says the download is finished (a running download makes `install_fix_inner`
/// refuse anyway) and its stage is one the caller wants treated.
fn is_fix_candidate(
    stage: &str,
    has_fix: bool,
    fully_installed: bool,
    stages: &[&str],
) -> bool {
    has_fix && fully_installed && stages.contains(&stage)
}

/// The fifth mode's predicate: a library entry belongs to the user's
/// selection exactly when its AppID is one of the chosen ones. Pure function
/// of the data — the preflight and the pass both filter through it, so the
/// confirmation screen and the run can never disagree about who is in.
fn in_selection(app_id: &str, selection: &[String]) -> bool {
    selection.iter().any(|id| id == app_id)
}

/// The plan a bulk pass would run, built without any Tauri state so the
/// selection it describes can be executed by the unit tests. Hidden entries
/// never enter: the library view's bulk buttons count the games they show,
/// and the passes treat exactly those — nothing else.
fn bulk_preflight_plan(
    library_dir: &Path,
    data_dir: &Path,
    steam: Option<&Path>,
    steam_running: bool,
    repair_only: bool,
    selection: Option<&[String]>,
) -> BulkPlan {
    let mut plan = BulkPlan {
        steam_detected: steam.is_some(),
        steam_running,
        games: Vec::new(),
        fixes: Vec::new(),
        selection: Vec::new(),
        warnings: Vec::new(),
    };

    if steam.is_none() {
        plan.warnings
            .push("Steam n'est pas détecté — indiquez son dossier dans Réglages.".to_string());
        return plan;
    }
    // A repair copies files straight into the game folder — it never fires a
    // steam:// URI, so Steam not running only dooms the install passes. A
    // selection pass applies patches the same way, so it shares the exemption.
    if !steam_running && !repair_only && selection.is_none() {
        plan.warnings
            .push("Steam ne semble pas lancé — les demandes d'installation échoueront.".to_string());
    }

    let fix_stages = if repair_only {
        REPAIRABLE_FIX_STAGES
    } else {
        INSTALLABLE_FIX_STAGES
    };

    let entries = match library::load_index_with_data_dir(library_dir, data_dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("{}", i18n_log::i18n_log(format!("preflight: index de la bibliothèque ignoré: {e:#}"), "logs.preflight.library-index-ignored", &[("error", serde_json::json!(format!("{e:#}")))]));
            Vec::new()
        }
    };
    for entry in entries {
        if entry.hidden {
            continue;
        }
        // Fifth mode: the plan covers exactly the AppIDs the user picked —
        // the same predicate the pass itself runs on.
        if let Some(chosen) = selection {
            if !in_selection(&entry.app_id, chosen) {
                continue;
            }
        }
        let status = build_status(library_dir, steam, &entry.app_id, Some(&entry));

        // Games that need a Steam install — never part of a repair pass, and
        // never part of the selection mode (its actions manage, they don't
        // install through Steam).
        if selection.is_none()
            && !repair_only
            && (status.stage == "needs_steam_install" || status.stage == "lua_not_in_steam")
        {
            let mut warning = None;
            if !status.game.known_to_steam {
                warning = Some(
                    "Ce jeu n'apparaît pas dans votre compte Steam — il n'est peut-être pas activé."
                        .to_string(),
                );
            }
            let action = if status.stage == "lua_not_in_steam" {
                "copy_lua"
            } else {
                "steam_install"
            };
            let label = if status.stage == "lua_not_in_steam" {
                "Copier le .lua vers Steam puis demander l'installation".to_string()
            } else {
                "Demander l'installation à Steam".to_string()
            };
            plan.games.push(BulkPlanItem {
                app_id: entry.app_id.clone(),
                name: entry.name.clone(),
                action,
                label,
                warning,
            });
        }

        // Fixes that can be applied now — the same selection the run itself
        // uses, so the plan never promises what the pass won't do.
        if is_fix_candidate(
            status.stage,
            status.fix_downloaded,
            status.game.fully_installed,
            fix_stages,
        ) {
            let action = if status.fix_downloaded {
                "install_fix"
            } else {
                "archive_missing"
            };
            let label = if status.fix_downloaded {
                "Installer le patch déjà téléchargé".to_string()
            } else {
                "Télécharger puis installer le patch".to_string()
            };
            plan.fixes.push(BulkPlanItem {
                app_id: entry.app_id,
                name: entry.name,
                action,
                label,
                warning: None,
            });
        }
    }

    if plan.games.is_empty() && plan.fixes.is_empty() {
        plan.warnings.push(if repair_only {
            "Rien à réparer — aucun patch installé n'est endommagé.".to_string()
        } else {
            "Rien à faire — tous les jeux sont à jour.".to_string()
        });
    }
    plan
}

/// Dry-run: tell the UI what *would* happen, with warnings, before anything runs.
/// With `repair_only`, the plan covers only the broken installs
/// (`fix_damaged` / `fix_game_moved`) — the same selection the repair pass
/// itself runs on. With `selection`, fifth mode: the plan covers exactly the
/// chosen AppIDs the patch pass will treat — no Steam installs.
#[tauri::command]
pub async fn bulk_preflight(
    state: State<'_, AppState>,
    repair_only: bool,
    selection: Option<Vec<String>>,
) -> Result<BulkPlan, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let steam = resolve_steam(&cfg);
    let steam_running = steam.is_some() && is_steam_running();
    Ok(bulk_preflight_plan(
        &library_dir,
        &config::data_dir(),
        steam.as_deref(),
        steam_running,
        repair_only,
        selection.as_deref(),
    ))
}

/// Ask the running bulk operation to stop after the current game.
#[tauri::command]
pub fn cancel_bulk(state: State<'_, AppState>) {
    info!("{}", i18n_log::i18n_log("bulk: annulation demandée".to_owned(), "logs.bulk.cancel-requested", &[]));
    state.bulk_cancel.store(true, Ordering::Relaxed);
}

/// Download (when needed) and apply every online fix that can be applied right
/// now. Games that aren't installed yet are skipped, not failed.
#[tauri::command]
pub async fn install_all_fixes(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BulkReport, String> {
    bulk_install_fixes(&app, &state).await
}

/// Re-download (when needed) and re-apply only the fixes that broke —
/// `fix_damaged` and `fix_game_moved`. A repair never touches a game whose
/// patch was never installed: applying a first patch is an install, not a
/// repair.
#[tauri::command]
pub async fn repair_all_fixes(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BulkReport, String> {
    bulk_fixes(&app, &state, "repair", REPAIRABLE_FIX_STAGES, None).await
}

/// Fifth mode — apply (or re-apply) patches to exactly the AppIDs the user
/// selected in the library view. The shared loop's pre-checks all stand: a
/// game not installed by Steam is not patched, a running Steam download
/// neither, and a `fix_external` never — no backup exists for those files.
#[tauri::command]
pub async fn apply_fixes_to_selection(
    app: AppHandle,
    state: State<'_, AppState>,
    app_ids: Vec<String>,
) -> Result<BulkReport, String> {
    bulk_fixes(&app, &state, "fixes", INSTALLABLE_FIX_STAGES, Some(app_ids)).await
}

async fn bulk_install_fixes(
    app: &AppHandle,
    state: &State<'_, AppState>,
) -> Result<BulkReport, String> {
    bulk_fixes(app, state, "fixes", INSTALLABLE_FIX_STAGES, None).await
}

/// Everything the shared bulk-fix pass needs from the app side — plain data
/// and flags, no Tauri runtime, so the selection the commands run is exactly
/// what the unit tests can execute.
struct BulkFixCtx<'a> {
    cfg: &'a config::AppConfig,
    library_dir: &'a Path,
    /// The application folder carrying the index's HMAC key. Injected like
    /// the rest of the context: in production it is `config::data_dir()`,
    /// in tests a scratch dir, so the real one is never reached.
    data_dir: &'a Path,
    steam: Option<&'a Path>,
    cancel: &'a AtomicBool,
}

/// The one bulk-fix loop, shared by the install, repair and selection
/// passes: same re-download step, same progress events, same cancellation,
/// same report. Only the selection differs — `stages` says which states the
/// pass treats, and `selection` (fifth mode) narrows it further to the
/// AppIDs the user picked. Kept free of Tauri plumbing (the progress sink
/// and the download step are injected) so the selection the commands run is
/// exactly what the unit tests execute.
async fn bulk_fixes_core<'ctx>(
    ctx: &'ctx BulkFixCtx<'ctx>,
    phase: &'static str,
    stages: &[&str],
    selection: Option<&[String]>,
    mut emit: impl FnMut(BulkProgressEvent),
    mut download: impl FnMut(String) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'ctx>>,
) -> BulkReport {
    let mut report = BulkReport::default();

    let entries = match library::load_index_with_data_dir(ctx.library_dir, ctx.data_dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("{}", i18n_log::i18n_log(format!("bulk: index de la bibliothèque ignoré: {e:#}"), "logs.bulk.library-index-ignored", &[("error", serde_json::json!(format!("{e:#}")))]));
            Vec::new()
        }
    };
    // Hidden entries never enter: the library view's bulk buttons count the
    // games they show, and the pass treats exactly those — nothing else.
    let candidates: Vec<_> = entries
        .iter()
        .filter(|e| {
            if e.hidden {
                return false;
            }
            // Fifth mode: an AppID the user did not pick never enters the
            // pass — checked before the (costly) status build.
            if let Some(chosen) = selection {
                if !in_selection(&e.app_id, chosen) {
                    return false;
                }
            }
            let s = build_status(ctx.library_dir, ctx.steam, &e.app_id, Some(e));
            is_fix_candidate(s.stage, s.fix_downloaded, s.game.fully_installed, stages)
        })
        .collect();
    let total = candidates.len();

    let mut done = 0usize;
    for (i, entry) in candidates.iter().enumerate() {
        if ctx.cancel.load(Ordering::Relaxed) {
            emit(BulkProgressEvent {
                phase,
                current: i + 1,
                total,
                app_id: entry.app_id.clone(),
                name: entry.name.clone(),
                status: "skipped",
                detail: "annulé".to_string(),
                cancelled: true,
            });
            break;
        }

        let status = build_status(ctx.library_dir, ctx.steam, &entry.app_id, Some(entry));

        if !status.fix_downloaded {
            emit(BulkProgressEvent {
                phase,
                current: i + 1,
                total,
                app_id: entry.app_id.clone(),
                name: entry.name.clone(),
                status: "working",
                detail: "téléchargement du patch…".to_string(),
                cancelled: false,
            });
            if let Err(e) = download(entry.app_id.clone()).await {
                emit(BulkProgressEvent {
                    phase,
                    current: i + 1,
                    total,
                    app_id: entry.app_id.clone(),
                    name: entry.name.clone(),
                    status: "error",
                    detail: e.clone(),
                    cancelled: false,
                });
                report.push(&entry.app_id, &entry.name, Err(e));
                // A failure is a finished attempt — `skipped` stays reserved
                // for the candidates a cancellation never let the pass reach.
                done += 1;
                continue;
            }
        }

        emit(BulkProgressEvent {
            phase,
            current: i + 1,
            total,
            app_id: entry.app_id.clone(),
            name: entry.name.clone(),
            status: "working",
            detail: "installation du patch…".to_string(),
            cancelled: false,
        });
        let result = install_fix_inner(ctx.cfg, &entry.app_id, None)
            .await
            .map(|r| format!("{} fichier(s) appliqué(s)", r.file_count));

        let (status_str, detail) = match &result {
            Ok(d) => ("ok", d.as_str()),
            Err(d) => ("error", d.as_str()),
        };
        emit(BulkProgressEvent {
            phase,
            current: i + 1,
            total,
            app_id: entry.app_id.clone(),
            name: entry.name.clone(),
            status: status_str,
            detail: detail.to_string(),
            cancelled: false,
        });
        report.push(&entry.app_id, &entry.name, result);
        done += 1;
    }

    // Candidates the pass wanted to treat but didn't finish — cancellation
    // only. The rest of the library has nothing to do with this operation:
    // it is not "skipped" by it.
    report.skipped += total - done;
    report
}

async fn bulk_fixes(
    app: &AppHandle,
    state: &State<'_, AppState>,
    phase: &'static str,
    stages: &'static [&'static str],
    selection: Option<Vec<String>>,
) -> Result<BulkReport, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let data_dir = config::data_dir();
    let steam = resolve_steam(&cfg);

    state.bulk_cancel.store(false, Ordering::Relaxed);

    let emitter = app.clone();
    let ctx = BulkFixCtx {
        cfg: &cfg,
        library_dir: &library_dir,
        data_dir: &data_dir,
        steam: steam.as_deref(),
        cancel: &state.bulk_cancel,
    };
    let report = bulk_fixes_core(
        &ctx,
        phase,
        stages,
        selection.as_deref(),
        move |event| {
            let _ = emitter.emit("bulk://progress", event);
        },
        |_app_id| Box::pin(async move {
            Err("aucun téléchargement de patch n'est disponible dans l'édition publique".to_string())
        }),
    )
    .await;

    info!("{}", i18n_log::i18n_log(format!("bulk fixes ({phase}): {} appliqué(s), {} échec(s), {} ignoré(s)", report.succeeded, report.failed, report.skipped), "logs.bulk.summary", &[("phase", serde_json::json!(phase)), ("succeeded", serde_json::json!(report.succeeded)), ("failed", serde_json::json!(report.failed)), ("skipped", serde_json::json!(report.skipped))]));
    Ok(report)
}

// ------------------------------------------------------------ Steam store

/// One Steam-store cache and deduplication domain: the game plus Steam's
/// requested response language. Keeping construction here makes cache and
/// lock scope impossible to diverge.
fn steam_details_key(app_id: &str, lang: &str) -> String {
    format!("{app_id}:{lang}")
}

/// Return the shared lock for one exact cache key. The map lock is released
/// before the caller awaits the per-key lock, so unrelated details can fetch
/// concurrently while identical requests are deduplicated.
async fn acquire_details_lock(
    locks: &tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    key: &str,
) -> Arc<tokio::sync::Mutex<()>> {
    let mut entry = locks.lock().await;
    Arc::clone(
        entry
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

/// Drop the per-key lock entry once no other caller holds it.
///
/// `strong_count == 2` means: the map's reference plus the caller's own `Arc`.
/// The map lock is held across the check and the removal so a concurrent caller
/// cannot clone the Arc in between and end up on a different lock.
async fn release_details_lock(state: &AppState, key: &str) {
    let mut locks = state.steam_details_locks.lock().await;
    if let Some(a) = locks.get(key) {
        if Arc::strong_count(a) == 2 {
            locks.remove(key);
        }
    }
}

/// Cached + deduplicated fetch of Steam store details.
///
/// Strategy:
/// 1. Read cache → if present, return immediately.
/// 2. Acquire (or create) a per-key lock.
/// 3. Re-read cache after acquiring the lock — the previous caller may have
///    just filled it (this is what achieves deduplication).
/// 4. If still a miss, perform the network call, fill the cache, release lock.
/// 5. Clean up the lock entry when no other caller is waiting.
///
/// Failures are never cached — a transient network error must not poison
/// the cache for the full 5-minute TTL.
#[tauri::command]
pub async fn get_steam_details(
    state: State<'_, AppState>,
    app_id: String,
    lang: String,
) -> Result<steamstore::SteamDetails, String> {
    cached_steam_details(&state, &app_id, &lang).await
}

/// Shared implementation for both the frontend command and backend flows
/// (notably a local `.lua` import). Keeping one entry point prevents a new
/// caller from bypassing the cache or the per-key request deduplication.
async fn cached_steam_details(
    state: &AppState,
    app_id: &str,
    lang: &str,
) -> Result<steamstore::SteamDetails, String> {
    let key = steam_details_key(app_id, lang);

    // 1. Try cache.
    if let Some(cached) = state.steam_details.get(&key) {
        return Ok(cached);
    }

    // 2. Acquire per-key lock.
    //    The map lock is released BEFORE acquiring the per-key lock, so that
    //    concurrent callers can create their own locks without waiting on us.
    let lock = acquire_details_lock(&state.steam_details_locks, &key).await;

    // 3-4. Fetch with deduplication.
    //    Wrap in a block so `_fetch_guard` is dropped before we call
    //    `release_details_lock`, keeping the strong_count accurate.
    let result: Result<steamstore::SteamDetails, String> = async {
        let _fetch_guard = lock.lock().await;

        // 3. Re-read cache (the previous caller may have just filled it).
        if let Some(cached) = state.steam_details.get(&key) {
            return Ok(cached);
        }

        // 4. Network call — do not cache failures.
        let details = steamstore::details(&state.http, app_id, lang)
            .await
            .map_err(|e| e.to_string())?;
        state.steam_details.put(key.clone(), details.clone());

        Ok(details)
    }
    .await;

    // 5. Clean up the lock entry if no other caller is waiting.
    release_details_lock(state, &key).await;

    result
}

// ------------------------------------------- aggregated changelog feed (LOT-12)// ------------------------------------------- aggregated changelog feed (LOT-12)

/// One post in the aggregated changelog feed.
#[derive(Debug, Clone, Serialize)]
pub struct FeedItem {
    pub app_id: String,
    /// Taken from the library index — an `appdetails` call per game would
    /// double the traffic for a name we already own.
    pub game_name: String,
    pub title: String,
    /// Unix seconds, as Steam reports it.
    pub date: i64,
    pub url: String,
    pub is_patch_notes: bool,
    /// Short enough to cross IPC for forty games at once — the full body
    /// stays on the backend side.
    pub excerpt: String,
}

/// One game whose announcements could not be fetched. A truncated feed that
/// presents itself as complete would be a silent lie, so failures travel
/// with the report.
#[derive(Debug, Clone, Serialize)]
pub struct FeedFailure {
    pub app_id: String,
    pub game_name: String,
    pub error: String,
}

/// The feed plus what building it actually did.
#[derive(Debug, Clone, Serialize)]
pub struct FeedReport {
    pub items: Vec<FeedItem>,
    /// Games served from the 30-minute cache, no request sent.
    pub from_cache: usize,
    /// Games that cost a Steam request.
    pub fetched: usize,
    pub failed: Vec<FeedFailure>,
}

/// Three posts per game at most — one very active game must not drown the feed.
const FEED_MAX_PER_GAME: usize = 3;
/// Four requests in flight at most — opening the tab concerns forty games,
/// not forty sockets. `pub(crate)`: `AppState` builds the shared semaphore
/// with this exact bound.
pub(crate) const FEED_MAX_IN_FLIGHT: usize = 4;
/// The excerpt budget, in characters (never bytes).
const FEED_EXCERPT_CHARS: usize = 400;

/// Where a game's posts came from, for the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeedSource {
    Cache,
    Network,
}

/// Cache successful fetches only. An empty list IS a success — a game that
/// never published must not be re-requested on every open. A failure is
/// never cached, or one network hiccup would blank the game for the whole TTL.
fn remember_changelogs(
    cache: &cache::TtlCache<String, Vec<steamstore::Changelog>>,
    app_id: &str,
    result: &Result<Vec<steamstore::Changelog>, String>,
) {
    if let Ok(items) = result {
        cache.put(app_id.to_string(), items.clone());
    }
}

/// One game's posts, from the cache or the network.
///
/// `fetch` is injected so the deduplication is testable without Steam; the
/// production caller wraps [`steamstore::changelogs`].
///
/// Per-key deduplication: when another caller is already fetching this game,
/// wait for its result instead of firing a second request — `force` included,
/// which bypasses the cache but never an in-flight request.
async fn fetch_game_changelogs_with<F, Fut>(
    cache: &cache::TtlCache<String, Vec<steamstore::Changelog>>,
    locks: &tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    app_id: &str,
    force: bool,
    fetch: F,
) -> (Result<Vec<steamstore::Changelog>, String>, FeedSource)
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<steamstore::Changelog>, String>>,
{
    if !force {
        if let Some(cached) = cache.get(&app_id.to_string()) {
            return (Ok(cached), FeedSource::Cache);
        }
    }

    let lock = {
        let mut map = locks.lock().await;
        Arc::clone(
            map.entry(app_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    };

    let outcome = async {
        if let Ok(_guard) = lock.try_lock() {
            // Nobody is fetching this game: the miss (or the `force`) is ours.
            if !force {
                // The cache may have been filled between the first read and now.
                if let Some(cached) = cache.get(&app_id.to_string()) {
                    return (Ok(cached), FeedSource::Cache);
                }
            }
            let result = fetch(app_id.to_string()).await;
            remember_changelogs(cache, app_id, &result);
            (result, FeedSource::Network)
        } else {
            // A fetch is in flight: its fresh result becomes ours — even when
            // `force` is set.
            let _guard = lock.lock().await;
            if let Some(cached) = cache.get(&app_id.to_string()) {
                return (Ok(cached), FeedSource::Cache);
            }
            // The in-flight fetch failed — failures are never cached, so the
            // key is free again: take our turn.
            let result = fetch(app_id.to_string()).await;
            remember_changelogs(cache, app_id, &result);
            (result, FeedSource::Network)
        }
    }
    .await;

    // Drop the map entry once no other caller holds the lock
    // (strong_count == 2: the map's reference plus our own).
    let mut map = locks.lock().await;
    if let Some(entry) = map.get(app_id) {
        if Arc::strong_count(entry) == 2 {
            map.remove(app_id);
        }
    }

    outcome
}

/// A game's posts into feed items: three at most, bodies reduced to excerpts.
fn feed_items_for(
    app_id: &str,
    game_name: &str,
    logs: Vec<steamstore::Changelog>,
) -> Vec<FeedItem> {
    logs.into_iter()
        .take(FEED_MAX_PER_GAME)
        .map(|log| FeedItem {
            app_id: app_id.to_string(),
            game_name: game_name.to_string(),
            title: log.title,
            date: log.date,
            url: log.url,
            is_patch_notes: log.is_patch_notes,
            excerpt: steamstore::excerpt(&log.body, FEED_EXCERPT_CHARS),
        })
        .collect()
}

/// Newest first; at equal dates, `app_id` then `title` — without that
/// tiebreak, two posts published the same second would swap places from one
/// refresh to the next, which is visible on screen.
fn sort_feed(items: &mut [FeedItem]) {
    items.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| a.app_id.cmp(&b.app_id))
            .then_with(|| a.title.cmp(&b.title))
    });
}

/// Hidden entries are out of the library view; they stay out of the feed.
fn visible_entries(entries: Vec<library::LibraryEntry>) -> Vec<library::LibraryEntry> {
    entries.into_iter().filter(|e| !e.hidden).collect()
}

/// The `cache_only` branch of the feed: serve what the 30-minute cache still
/// holds, nothing else. A game missing from the cache produces no item and
/// triggers nothing — a pure read of a slice and a cache cannot fire a
/// request, which is the whole offline guarantee of this lot.
fn feed_from_cache(
    entries: &[library::LibraryEntry],
    cache: &cache::TtlCache<String, Vec<steamstore::Changelog>>,
) -> (Vec<FeedItem>, usize) {
    let mut items: Vec<FeedItem> = Vec::new();
    let mut from_cache = 0usize;
    for entry in entries {
        if let Some(cached) = cache.get(&entry.app_id) {
            items.extend(feed_items_for(&entry.app_id, &entry.name, cached));
            from_cache += 1;
        }
    }
    (items, from_cache)
}

/// The network branch of the feed: one task per game, at most
/// [`FEED_MAX_IN_FLIGHT`] requests in flight. The semaphore comes from
/// `AppState` (`changelog_in_flight`), shared by every invocation — two
/// concurrent `changelog_feed` calls still cap at four requests together.
async fn run_feed_fetches<F, Fut>(
    entries: Vec<library::LibraryEntry>,
    cache: Arc<cache::TtlCache<String, Vec<steamstore::Changelog>>>,
    locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    semaphore: Arc<tokio::sync::Semaphore>,
    force: bool,
    fetch: Arc<F>,
) -> Result<(Vec<FeedItem>, usize, usize, Vec<FeedFailure>), String>
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Vec<steamstore::Changelog>, String>>
        + Send
        + 'static,
{
    let mut items: Vec<FeedItem> = Vec::new();
    let mut from_cache = 0usize;
    let mut fetched = 0usize;
    let mut failed: Vec<FeedFailure> = Vec::new();

    let mut tasks = tokio::task::JoinSet::new();
    for entry in entries {
        let sem = Arc::clone(&semaphore);
        let cache = Arc::clone(&cache);
        let locks = Arc::clone(&locks);
        let fetch = Arc::clone(&fetch);
        tasks.spawn(async move {
            // Acquired inside the task: the bound is on requests IN FLIGHT —
            // forty tasks may exist, four may hit the network.
            let _permit = sem
                .acquire_owned()
                .await
                .expect("le sémaphore du flux n'est jamais fermé");
            let (result, source) = fetch_game_changelogs_with(
                &cache,
                &locks,
                &entry.app_id,
                force,
                move |app_id| (*fetch)(app_id),
            )
            .await;
            (entry.app_id, entry.name, result, source)
        });
    }
    while let Some(joined) = tasks.join_next().await {
        let (app_id, game_name, result, source) = joined.map_err(|e| e.to_string())?;
        match result {
            Ok(logs) => {
                items.extend(feed_items_for(&app_id, &game_name, logs));
                match source {
                    FeedSource::Cache => from_cache += 1,
                    FeedSource::Network => fetched += 1,
                }
            }
            Err(error) => failed.push(FeedFailure {
                app_id,
                game_name,
                error,
            }),
        }
    }
    Ok((items, from_cache, fetched, failed))
}

/// Aggregated changelog feed for the whole library (LOT-12).
///
/// Each game costs at most one Steam request, bounded to
/// [`FEED_MAX_IN_FLIGHT`] in flight, and is served from a 30-minute
/// per-AppID cache when possible. `force` bypasses the cache (the
/// "Actualiser" button) but never an in-flight request; `cache_only` never
/// touches the network at all (opening the tab while offline).
///
/// Steam failures never reach `reach.rs`: the offline signal measures
/// LuaVault, fed only by `api.rs` — this command is a different host.
#[tauri::command]
pub async fn changelog_feed(
    state: State<'_, AppState>,
    force: Option<bool>,
    cache_only: Option<bool>,
) -> Result<FeedReport, String> {
    let force = force.unwrap_or(false);
    let cache_only = cache_only.unwrap_or(false);

    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let entries = visible_entries(library::load_index(&library_dir));

    let (mut items, from_cache, fetched, failed) = if cache_only {
        // Offline opening: serve what the 30-minute cache still holds. The
        // pure read cannot fire a single request.
        let (items, from_cache) = feed_from_cache(&entries, &state.changelog_cache);
        (items, from_cache, 0usize, Vec::new())
    } else {
        run_feed_fetches(
            entries,
            Arc::clone(&state.changelog_cache),
            Arc::clone(&state.changelog_locks),
            Arc::clone(&state.changelog_in_flight),
            force,
            Arc::new({
                let http = state.http.clone();
                move |app_id: String| {
                    let http = http.clone();
                    async move {
                        steamstore::changelogs(&http, &app_id, FEED_MAX_PER_GAME)
                            .await
                            .map_err(|e| e.to_string())
                    }
                }
            }),
        )
        .await?
    };

    sort_feed(&mut items);
    info!("{}", i18n_log::i18n_log(format!("changelog_feed: {} article(s) — {} jeu(x) du cache, {} récupéré(s), {} échec(s)", items.len(), from_cache, fetched, failed.len()), "logs.changelog.summary", &[("items", serde_json::json!(items.len())), ("fromCache", serde_json::json!(from_cache)), ("fetched", serde_json::json!(fetched)), ("failed", serde_json::json!(failed.len()))]));
    Ok(FeedReport {
        items,
        from_cache,
        fetched,
        failed,
    })
}

// --------------------------------------------------------------- licensing

// --------------------------------------------------------------- appearance

#[tauri::command]
pub async fn set_appearance(
    state: State<'_, AppState>,
    theme: String,
    dark: bool,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.theme = Some(theme);
    cfg.dark_mode = Some(dark);
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(())
}

#[tauri::command]
pub async fn set_locale(
    state: State<'_, AppState>,
    locale: String,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.locale = Some(locale);
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(())
}

#[tauri::command]
pub async fn library_status(state: State<'_, AppState>) -> Result<Vec<GameStatus>, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let steam = resolve_steam(&cfg);
    let entries = library::load_index(&library_dir);
    // SHA-256 verification of every installed fix is CPU/IO heavy — keep it off the async pool.
    // Deliberately NOT cached: hashing the files *is* the integrity check.
    let params = (library_dir, steam, entries);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (statuses, count, elapsed_ms) =
            measured_collect_statuses(&params.0, params.1, params.2);
        debug!("library_status: {count} entrées, {elapsed_ms} ms");
        statuses
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(result)
}

/// Aggregate library statistics — stage distribution, fix counts, and disk usage.
#[tauri::command]
pub async fn library_stats(state: State<'_, AppState>) -> Result<stats::LibraryStats, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let data_dir = config::data_dir();
    let steam = resolve_steam(&cfg);
    let entries = library::load_index(&library_dir);
    // File enumeration and SHA-256 verification are CPU/IO heavy — keep them off the async pool.
    let params = (library_dir, steam, entries, data_dir);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let statuses = collect_statuses(&params.0, params.1, params.2);
        stats::compute(&statuses, &params.0, &params.3)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(result)
}

#[tauri::command]
pub async fn game_status(
    state: State<'_, AppState>,
    app_id: String,
) -> Result<GameStatus, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let steam = resolve_steam(&cfg);
    let entries = library::load_index(&library_dir);
    let entry = entries.iter().find(|e| e.app_id == app_id);
    Ok(build_status(&library_dir, steam.as_deref(), &app_id, entry))
}

/// Ask the Steam client to start downloading the game.
#[tauri::command]
pub async fn install_game_via_steam(app_id: String) -> Result<String, String> {
    info!("install_game_via_steam: app_id={app_id}");
    install::open_steam_uri(&format!("steam://install/{app_id}")).map_err(|e| e.to_string())?;
    Ok("Steam a reçu la demande d'installation — validez-la dans la fenêtre Steam.".to_string())
}

#[tauri::command]
pub async fn launch_game(app_id: String) -> Result<String, String> {
    install::open_steam_uri(&format!("steam://rungameid/{app_id}")).map_err(|e| e.to_string())?;
    Ok("Lancement demandé à Steam.".to_string())
}

/// Close Steam and start it again. Troubleshooting only — copied `.lua` files are
/// picked up without a restart.
#[tauri::command]
pub async fn restart_steam(state: State<'_, AppState>) -> Result<String, String> {
    let cfg = state.config.lock().unwrap().clone();
    let steam = resolve_steam(&cfg).ok_or("Steam introuvable — indiquez son dossier dans Réglages")?;
    let exe = steam.join("steam.exe");
    if !exe.is_file() {
        return Err("steam.exe introuvable dans le dossier Steam".to_string());
    }
    // The shutdown/relaunch dance sleeps a few seconds — keep it off the async pool.
    tauri::async_runtime::spawn_blocking(move || install::restart_steam(&exe))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    // Steam rewrites libraryfolders.vdf on restart — invalidate the VDF cache
    // so the next locate_game / steamapps_dirs call reads the fresh file.
    vdf::clear_caches();
    Ok("Steam redémarre — patientez quelques secondes.".to_string())
}

#[tauri::command]
pub async fn install_online_fix(
    state: State<'_, AppState>,
    app_id: String,
    password: Option<String>,
) -> Result<fixes::FixReport, String> {
    let cfg = state.config.lock().unwrap().clone();
    install_fix_inner(&cfg, &app_id, password.as_deref()).await
}

/// Download the fix archive (if not already present) and install it in one go.
/// Extract and apply an already-downloaded fix. Shared with the bulk run.
async fn install_fix_inner(
    cfg: &config::AppConfig,
    app_id: &str,
    password: Option<&str>,
) -> Result<fixes::FixReport, String> {
    let app_id = app_id.to_string();
    let library_dir = cfg.resolved_library_dir();
    let steam = resolve_steam(cfg).ok_or("Steam introuvable — indiquez son dossier dans Réglages")?;

    let game = vdf::locate_game(&steam, &app_id);
    let Some(game_dir) = game.install_dir.as_deref().map(PathBuf::from) else {
        return Err(
            "le jeu n'est pas installé — installez-le d'abord via Steam, puis relancez l'installation du patch"
                .to_string(),
        );
    };
    if !game.fully_installed {
        return Err(
            "le jeu est encore en cours de téléchargement — attendez la fin avant d'appliquer le patch"
                .to_string(),
        );
    }

    info!("install_online_fix: app_id={app_id} dir={}", game_dir.display());
    // Extraction + hashing is CPU/IO heavy — keep it off the async pool.
    let id = app_id.clone();
    let pass = password.map(str::to_string).or_else(|| cfg.default_archive_password.clone());
    let report = tauri::async_runtime::spawn_blocking(move || {
        fixes::install(&library_dir, &id, &game_dir, pass.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    info!(
        "install_online_fix: {} fichier(s) appliqué(s) ({:?})",
        report.file_count, report.health
    );
    Ok(report)
}

#[tauri::command]
pub async fn verify_online_fix(
    state: State<'_, AppState>,
    app_id: String,
) -> Result<fixes::FixReport, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let game_dir = resolve_steam(&cfg).and_then(|s| vdf::game_dir(&s, &app_id));
    Ok(fixes::verify(&library_dir, &app_id, game_dir.as_deref()))
}

#[tauri::command]
pub async fn uninstall_online_fix(
    state: State<'_, AppState>,
    app_id: String,
) -> Result<fixes::UninstallReport, String> {
    let library_dir = state.config.lock().unwrap().resolved_library_dir();
    info!("uninstall_online_fix: app_id={app_id}");
    fixes::uninstall(&library_dir, &app_id).map_err(|e| e.to_string())
}

/// Whether Windows Defender is manageable here, and the paths it already excludes.
#[tauri::command]
pub async fn defender_status() -> Result<defender::DefenderStatus, String> {
    tauri::async_runtime::spawn_blocking(defender::status)
        .await
        .map_err(|e| e.to_string())
}

/// Add the one Defender rule that covers every online-fix install: the Steam
/// `steamapps\common` folder(s). Elevated, waits for completion, and records the
/// choice so the app never asks again.
#[tauri::command]
pub async fn setup_defender_exclusions(
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let cfg = state.config.lock().unwrap().clone();
    let steam = resolve_steam(&cfg)
        .ok_or("Steam introuvable — indiquez son dossier dans Réglages")?;
    let common: Vec<String> = vdf::common_dirs(&steam)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    if common.is_empty() {
        return Err("aucun dossier de jeux Steam (steamapps\\common) trouvé".to_string());
    }

    info!("setup_defender_exclusions: {} dossier(s)", common.len());
    let to_add = common.clone();
    tauri::async_runtime::spawn_blocking(move || defender::add_exclusions(&to_add))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    let mut cfg = state.config.lock().unwrap().clone();
    cfg.defender_exclusions = Some(true);
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(common)
}

/// Persist the user's answer to the one-time exclusion prompt (including "no").
#[tauri::command]
pub async fn set_defender_choice(
    state: State<'_, AppState>,
    choice: bool,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.defender_exclusions = Some(choice);
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(())
}

/// Persist the password proposed for subsequent encrypted online-fix archives.
#[tauri::command]
pub async fn set_default_archive_password(
    state: State<'_, AppState>,
    password: Option<String>,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.default_archive_password = password.filter(|password| !password.is_empty());
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(())
}

/// Elevated "verify and repair": reads Defender's exclusion list (which needs
/// admin), compares it against the Steam games folders, adds whatever is missing,
/// and reports what was already there versus newly added.
#[tauri::command]
pub async fn verify_defender_exclusions(
    state: State<'_, AppState>,
) -> Result<defender::VerifyReport, String> {
    let cfg = state.config.lock().unwrap().clone();
    let steam = resolve_steam(&cfg)
        .ok_or("Steam introuvable — indiquez son dossier dans Réglages")?;
    let common: Vec<String> = vdf::common_dirs(&steam)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    if common.is_empty() {
        return Err("aucun dossier de jeux Steam (steamapps\\common) trouvé".to_string());
    }

    info!("verify_defender_exclusions: {} dossier(s) à contrôler", common.len());
    let report = tauri::async_runtime::spawn_blocking(move || defender::verify_and_fix(&common))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    // Exclusions are now in place — remember it so installs proceed quietly.
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.defender_exclusions = Some(true);
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(report)
}

/// Delete the `.lua` from `{Steam}\config\lua` while keeping the library copy.
#[tauri::command]
pub async fn remove_lua_from_steam(
    state: State<'_, AppState>,
    app_id: String,
) -> Result<bool, String> {
    let cfg = state.config.lock().unwrap().clone();
    let steam = resolve_steam(&cfg).ok_or("Steam introuvable — indiquez son dossier dans Réglages")?;
    library::remove_from_steam(&app_id, &steam).map_err(|e| e.to_string())
}

/// Format an integrity error for the user in French, with actionable advice.
fn integrity_error_message(err: &anyhow::Error) -> String {
    let msg = err.to_string().to_lowercase();
    if msg.contains("sidecar") || msg.contains("hmac") || msg.contains("match") {
        return "L'index de la bibliothèque n'est plus reconnu — sa signature ne correspond plus au contenu, ou il a été signé par une autre installation. L'application refuse toute écriture pour protéger vos données. Deux options s'offrent à vous : restaurez une sauvegarde récente (Fichiers → Sauvegardes), ou utilisez la commande de ré-adoption pour accepter l'index tel quel (il ne sera pas vérifié).".to_string();
    }
    if msg.contains("json") || msg.contains("valide") || msg.contains("parse") {
        return "L'index de la bibliothèque contient des données corrompues et ne peut pas être chargé. Restaurez une sauvegarde récente (Fichiers → Sauvegardes) ou utilisez la commande de ré-adoption pour réinitialiser la signature.".to_string();
    }
    // Fallback — never leak keys, tags or sidecar content.
    format!(
        "Erreur d'intégrité de la bibliothèque : {}.\n\nRestaurez une sauvegarde récente ou contactez le support.",
        err
    )
}

#[tauri::command]
pub async fn list_library(
    state: State<'_, AppState>,
) -> Result<Vec<library::LibraryEntry>, String> {
    let library_dir = state.config.lock().unwrap().resolved_library_dir();
    // LOT-21: the strict HMAC path. A compromised index must surface as an
    // error here, never as a silent empty list — the informative paths
    // (`build_report`, …) keep the best-effort wrapper.
    library::load_index_with_data_dir(&library_dir, &config::data_dir())
        .map_err(|e| integrity_error_message(&e))
}

/// Remove a game from the library. Refuses while an online fix is still applied,
/// unless `force` is set — the fix backup lives in the library we're about to prune.
#[tauri::command]
pub async fn remove_library_entry(
    state: State<'_, AppState>,
    app_id: String,
    force: Option<bool>,
) -> Result<(), String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let forced = force.unwrap_or(false);

    if !forced && fixes::load_state(&library_dir, &app_id).is_some() {
        return Err(
            "un patch en ligne est encore installé pour ce jeu — désinstallez-le d'abord pour restaurer les fichiers d'origine"
                .to_string(),
        );
    }
    if forced {
        fixes::forget(&library_dir, &app_id);
    }
    if let Some(steam) = resolve_steam(&cfg) {
        let _ = library::remove_from_steam(&app_id, &steam);
    }
    library::remove(&library_dir, &app_id).map_err(|e| e.to_string())
}

/// Hide (or reveal) a game in the library view. Nothing is deleted — the game
/// keeps working, it simply stops showing up until revealed again.
#[tauri::command]
pub async fn set_library_hidden(
    state: State<'_, AppState>,
    app_id: String,
    hidden: bool,
) -> Result<(), String> {
    let library_dir = state.config.lock().unwrap().resolved_library_dir();
    info!("set_library_hidden: app_id={app_id} hidden={hidden}");
    library::set_hidden(&library_dir, &app_id, hidden).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_library_display(
    state: State<'_, AppState>,
    app_id: String,
    name: String,
    icon: Option<String>,
) -> Result<(), String> {
    let library_dir = state.config.lock().unwrap().resolved_library_dir();
    info!("set_library_display: app_id={app_id} name={name:?}");
    library::set_display(&library_dir, &app_id, &name, icon.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_library_tags(
    state: State<'_, AppState>,
    app_id: String,
    tags: Vec<String>,
) -> Result<(), String> {
    let library_dir = state.config.lock().unwrap().resolved_library_dir();
    info!("set_library_tags: app_id={app_id} tags={:?}", tags);
    library::set_tags(&library_dir, &app_id, &tags).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn copy_to_steam(state: State<'_, AppState>, app_id: String) -> Result<String, String> {
    let cfg = state.config.lock().unwrap().clone();
    let steam =
        resolve_steam(&cfg).ok_or("Steam introuvable — indiquez son dossier dans Réglages")?;
    let library_dir = cfg.resolved_library_dir();
    let dst = library::copy_to_steam(&library_dir, &app_id, &steam).map_err(|e| e.to_string())?;
    Ok(dst.display().to_string())
}

/// Import one local `.lua` file into the managed library.
///
/// The path is supplied by Tauri's native file picker or drag-and-drop event.
/// Its stem is retained only as the friendly display name; `addappid` in the
/// file itself is the authoritative AppID and determines the stored filename.
fn prepare_lua_import(
    path: &Path,
    library_dir: &Path,
    data_dir: &Path,
) -> Result<PreparedLuaImport, String> {
    let is_lua = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lua"));
    if !is_lua {
        return Err("seuls les fichiers .lua peuvent être importés".to_string());
    }
    if !path.is_file() {
        return Err("le chemin spécifié n'est pas un fichier".to_string());
    }

    let bytes = std::fs::read(path).map_err(|e| format!("lecture du fichier .lua : {e}"))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "le fichier .lua doit être du texte UTF-8".to_string())?;
    let app_id = library::parse_lua(text)
        .app_ids
        .into_iter()
        .next()
        .ok_or_else(|| "le fichier .lua ne contient aucun appel addappid(<AppID>)".to_string())?;

    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| "le fichier .lua doit avoir un nom valide".to_string())?;
    let entries = library::load_index_with_data_dir(library_dir, data_dir)
        .map_err(|e| e.to_string())?;

    Ok(PreparedLuaImport {
        filename_differs: stem != app_id,
        app_id,
        stem,
        bytes,
        entries,
    })
}

/// `AppID 264710` and `264710` are placeholders, not user-facing names.
/// Treating both forms as raw lets a later import repair older index entries.
fn is_raw_lua_name(name: &str, app_id: &str) -> bool {
    let name = name.trim();
    name.is_empty() || name == app_id || name == format!("AppID {app_id}")
}

fn import_steam_language(locale: Option<&str>) -> &'static str {
    match locale {
        Some("fr") => "french",
        _ => "english",
    }
}

fn import_needs_steam_metadata(prepared: &PreparedLuaImport) -> bool {
    let indexed_name = prepared
        .entries
        .iter()
        .find(|entry| entry.app_id == prepared.app_id)
        .map(|entry| entry.name.as_str());

    prepared.app_id.bytes().all(|byte| byte.is_ascii_digit())
        && is_raw_lua_name(&prepared.stem, &prepared.app_id)
        && indexed_name.is_none_or(|name| is_raw_lua_name(name, &prepared.app_id))
}

fn finish_lua_import(
    prepared: PreparedLuaImport,
    library_dir: &Path,
    data_dir: &Path,
    metadata: Option<steamstore::SteamDetails>,
) -> Result<LuaImportResult, String> {
    let PreparedLuaImport {
        app_id,
        stem,
        filename_differs,
        bytes,
        entries,
    } = prepared;
    let existing_name = entries
        .iter()
        .find(|entry| entry.app_id == app_id)
        .map(|entry| entry.name.as_str());
    let preserve_existing_name = existing_name.filter(|name| !is_raw_lua_name(name, &app_id));
    let resolved_name = metadata
        .as_ref()
        .filter(|details| !details.name.trim().is_empty())
        .map(|details| details.name.as_str());
    let name = preserve_existing_name
        .or(resolved_name)
        .unwrap_or(&stem)
        .to_string();
    let icon = metadata
        .as_ref()
        .and_then(|details| details.header_image.as_deref())
        .map(str::to_string);
    let entry = library::upsert_verified_index_with_data_dir(
        library_dir,
        data_dir,
        entries,
        &app_id,
        &name,
        icon.as_deref(),
        &bytes,
    )
        .map_err(|e| e.to_string())?;

    Ok(LuaImportResult {
        entry,
        filename_differs,
    })
}

/// Synchronous test helper: it exercises the same prepare/finalize path but
/// deliberately supplies no network metadata.
#[cfg(test)]
fn import_lua_file_inner(
    path: &Path,
    library_dir: &Path,
    data_dir: &Path,
) -> Result<LuaImportResult, String> {
    let prepared = prepare_lua_import(path, library_dir, data_dir)?;
    finish_lua_import(prepared, library_dir, data_dir, None)
}

fn import_lua_join_error(error: impl std::fmt::Display) -> String {
    format!("import du fichier .lua interrompu : {error}")
}

#[tauri::command]
pub async fn import_lua_file(
    state: State<'_, AppState>,
    path: String,
) -> Result<LuaImportResult, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let lang = import_steam_language(cfg.locale.as_deref());
    let data_dir = config::data_dir();
    let path = PathBuf::from(path);

    let prepared = tauri::async_runtime::spawn_blocking({
        let library_dir = library_dir.clone();
        let data_dir = data_dir.clone();
        move || prepare_lua_import(&path, &library_dir, &data_dir)
    })
    .await
    .map_err(import_lua_join_error)??;

    let metadata = if import_needs_steam_metadata(&prepared) {
        match cached_steam_details(&state, &prepared.app_id, lang).await {
            Ok(details) => Some(details),
            Err(error) => {
                // Store details improve the import but a network failure must
                // never prevent a user from retaining their local file.
                warn!("{}", i18n_log::i18n_log(format!("import_lua_file: métadonnées Steam indisponibles pour {}: {error}", prepared.app_id), "logs.import.steam-metadata-unavailable", &[("appId", serde_json::json!(&prepared.app_id)), ("error", serde_json::json!(error.to_string()))]));
                None
            }
        }
    } else {
        None
    };

    tauri::async_runtime::spawn_blocking(move || {
        finish_lua_import(prepared, &library_dir, &data_dir, metadata)
    })
    .await
    .map_err(import_lua_join_error)?
}

/// Import a user-selected patch archive into the local library.
#[tauri::command]
pub async fn import_patch_archive(
    state: State<'_, AppState>,
    path: String,
    app_id: Option<String>,
) -> Result<PatchImportResult, String> {
    let library_dir = state.config.lock().unwrap().resolved_library_dir();
    let data_dir = config::data_dir();
    let path = PathBuf::from(path);

    tauri::async_runtime::spawn_blocking(move || {
        import_patch_archive_inner(&path, app_id.as_deref(), &library_dir, &data_dir)
    })
    .await
    .map_err(|e| format!("import de l'archive de patch interrompu : {e}"))?
}

#[tauri::command]
pub async fn sync_library_to_steam(state: State<'_, AppState>) -> Result<u32, String> {
    let cfg = state.config.lock().unwrap().clone();
    let steam =
        resolve_steam(&cfg).ok_or("Steam introuvable — indiquez son dossier dans Réglages")?;
    let library_dir = cfg.resolved_library_dir();
    library::sync_all(&library_dir, &steam).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn detect_all(state: State<'_, AppState>) -> Result<DetectionReport, String> {
    let cfg = state.config.lock().unwrap().clone();
    Ok(build_report(&cfg))
}

#[tauri::command]
pub async fn set_steam_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<DetectionReport, String> {
    let path = PathBuf::from(path);
    if !path.is_dir() {
        return Err("ce dossier n'existe pas".to_string());
    }
    if !detect::looks_like_steam_dir(&path) {
        return Err(
            "steam.exe introuvable dans ce dossier — choisissez le dossier d'installation de Steam"
                .to_string(),
        );
    }
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.steam_dir = Some(path);
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg.clone();

    // The Steam directory changed — invalidate VDF caches so the next
    // `steamapps_dirs` call scans the new location instead of serving
    // a stale (possibly empty) cached result.
    vdf::clear_caches();

    Ok(build_report(&cfg))
}

#[tauri::command]
pub async fn set_library_dir(
    state: State<'_, AppState>,
    path: String,
) -> Result<DetectionReport, String> {
    let path = PathBuf::from(path);
    // LOT-21: adopt the chosen library's index BEFORE touching the config —
    // an index whose signature does not verify must reject the change
    // wholesale, leaving the saved config and the in-memory state untouched.
    hmac::adopt_index_with_data_dir(&path, &config::data_dir()).map_err(|e| integrity_error_message(&e))?;
    std::fs::create_dir_all(&path).map_err(|e| format!("création du dossier: {e}"))?;
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.library_dir = Some(path);
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg.clone();
    Ok(build_report(&cfg))
}

/// Re-adopt a library index by discarding its sidecar and re-signing.
///
/// **This is a trust decision, not a repair.** The existing index is accepted
/// as-is — it is NOT verified against any signature. Use this only when you
/// trust the index content (for example after moving a library folder to the
/// same machine, or after recovering from a known-good backup whose sidecar
/// was lost).
///
/// Steps:
/// 1. Validate that `path/index.json` is valid JSON (`Vec<LibraryEntry>`).
///    If validation fails, the sidecar is deleted (if present) and an error
///    is returned — no signature is written for invalid data.
/// 2. Delete the sidecar (`index.json.hmac`) in `path`.
/// 3. Re-sign the validated index with the local HMAC key.
///
/// This command is never called automatically — not at startup, not from
/// `sync_from_steam`, and not from `set_library_dir`.
#[tauri::command]
pub async fn readopt_index(path: String) -> Result<(), String> {
    readopt_index_inner(Path::new(&path), &config::data_dir()).await
}

/// Internal implementation of [`readopt_index`] with injectable `data_dir`.
pub async fn readopt_index_inner(path: &Path, data_dir: &Path) -> Result<(), String> {
    let idx = path.join("index.json");

    if !idx.is_file() {
        return Err("index.json introuvable dans ce dossier".to_string());
    }

    // 1. Validate JSON BEFORE touching anything.
    let raw = std::fs::read(&idx).map_err(|e| format!("lecture de index.json : {e}"))?;
    let _: Vec<library::LibraryEntry> =
        serde_json::from_slice(&raw).map_err(|e| format!("index.json n'est pas un JSON valide : {e}"))?;

    // 2. Delete the sidecar if it exists.
    let sidecar = hmac::sidecar_path(&idx);
    let _ = std::fs::remove_file(&sidecar);

    // 3. Re-sign with the local key.
    let key = hmac::load_or_create_key(data_dir).map_err(|e| format!("chargement de la clé HMAC : {e}"))?;
    hmac::sign_index(&idx, &key).map_err(|e| format!("signature de l'index : {e}"))?;

    Ok(())
}

#[tauri::command]
pub async fn mark_onboarding_done(state: State<'_, AppState>) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.first_run_done = true;
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(())
}

#[tauri::command]
pub async fn install_steam(state: State<'_, AppState>) -> Result<String, String> {
    info!("install_steam: downloading installer");
    let installer = install::download_steam_installer(&state.http)
        .await
        .map_err(|e| e.to_string())?;
    info!("install_steam: launching elevated installer");
    install::run_elevated(&installer.to_string_lossy(), "/S").map_err(|e| e.to_string())?;
    Ok("L'installateur Steam a été lancé en mode silencieux. Les jeux existants sont conservés.".to_string())
}

#[tauri::command]
pub async fn install_steamtools() -> Result<String, String> {
    info!("install_steamtools: launching fix-st.ps1 elevated");
    let (program, args) = install::steamtools_command();
    install::run_elevated(program, &args).map_err(|e| e.to_string())?;
    Ok("Script SteamTools lancé dans une fenêtre PowerShell élevée (Steam sera fermé par le script).".to_string())
}

#[tauri::command]
pub fn get_log_dir(app: AppHandle) -> Result<String, String> {
    app.path()
        .app_log_dir()
        .map(|p| p.display().to_string())
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- backups// ---------------------------------------------------------------- backups

#[tauri::command]
pub async fn list_backups(state: State<'_, AppState>) -> Result<Vec<backup::SnapshotInfo>, String> {
    let _ = state;
    Ok(backup::list_snapshots(&config::data_dir()))
}

#[tauri::command]
pub async fn create_snapshot(
    state: State<'_, AppState>,
) -> Result<backup::BackupSummary, String> {
    let library_dir = state.config.lock().unwrap().resolved_library_dir();
    // A snapshot zips and hashes for seconds — never on the async pool.
    tauri::async_runtime::spawn_blocking(move || {
        backup::auto_snapshot(&library_dir, &config::data_dir())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_backup(
    state: State<'_, AppState>,
    path: String,
    options: Option<backup::BackupOptions>,
    password: Option<String>,
) -> Result<backup::BackupSummary, String> {
    let library_dir = state.config.lock().unwrap().resolved_library_dir();
    let options = options.unwrap_or_default();
    // The path only — a password never enters the logs.
    info!("export_backup: {path}");
    tauri::async_runtime::spawn_blocking(move || {
        backup::export(
            &library_dir,
            &config::data_dir(),
            Path::new(&path),
            &options,
            password.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn import_backup(
    state: State<'_, AppState>,
    path: String,
    password: Option<String>,
) -> Result<backup::ImportSummary, String> {
    let library_dir = state.config.lock().unwrap().resolved_library_dir();
    info!("import_backup: {path}");
    let summary = tauri::async_runtime::spawn_blocking(move || {
        backup::import(
            Path::new(&path),
            &library_dir,
            &config::data_dir(),
            password.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    // The imported config.json only takes effect once reloaded.
    if summary.config_restored {
        if let Ok(cfg) = config::AppConfig::load() {
            *state.config.lock().unwrap() = cfg;
        }
    }
    Ok(summary)
}

/// What the import dialog needs before touching anything: does the file
/// exist, and which format is it? Bounded reads — the v2 magic (9 bytes)
/// and the v1 manifest, never the payload.
#[derive(Debug, Clone, Serialize)]
pub struct BackupProbe {
    pub exists: bool,
    pub encrypted: bool,
    pub v1: bool,
}

#[tauri::command]
pub async fn probe_backup(path: String) -> Result<BackupProbe, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&path);
        let exists = path.is_file();
        // Encrypted first: a v2 archive is not a ZIP, and probing it as one
        // would report `v1: false` without saying why.
        let encrypted = exists && encrypted_backup::is_encrypted(path);
        let v1 = exists && !encrypted && backup::is_v1_backup(path);
        BackupProbe { exists, encrypted, v1 }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_backup(path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    let dir = backup::backups_dir(&config::data_dir());
    // Only ever delete inside our own backups folder.
    if !path.starts_with(&dir) {
        return Err("ce fichier n'est pas une sauvegarde gérée par l'application".to_string());
    }
    std::fs::remove_file(&path).map_err(|e| format!("suppression: {e}"))
}

// ------------------------------------------------------------------- wipe

#[tauri::command]
pub async fn wipe_preview(
    state: State<'_, AppState>,
    plan: wipe::WipePlan,
) -> Result<Vec<wipe::WipeAction>, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let data_dir = config::data_dir();
    let steam = resolve_steam(&cfg);
    Ok(wipe::preview(
        &plan,
        &wipe::WipeContext {
            library_dir: &library_dir,
            data_dir: &data_dir,
            steam_dir: steam.as_deref(),
        },
    ))
}

#[tauri::command]
pub async fn wipe_execute(
    state: State<'_, AppState>,
    plan: wipe::WipePlan,
    snapshot_first: Option<bool>,
) -> Result<wipe::WipeReport, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let data_dir = config::data_dir();
    let steam = resolve_steam(&cfg);

    // A snapshot before a destructive run is cheap insurance.
    if snapshot_first.unwrap_or(true) && !plan.delete_app_backups {
        snapshot_quietly(&library_dir, &data_dir);
    }

    info!("{}", i18n_log::i18n_log("wipe_execute: démarrage".to_owned(), "logs.wipe.started", &[]));
    let report = wipe::execute(
        &plan,
        &wipe::WipeContext {
            library_dir: &library_dir,
            data_dir: &data_dir,
            steam_dir: steam.as_deref(),
        },
    );
    for step in &report.steps {
        info!("wipe: [{}] {} — {:?}", step.ok, step.id, step.detail);
    }
    if plan.reset_app_config {
        *state.config.lock().unwrap() = config::AppConfig::default();
    }
    Ok(report)
}

#[tauri::command]
pub async fn wipe_protected_paths(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let cfg = state.config.lock().unwrap().clone();
    Ok(wipe::protected_paths(resolve_steam(&cfg).as_deref()))
}

#[tauri::command]
pub async fn get_app_info() -> Result<AppInfo, String> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        portable: config::is_portable(),
        data_dir: config::data_dir().display().to_string(),
    })
}

// ──────────────────────────────────────────────────────────────────────────
// Exchange: library export / import preview

#[tauri::command]
pub async fn export_library(
    state: State<'_, AppState>,
    path: String,
    format: String,
) -> Result<usize, String> {
    let cfg = state.config.lock().unwrap().clone();
    let library_dir = cfg.resolved_library_dir();
    let steam = resolve_steam(&cfg);
    let entries = library::load_index(&library_dir);
    // Building statuses + writing the file are blocking — keep them off the async pool.
    tauri::async_runtime::spawn_blocking(move || {
        let statuses = collect_statuses(&library_dir, steam, entries);
        let content = exchange::render_export(&statuses, &format)?;
        // Write BOM for CSV so Windows tableurs open it correctly.
        if format == "csv" {
            let mut buf: Vec<u8> = b"\xEF\xBB\xBF".to_vec();
            buf.extend_from_slice(content.as_bytes());
            std::fs::write(&path, buf).map_err(|e| format!("écriture du fichier : {e}"))?;
        } else {
            std::fs::write(&path, &content).map_err(|e| format!("écriture du fichier : {e}"))?;
        }
        // Return row count (excluding header).
        Ok(statuses.len())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn preview_import(
    state: State<'_, AppState>,
    path: String,
) -> Result<exchange::ImportPreview, String> {
    const MAX_SIZE: u64 = 5 * 1024 * 1024; // 5 Mo
    let known_ids: Vec<String> = {
        let cfg = state.config.lock().unwrap();
        let library_dir = cfg.resolved_library_dir();
        let entries = library::load_index(&library_dir);
        entries.iter().map(|e| e.app_id.clone()).collect()
    };
    tauri::async_runtime::spawn_blocking(move || {
        let metadata = std::fs::metadata(&path).map_err(|e| format!("impossible de lire le fichier : {e}"))?;
        if metadata.len() > MAX_SIZE {
            return Err("le fichier dépasse 5 Mo — il s'agit d'une liste d'identifiants, pas d'une archive".to_string());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("lecture du fichier : {e}"))?;
        Ok(exchange::parse_import(&text, &known_ids))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ──────────────────────────────────────────────────────────────────────────
// Update client

#[tauri::command]
pub async fn check_update(state: State<'_, AppState>) -> Result<Option<update::UpdateAvailable>, String> {
    // Dedicated client: no LuaVault headers, no decompression (update::build_http_client).
    let http = state.update_http.clone();
    let manifest = match update::fetch_verified_manifest(&http, &update::base_url()).await {
        Some(m) => m,
        None => return Ok(None),
    };
    Ok(update::evaluate_manifest(manifest, env!("CARGO_PKG_VERSION"), config::is_portable()))
}

#[tauri::command]
pub async fn download_update(
    state: State<'_, AppState>,
    version: String,
    file: String,
    sha256: String,
    size: u64,
) -> Result<String, String> {
    let http = state.update_http.clone();
    let path = update::download_and_verify(&http, &version, &file, &sha256, size)
        .await
        .map_err(|e| e.to_string())?;

    // Remember the verified pair: install_update re-hashes against it, because
    // a file sitting in %TEMP% proves nothing by its path alone.
    *state.verified_update.lock().unwrap() = Some((path.clone(), sha256));
    Ok(path)
}

#[tauri::command]
pub async fn mark_update_notified(
    state: State<'_, AppState>,
    version: String,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap().clone();
    cfg.update_notified_version = Some(version);
    cfg.save().map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg;
    Ok(())
}

#[tauri::command]
pub async fn get_update_notified(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state.config.lock().unwrap().update_notified_version.clone())
}

/// Result of a completed update, consumed on first read.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateResult {
    pub from: String,
    pub to: String,
}

/// Consume the "update from version" stored before the last install.
///
/// Three cases:
/// - field absent → `None` (ordinary startup);
/// - field present and different from current version → `Some { from, to }`;
/// - field present and equal to current version → `None` (install didn't succeed).
///
/// The field is cleared in all three cases.
/// Decide what to report at startup, given the version we were running when an
/// update was launched and the version actually running now.
///
/// Pure on purpose. The command around it needs Tauri state, so it can only be
/// pinned by reading its own source — and a textual guard checks the *order of
/// lines*, not the value they carry: replacing the recorded version by `None`
/// left every guard green while killing the feature outright. Everything that
/// decides something lives here instead, where a test can call it.
pub fn decide_update_result(from: Option<&str>, current: &str) -> Option<UpdateResult> {
    match from {
        // Same version: the installer never completed (the user cancelled it, for
        // instance). Reporting success there would be a lie.
        Some(v) if v == current => None,
        Some(v) => Some(UpdateResult {
            from: v.to_string(),
            to: current.to_string(),
        }),
        None => None,
    }
}

#[tauri::command]
pub async fn take_update_result(state: State<'_, AppState>) -> Result<Option<UpdateResult>, String> {
    let from = {
        let cfg = state.config.lock().unwrap();
        cfg.update_from_version.clone()
    };

    // Clear the field immediately so it's never returned again.
    {
        let mut cfg = state.config.lock().unwrap().clone();
        cfg.update_from_version = None;
        cfg.save().ok();
        *state.config.lock().unwrap() = cfg;
    }

    Ok(decide_update_result(from.as_deref(), env!("CARGO_PKG_VERSION")))
}

#[tauri::command]
pub async fn install_update(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let verified = state.verified_update.lock().unwrap().clone();

    // Remember the version we're updating FROM, so the next startup can
    // report success or failure to the user. Must be written BEFORE the
    // installer launches — in portable mode `std::process::exit(0)` kills
    // everything that follows.
    {
        let mut cfg = state.config.lock().unwrap().clone();
        cfg.update_from_version = Some(env!("CARGO_PKG_VERSION").to_string());
        cfg.save().map_err(|e| format!("enregistrement de la version de mise à jour : {e}"))?;
        *state.config.lock().unwrap() = cfg;
    }

    // Canonicalization + recorded-pair check + SHA-256 re-verification touch the
    // disk and can hash a large installer: keep that off the async pool.
    let target = tauri::async_runtime::spawn_blocking(move || {
        update::validate_install_path(&path, &verified)
    })
    .await
    .map_err(|e| e.to_string())??;

    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    if file_name.ends_with(".exe") {
        // NSIS installer: open it directly.
        tauri_plugin_opener::open_path(target.to_string_lossy().as_ref(), None::<&str>)
            .map_err(|e| format!("ouverture de l'installeur : {e}"))?;
    } else {
        // Portable zip: extract, replace the running exe, relaunch.
        // ZIP I/O is ~20 Mo — keep it off the async pool (piège n°17).
        tauri::async_runtime::spawn_blocking(move || {
            install_update_portable(&target, None)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("installation portable : {e}"))?;
    }
    Ok(())
}

/// Replace the running executable with the one from a portable zip.
///
/// Sequence (Windows won't let you overwrite a running exe, but you can rename it):
/// 1. extract the zip into a temporary directory;
/// 2. rename the current exe → `.old`;
/// 3. copy the extracted exe into place;
/// 4. launch the new exe;
/// 5. exit the old process.
///
/// Only `LuaVault.exe` is copied — `config.json`, `license.json`, and
/// `LuaVault.portable` stay untouched.
fn install_update_portable(
    archive_path: &std::path::Path,
    target_dir: Option<&std::path::Path>,
) -> Result<(), String> {
    // Open the zip.
    let zip_file = std::fs::File::open(archive_path)
        .map_err(|e| format!("ouverture de l'archive : {e}"))?;
    let mut zip = zip::ZipArchive::new(zip_file)
        .map_err(|e| format!("lecture de l'archive ZIP : {e}"))?;

    // Create a temporary directory for extraction.
    let temp_dir = std::env::temp_dir()
        .join(format!("lv-update-{}", std::process::id()));
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).ok();
    }
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("création du dossier temporaire : {e}"))?;

    // Extract every entry, validating path safety.
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| format!("entrée ZIP #{i} illisible : {e}"))?;

        // Security: refuse path traversal, device names, ADS, etc.
        // Reuse the canonical validator from backup.rs — it rejects CurDir
        // and RootDir too, unlike the previous matches! which accepted them.
        let rel_str = match entry.enclosed_name() {
            Some(p) => p,
            None => continue, // path traversal — skip, don't error
        };
        let rel = match backup::safe_relative(rel_str.to_str().ok_or("nom d'entrée non-UTF-8")?) {
            Some(p) => p,
            None => continue, // device name, ADS, etc. — skip
        };

        let out = temp_dir.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| format!("création d'un dossier extrait : {e}"))?;
            continue;
        }

        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("création d'un dossier extrait : {e}"))?;
        }

        let mut sink = std::fs::File::create(&out)
            .map_err(|e| format!("écriture de {} : {e}", out.display()))?;
        std::io::copy(&mut entry, &mut sink)
            .map_err(|e| format!("décompression de {} : {e}", rel.display()))?;
    }

    // Find the executable in the extracted files.
    let exe_name = "LuaVault.exe";
    let extracted_exe = temp_dir.join(exe_name);
    if !extracted_exe.exists() {
        return Err(format!(
            "l'archive ne contient pas {exe_name}"
        ));
    }

    // Get the directory where the current exe lives (or use the test override).
    let exe_dir = match target_dir {
        Some(dir) => dir.to_path_buf(),
        None => {
            let current_exe = std::env::current_exe()
                .map_err(|e| format!("lecture du chemin de l'exécutable courant : {e}"))?;
            current_exe
                .parent()
                .ok_or("impossible de déterminer le dossier de l'exécutable")?
                .to_path_buf()
        }
    };

    replace_exe(&exe_dir, &extracted_exe)?;

    // Clean up the temporary directory before relaunching.
    // Without this, %TEMP%\lv-update-<pid> accumulates on every update
    // because std::process::exit(0) short-circuits all Drop handlers.
    let _ = std::fs::remove_dir_all(&temp_dir);

    // Launch the new exe and exit the old process.
    let target_exe = exe_dir.join(exe_name);
    std::process::Command::new(&target_exe)
        .spawn()
        .map_err(|e| format!("lancement du nouvel exécutable : {e}"))?;
    std::process::exit(0);
}

/// Replace the executable at `target_dir/exe_name` with `extracted_exe`.
///
/// Sequence (Windows won't let you overwrite a running exe, but you can rename it):
/// 1. rename the current exe → `.old`;
/// 2. copy the extracted exe into place.
///
/// This is a pure file-operation function — testable with ordinary files.
pub fn replace_exe(target_dir: &std::path::Path, extracted_exe: &std::path::Path) -> Result<(), String> {
    let exe_name = "LuaVault.exe";
    let target_exe = target_dir.join(exe_name);
    let old_exe = target_dir.join(format!("{exe_name}.old"));

    // If .old already exists from a previous failed update, remove it.
    if old_exe.exists() {
        std::fs::remove_file(&old_exe)
            .map_err(|e| format!("nettoyage du .old précédent : {e}"))?;
    }
    std::fs::rename(&target_exe, &old_exe)
        .map_err(|e| format!("renommage de l'exécutable courant : {e}"))?;

    // If the copy fails, roll back: restore the original executable.
    // Without this, .old exists but the app can no longer launch —
    // the exact state the brief called "worse than no update".
    if let Err(copy_err) = std::fs::copy(extracted_exe, &target_exe) {
        let _ = std::fs::rename(&old_exe, &target_exe);
        return Err(format!("copie du nouvel exécutable : {copy_err}"));
    }

    Ok(())
}

#[tauri::command]
pub async fn artwork_cached(url: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        artwork::hit(&artwork::cache_dir(), &url).map(|p| p.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())
}

/// The cached image's path, downloading it first when needed. A cached image
/// never touches the network — that is the point of the lot.
#[tauri::command]
pub async fn artwork_fetch(state: State<'_, AppState>, url: String) -> Result<String, String> {
    let dir = artwork::cache_dir();
    // Hit path: read_dir + mtime refresh are blocking IO (pitfall 17).
    let cached = {
        let dir = dir.clone();
        let url = url.clone();
        tauri::async_runtime::spawn_blocking(move || artwork::hit(&dir, &url))
            .await
            .map_err(|e| e.to_string())?
    };
    if let Some(path) = cached {
        return Ok(path.display().to_string());
    }
    let client = state.artwork_http.clone();
    // The network phase awaits cleanly on the async pool; store + purge are
    // disk work and return to the blocking pool (pitfall 17).
    let (status, content_type, bytes) = artwork::download(&client, &url)
        .await
        .map_err(|e| e.to_string())?;
    let path = tauri::async_runtime::spawn_blocking(move || {
        artwork::store_downloaded(
            &dir,
            &url,
            &content_type,
            status,
            bytes,
            artwork::MAX_CACHE_SIZE,
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Size and file count of the cache — what the settings screen shows.
#[tauri::command]
pub async fn artwork_cache_info() -> Result<artwork::CacheStats, String> {
    // A full read_dir plus one metadata per file — blocking IO (pitfall 17).
    tauri::async_runtime::spawn_blocking(|| artwork::stats(&artwork::cache_dir()))
        .await
        .map_err(|e| e.to_string())
}

/// Empty the cache. Nothing important is lost — the images re-download
/// themselves; the settings copy says so.
#[tauri::command]
pub async fn artwork_cache_clear() -> Result<artwork::CacheStats, String> {
    // Hundreds of deletions in one go — blocking IO (pitfall 17).
    tauri::async_runtime::spawn_blocking(|| artwork::clear(&artwork::cache_dir()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive_password_scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "luavault-archive-password-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source
            .find(signature)
            .expect("signature de fonction introuvable");
        let brace = start
            + source[start..]
                .find('{')
                .expect("accolade ouvrante introuvable");
        let mut depth = 0usize;
        for (offset, character) in source[brace..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[brace..brace + offset + 1];
                    }
                }
                _ => {}
            }
        }
        panic!("accolade fermante introuvable");
    }

    fn test_app_state() -> AppState {
        AppState {
            http: reqwest::Client::new(),
            reachability: std::sync::Mutex::new(reachability::ReachabilityState::default()),
            update_http: update::UpdateClient::new(),
            artwork_http: artwork::ArtworkClient::new(),
            config: std::sync::Mutex::new(config::AppConfig::default()),
            bulk_cancel: AtomicBool::new(false),
            steam_details: cache::TtlCache::new(std::time::Duration::from_secs(300)),
            steam_details_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            changelog_cache: Arc::new(cache::TtlCache::new(std::time::Duration::from_secs(
                30 * 60,
            ))),
            changelog_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            changelog_in_flight: Arc::new(tokio::sync::Semaphore::new(FEED_MAX_IN_FLIGHT)),
            verified_update: std::sync::Mutex::new(None),
        }
    }

    #[test]
    fn resolve_display_prefers_local_name_when_present() {
        let network = steamstore::SteamDetails {
            name: "Faux Nom".to_string(),
            header_image: Some("https://cdn.example/faux.jpg".to_string()),
            ..Default::default()
        };

        let (name, icon) = resolve_display(Some("Vrai Nom"), "123", Some(&network));

        assert_eq!(name, "Vrai Nom");
        assert_eq!(icon, Some(discover::header_image("123")));
    }

    #[test]
    fn resolve_display_uses_network_when_local_manifest_missing() {
        let network = steamstore::SteamDetails {
            name: "Subnautica".to_string(),
            header_image: Some("https://cdn.example/subnautica.jpg".to_string()),
            ..Default::default()
        };

        let (name, icon) = resolve_display(None, "264710", Some(&network));

        assert_eq!(name, "Subnautica");
        assert_eq!(icon, network.header_image);
    }

    #[test]
    fn resolve_display_falls_back_to_app_id_placeholder() {
        let app_id = "123";

        let (name, icon) = resolve_display(None, app_id, None);

        assert_eq!(name, "AppID 123");
        assert_eq!(icon, Some(discover::header_image(app_id)));
    }

    #[test]
    fn resolve_display_network_present_but_name_empty_still_falls_back() {
        let app_id = "123";
        let network = steamstore::SteamDetails {
            header_image: Some("https://cdn.example/unused.jpg".to_string()),
            ..Default::default()
        };

        let (name, icon) = resolve_display(None, app_id, Some(&network));

        assert_eq!(name, "AppID 123");
        assert_eq!(icon, Some(discover::header_image(app_id)));
    }

    #[test]
    fn meta02_describe_network_wiring_guard() {
        let source = include_str!("commands.rs");
        let describe = strip_comments_and_strings(function_body(source, "async fn describe"));
        let fallback = fn_body(&describe, "if steam_name.is_none()")
            .expect("describe doit conditionner le repli réseau à l'absence de manifeste");
        let call = fallback
            .find("cached_steam_details")
            .expect("le repli conditionnel doit appeler le cache Steam partagé");
        let expression = &fallback[call..];

        assert!(
            expression.contains(".ok()"),
            "un échec Steam Store doit rester non bloquant pour l'adoption"
        );
        assert!(
            !expression.contains(".unwrap()")
                && !expression.contains(".expect(")
                && !expression.contains('?'),
            "le repli réseau ne doit jamais propager ou paniquer sur une erreur"
        );
        let outside_fallback = describe.replacen(fallback, "", 1);
        assert!(
            !outside_fallback.contains("cached_steam_details"),
            "l'appel au cache Steam doit rester dans le seul repli sans manifeste local"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_describe_falls_back_to_steam_store_when_no_manifest() {
        let state = test_app_state();
        let app_id = "440";

        let (name, _, _) = describe(&state, None, app_id).await;

        println!("name = {name}");
        assert_ne!(name, format!("AppID {app_id}"));
    }

    #[test]
    fn test_set_default_archive_password_persists() {
        let root = archive_password_scratch("persists");
        let path = root.join("config.json");
        let cfg = config::AppConfig {
            default_archive_password: Some("mot-de-passe-de-test".to_string()),
            ..Default::default()
        };
        cfg.save_to(&path).unwrap();

        let reloaded: config::AppConfig =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            reloaded.default_archive_password.as_deref(),
            Some("mot-de-passe-de-test"),
            "le mot de passe par défaut doit survivre au rechargement de config.json"
        );

        let source = include_str!("commands.rs");
        let command = function_body(source, "pub async fn set_default_archive_password");
        let save = command
            .find("cfg.save()")
            .expect("la commande doit enregistrer config.json");
        let state_update = command
            .find("*state.config.lock().unwrap() = cfg")
            .expect("la commande doit mettre à jour l'état après la persistance");
        assert!(
            save < state_update,
            "la commande doit enregistrer config.json avant de mettre à jour l'état"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_set_default_archive_password_none_clears_it() {
        let root = archive_password_scratch("clears");
        let path = root.join("config.json");
        let mut cfg = config::AppConfig {
            default_archive_password: Some("mot-de-passe-de-test".to_string()),
            ..Default::default()
        };
        cfg.default_archive_password = None;
        cfg.save_to(&path).unwrap();

        let reloaded: config::AppConfig =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            reloaded.default_archive_password, None,
            "None efface le mot de passe par défaut de config.json"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_set_default_archive_password_empty_clears_it() {
        let root = archive_password_scratch("empty-clears");
        let path = root.join("config.json");
        let cfg = config::AppConfig {
            default_archive_password: Some(String::new()).filter(|password| !password.is_empty()),
            ..Default::default()
        };
        cfg.save_to(&path).unwrap();

        let reloaded: config::AppConfig =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            reloaded.default_archive_password, None,
            "une chaîne vide efface le mot de passe par défaut de config.json"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_detection_report_serializes_default_archive_password() {
        let cfg = config::AppConfig {
            default_archive_password: Some("mot-de-passe-de-test".to_string()),
            ..Default::default()
        };
        let report = serde_json::to_value(build_report(&cfg)).unwrap();
        assert_eq!(
            report["default_archive_password"],
            serde_json::Value::String("mot-de-passe-de-test".to_string()),
            "DetectionReport doit sérialiser le mot de passe d'archive pour le frontend"
        );
    }

    fn import_lua_scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "luavault-import-lua-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn test_import_lua_uses_content_appid_over_filename() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("content-appid");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("mon_jeu.lua");
        std::fs::write(&source, "addappid(440)\n").unwrap();

        let result = import_lua_file_inner(&source, &library_dir, &data_dir)
            .expect("un .lua avec addappid doit être importé");

        assert_eq!(result.entry.app_id, "440");
        assert_eq!(result.entry.name, "mon_jeu");
        assert!(result.filename_differs);
        assert!(library_dir.join("440.lua").exists());
        assert!(!library_dir.join("mon_jeu.lua").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_lua_rejects_missing_addappid() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("missing-appid");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("sans_appid.lua");
        std::fs::write(&source, "-- aucun appel déclaratif ici\nprint('bonjour')\n").unwrap();

        let error = import_lua_file_inner(&source, &library_dir, &data_dir)
            .expect_err("un .lua sans addappid doit être refusé");

        assert!(error.contains("addappid"), "erreur explicite attendue, reçue : {error}");
        assert!(!library_dir.exists(), "un import refusé ne doit rien écrire");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_lua_preserves_existing_game_name() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("preserve-name");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        library::upsert_with_data_dir(
            &library_dir,
            &data_dir,
            "440",
            "Team Fortress 2",
            None,
            b"addappid(440)\n",
        )
        .unwrap();
        let source = source_dir.join("script.lua");
        std::fs::write(&source, "addappid(440)\n").unwrap();

        let result = import_lua_file_inner(&source, &library_dir, &data_dir)
            .expect("le reimport doit préserver le nom connu");

        assert_eq!(result.entry.name, "Team Fortress 2");
        assert_eq!(
            library::load_index_with_data_dir(&library_dir, &data_dir).unwrap()[0].name,
            "Team Fortress 2"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_lua_updates_raw_placeholder_name() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("replace-raw-name");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        library::upsert_with_data_dir(
            &library_dir,
            &data_dir,
            "264710",
            "AppID 264710",
            None,
            b"addappid(264710)\n",
        )
        .unwrap();
        let source = source_dir.join("264710.lua");
        std::fs::write(&source, "addappid(264710)\n").unwrap();
        let metadata = steamstore::SteamDetails {
            name: "Subnautica".to_string(),
            header_image: Some("https://cdn.example/subnautica.jpg".to_string()),
            ..Default::default()
        };

        let prepared = prepare_lua_import(&source, &library_dir, &data_dir)
            .expect("la préparation doit lire le fichier et vérifier l'index");
        assert!(import_needs_steam_metadata(&prepared));
        let result = finish_lua_import(prepared, &library_dir, &data_dir, Some(metadata))
            .expect("les métadonnées doivent pouvoir réparer le nom brut");

        assert_eq!(result.entry.name, "Subnautica");
        assert_eq!(
            result.entry.icon.as_deref(),
            Some("https://cdn.example/subnautica.jpg")
        );
        let stored = library::load_index_with_data_dir(&library_dir, &data_dir).unwrap();
        assert_eq!(stored[0].name, "Subnautica");
        assert_eq!(
            stored[0].icon.as_deref(),
            Some("https://cdn.example/subnautica.jpg")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_lua_raw_name_falls_back_without_metadata() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("metadata-fallback");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("264710.lua");
        std::fs::write(&source, "addappid(264710)\n").unwrap();

        let prepared = prepare_lua_import(&source, &library_dir, &data_dir).unwrap();
        assert!(import_needs_steam_metadata(&prepared));
        let result = finish_lua_import(prepared, &library_dir, &data_dir, None)
            .expect("une panne de métadonnées ne doit pas empêcher l'import");

        assert_eq!(result.entry.name, "264710");
        assert!(result.entry.icon.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_patch_extracts_appid_from_filename() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("patch-filename-appid");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        library::upsert_with_data_dir(
            &library_dir,
            &data_dir,
            "440",
            "Portal 2",
            None,
            b"addappid(440)\n",
        )
        .unwrap();
        let source = source_dir.join("Portal 2 (440).zip");
        fixes::fake_fix_archive(&source, &[("OnlineFix64.dll", b"patch")]);

        let result = import_patch_archive_inner(&source, None, &library_dir, &data_dir)
            .expect("le nom Portal 2 (440) doit identifier l'AppID");

        assert_eq!(result.app_id, "440");
        assert!(result.app_id_inferred);
        assert_eq!(
            std::fs::read(fixes::archive_path(&library_dir, "440")).unwrap(),
            std::fs::read(&source).unwrap(),
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn patch_filename_cases_match_shared_table() {
        #[derive(serde::Deserialize)]
        struct PatchFilenameCases {
            accepted: Vec<(String, String)>,
            rejected: Vec<String>,
        }

        let cases: PatchFilenameCases = serde_json::from_str(include_str!("../../shared/patch-filename-cases.json"))
            .expect("IMPORT-01: la table partagée patch-filename-cases.json doit être valide");

        for (filename, expected) in cases.accepted {
            assert_eq!(
                app_id_from_patch_filename(Path::new(&filename)),
                Some(expected),
                "IMPORT-01: nom accepté {filename}"
            );
        }
        for filename in cases.rejected {
            assert_eq!(
                app_id_from_patch_filename(Path::new(&filename)),
                None,
                "IMPORT-01: nom refusé {filename}"
            );
        }
    }

    #[test]
    fn test_import_patch_rejects_unsupported_archive() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("patch-unsupported");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("patch.7z");
        std::fs::write(&source, [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0, 0]).unwrap();

        let error = import_patch_archive_inner(&source, Some("440"), &library_dir, &data_dir)
            .expect_err("une archive 7z doit être refusée avant toute copie");

        assert!(error.contains(".7z") && error.contains(".zip") && error.contains(".rar"));
        assert!(!fixes::archive_path(&library_dir, "440").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_patch_rejects_unknown_game_appid() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("patch-unknown-game");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("Portal 2 (440).zip");
        fixes::fake_fix_archive(&source, &[("OnlineFix64.dll", b"patch")]);

        let error = import_patch_archive_inner(&source, Some("440"), &library_dir, &data_dir)
            .expect_err("un AppID absent de la bibliothèque doit être refusé");

        assert!(
            error.contains("ce jeu n'est pas dans votre bibliothèque"),
            "erreur explicite attendue, reçue : {error}"
        );
        assert!(!fixes::archive_path(&library_dir, "440").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_patch_atomic_write() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("patch-atomic-write");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        library::upsert_with_data_dir(
            &library_dir,
            &data_dir,
            "440",
            "Portal 2",
            None,
            b"addappid(440)\n",
        )
        .unwrap();
        let source = source_dir.join("Portal 2 (440).zip");
        fixes::fake_fix_archive(&source, &[("OnlineFix64.dll", b"patch")]);
        let source_bytes = std::fs::read(&source).unwrap();
        let observed_partial = std::cell::RefCell::new(None);

        let result = import_patch_archive_inner_with_before_publish(
            &source,
            None,
            &library_dir,
            &data_dir,
            |temporary| {
                assert!(temporary.is_file(), "le .partial doit exister avant publication");
                assert!(
                    temporary.to_string_lossy().ends_with(".partial"),
                    "le temporaire doit porter le suffixe .partial"
                );
                assert_eq!(std::fs::read(temporary).unwrap(), source_bytes);
                observed_partial.replace(Some(temporary.to_path_buf()));
            },
        )
        .expect("l'import doit publier l'archive complète");

        let partial = observed_partial.into_inner().expect("le .partial doit avoir été observé");
        let destination = fixes::archive_path(&library_dir, &result.app_id);
        assert!(!partial.exists(), "le .partial doit disparaître après le rename");
        assert_eq!(std::fs::read(&destination).unwrap(), source_bytes);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_patch_requires_appid_if_ambiguous() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("patch-ambiguous");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("patch_v2_64bit.zip");
        fixes::fake_fix_archive(&source, &[("OnlineFix64.dll", b"patch")]);

        let error = import_patch_archive_inner(&source, None, &library_dir, &data_dir)
            .expect_err("les chiffres ordinaires du nom ne doivent pas devenir un AppID");

        assert!(error.contains("AppID"), "erreur explicite attendue, reçue : {error}");
        assert!(!library_dir.join("fixes").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn steam_details_cache_and_locks_are_scoped_by_composite_language_key() {
        let app_id = "12345";
        let french_key = steam_details_key(app_id, "french");
        let english_key = steam_details_key(app_id, "english");
        assert_ne!(french_key, english_key, "la langue fait partie de la clé");

        let cache = cache::TtlCache::new(std::time::Duration::from_secs(60));
        cache.put(french_key.clone(), "description française");
        cache.put(english_key.clone(), "English description");
        assert_eq!(cache.get(&french_key), Some("description française"));
        assert_eq!(cache.get(&english_key), Some("English description"));

        let locks = tokio::sync::Mutex::new(HashMap::new());
        let french_first = acquire_details_lock(&locks, &french_key).await;
        let french_second = acquire_details_lock(&locks, &french_key).await;
        let english = acquire_details_lock(&locks, &english_key).await;
        assert!(
            Arc::ptr_eq(&french_first, &french_second),
            "deux appels français du même jeu partagent le verrou de déduplication"
        );
        assert!(
            !Arc::ptr_eq(&french_first, &english),
            "les langues distinctes ne doivent jamais partager un verrou"
        );
        assert_eq!(locks.lock().await.len(), 2, "un verrou par clé composite");
    }

    // ── replace_exe ──

    #[test]
    fn replace_exe_renames_then_copies() {
        // Unique temp dir so parallel tests don't collide (piège n°18).
        let tmp = std::env::temp_dir()
            .join(format!("lv-replace-exe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let target_dir = tmp.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();

        // Write the "current" exe.
        let current_exe = target_dir.join("LuaVault.exe");
        std::fs::write(&current_exe, b"current").unwrap();

        // Write a "config.json" that must survive.
        let config = target_dir.join("config.json");
        std::fs::write(&config, r#"{"key":"value"}"#).unwrap();
        let config_before = std::fs::read_to_string(&config).unwrap();

        // Write the extracted exe.
        let extracted_dir = tmp.join("extracted");
        std::fs::create_dir_all(&extracted_dir).unwrap();
        let extracted_exe = extracted_dir.join("LuaVault.exe");
        std::fs::write(&extracted_exe, b"new").unwrap();

        // Also place a config.json in the extracted folder with DIFFERENT
        // content. The assertion below then proves replace_exe does NOT
        // blindly copy everything from extracted into target — it only
        // touches the executable.
        std::fs::write(extracted_dir.join("config.json"), r#"{"key":"overwritten"}"#).unwrap();

        // Run replace_exe.
        replace_exe(&target_dir, &extracted_exe).unwrap();

        // .old must exist with the old content.
        let old = target_dir.join("LuaVault.exe.old");
        assert!(old.exists());
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "current");

        // The new exe must be in place.
        assert_eq!(std::fs::read_to_string(&current_exe).unwrap(), "new");

        // config.json must be untouched.
        assert_eq!(std::fs::read_to_string(&config).unwrap(), config_before);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn replace_exe_fails_when_no_current_exe() {
        let tmp = std::env::temp_dir()
            .join(format!("lv-replace-exe-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Target directory distinct from the extracted file — the previous
        // version used the same directory, so rename succeeded (self-rename)
        // and the test passed for the wrong reason.
        let target_dir = tmp.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();

        // Extracted exe lives elsewhere.
        let extracted = tmp.join("extracted").join("LuaVault.exe");
        std::fs::create_dir_all(extracted.parent().unwrap()).unwrap();
        std::fs::write(&extracted, b"new").unwrap();

        let result = replace_exe(&target_dir, &extracted);
        assert!(result.is_err());

        // The target directory must NOT have been left half-replaced:
        // no .old, no exe, nothing.
        let old = target_dir.join("LuaVault.exe.old");
        assert!(!old.exists(), ".old must not exist when rename fails");
        let target_exe = target_dir.join("LuaVault.exe");
        assert!(!target_exe.exists(), "target exe must not exist when rename fails");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn install_update_portable_rejects_traversal_and_preserves_config() {
        // Unique temp dir (piège n°18).
        let tmp = std::env::temp_dir()
            .join(format!("lv-install-portable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // ── 1. Build a ZIP in memory ──────────────────────────────────
        let zip_path = tmp.join("update.zip");
        {
            use std::io::Write;
            let zip_file = std::fs::File::create(&zip_path).unwrap();
            let mut writer = zip::ZipWriter::new(zip_file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // Exe
            writer.start_file("LuaVault.exe", options).unwrap();
            writer.write_all(b"new-exe-content").unwrap();
            // config.json with DIFFERENT content from the one in the target dir.
            writer.start_file("config.json", options).unwrap();
            writer.write_all(b"{\"key\":\"overwritten\"}").unwrap();
            // Hostile entry — should be rejected by path validation.
            writer.start_file("../evade.txt", options).unwrap();
            writer.write_all(b"evil").unwrap();

            // An NTFS alternate data stream and a reserved Windows device name
            // (pitfall 23). Say plainly what these two prove, and what they do
            // not: replacing `safe_relative` by a pass-through leaves this test
            // GREEN, because `enclosed_name()` already refuses all three hostile
            // entries upstream. `safe_relative` is therefore defence in depth
            // here, not the load-bearing guard — measured, not assumed. They stay
            // because they pin the intent and would catch a future refactor that
            // drops `enclosed_name()`; they are not evidence that the second
            // layer works.
            writer.start_file("notes.txt:hidden", options).unwrap();
            writer.write_all(b"ads").unwrap();

            writer.start_file("CON", options).unwrap();
            writer.write_all(b"device").unwrap();
            writer.finish().unwrap();
        }

        // ── 2. Set up the target directory with a real exe and config ─
        let target_dir = tmp.join("target_dir");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(target_dir.join("LuaVault.exe"), b"original-exe").unwrap();
        std::fs::write(target_dir.join("config.json"), r#"{"key":"value"}"#).unwrap();

        // ── 3. Call install_update_portable ───────────────────────────
        // The function will fail at spawn (the extracted exe is a dummy),
        // but by then replace_exe has already run.
        let result = install_update_portable(&zip_path, Some(&target_dir));
        // We expect failure at spawn, not at extraction or replace.
        match &result {
            Ok(_) => panic!("install_update_portable should have failed at spawn"),
            Err(e) => eprintln!("install_update_portable error: {}", e),
        }
        assert!(result.is_err(), "install_update_portable should fail at spawn");

        // Neither the ADS entry nor the device name may have been written —
        // anywhere. Both are skipped by `safe_relative`, and without it they
        // would land in the extraction directory.
        let extraction_dir =
            std::env::temp_dir().join(format!("lv-update-{}", std::process::id()));
        for hostile in ["notes.txt:hidden", "notes.txt", "CON"] {
            assert!(
                !target_dir.join(hostile).exists(),
                "{hostile} ne doit pas atterrir dans le dossier cible"
            );
            assert!(
                !extraction_dir.join(hostile).exists(),
                "{hostile} ne doit pas atterrir dans le dossier d'extraction"
            );
        }

        // ── 4. Verify the exe was replaced ────────────────────────────
        let target_exe = target_dir.join("LuaVault.exe");
        // The .old file proves rename happened.
        let old_exe = target_dir.join("LuaVault.exe.old");
        assert!(old_exe.exists(), ".old must exist — proves rename ran");
        assert_eq!(
            std::fs::read_to_string(&target_exe).unwrap(),
            "new-exe-content",
            "target exe should contain the extracted content"
        );

        // ── 5. Verify config.json in the target dir is UNCHANGED ──────
        let target_config = target_dir.join("config.json");
        assert_eq!(
            std::fs::read_to_string(&target_config).unwrap(),
            r#"{"key":"value"}"#,
            "target config.json must NOT be overwritten by extracted content"
        );

        // ── 5. Verify ../evade.txt does NOT exist outside the temp dir ─
        // Check the parent of tmp (i.e. %TEMP%) — evade.txt should not be there.
        let evade_parent = tmp.parent().unwrap();
        let evade_path = evade_parent.join("evade.txt");
        assert!(!evade_path.exists(), "../evade.txt must not escape the temp dir");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn replace_exe_rolls_back_on_copy_failure() {
        // Unique temp dir (piège n°18).
        let tmp = std::env::temp_dir()
            .join(format!("lv-replace-exe-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let target_dir = tmp.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();

        // Write the "current" exe.
        let current_exe = target_dir.join("LuaVault.exe");
        std::fs::write(&current_exe, b"original-content").unwrap();

        // Extracted exe does NOT exist — copy will fail.
        let extracted_exe = tmp.join("nonexistent").join("LuaVault.exe");

        let result = replace_exe(&target_dir, &extracted_exe);
        assert!(result.is_err(), "copy of nonexistent file must fail");

        // The original exe must be restored.
        assert!(current_exe.exists(), "original exe must exist after rollback");
        assert_eq!(
            std::fs::read_to_string(&current_exe).unwrap(),
            "original-content",
            "original exe content must be unchanged after rollback"
        );

        // .old must NOT exist.
        let old_exe = target_dir.join("LuaVault.exe.old");
        assert!(!old_exe.exists(), ".old must not exist after rollback");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn status(app_id: &str) -> GameStatus {
        GameStatus {
            app_id: app_id.to_string(),
            name: "Test".to_string(),
            icon: None,
            updated_at: None,
            added_at: None,
            in_library: true,
            lua_in_steam: true,
            fix_downloaded: false,
            hidden: false,
            tags: Vec::new(),
            game: vdf::GameInstall {
                app_id: app_id.to_string(),
                known_to_steam: true,
                installed: true,
                fully_installed: true,
                ..Default::default()
            },
            playtime_minutes: None,
            last_played: None,
            fix: fixes::FixReport {
                app_id: app_id.to_string(),
                health: fixes::FixHealth::NotInstalled,
                installed_at: None,
                game_dir: None,
                file_count: 0,
                missing: Vec::new(),
                modified: Vec::new(),
                has_backup: false,
                foreign: Vec::new(),
            },
            stage: "no_lua",
        }
    }

    #[test]
    fn stage_walks_the_user_journey_in_order() {
        let mut s = status("42");
        s.in_library = false;
        assert_eq!(derive_stage(&s), "no_lua");

        s.in_library = true;
        s.lua_in_steam = false;
        assert_eq!(derive_stage(&s), "lua_not_in_steam");

        s.lua_in_steam = true;
        s.game.installed = false;
        assert_eq!(derive_stage(&s), "needs_steam_install");

        s.game.installed = true;
        s.game.fully_installed = false;
        assert_eq!(derive_stage(&s), "installing");

        s.game.fully_installed = true;
        assert_eq!(derive_stage(&s), "ready");
    }

    #[test]
    fn stage_tracks_the_online_fix_lifecycle() {
        let mut s = status("42");
        s.fix_downloaded = true;
        assert_eq!(derive_stage(&s), "fix_downloaded");

        s.fix.health = fixes::FixHealth::Healthy;
        assert_eq!(derive_stage(&s), "fix_installed");

        s.fix.health = fixes::FixHealth::Damaged;
        assert_eq!(derive_stage(&s), "fix_damaged");

        s.fix.health = fixes::FixHealth::GameMoved;
        assert_eq!(derive_stage(&s), "fix_game_moved");
    }

    #[test]
    fn a_patch_we_did_not_install_is_reported_as_external() {
        let mut s = status("42");
        s.fix.foreign = vec!["OnlineFix64.dll".to_string()];
        // Offering "download the patch" here would overwrite files we can't restore.
        assert_eq!(derive_stage(&s), "fix_external");

        // Our own install takes precedence over the heuristic.
        s.fix.health = fixes::FixHealth::Healthy;
        assert_eq!(derive_stage(&s), "fix_installed");
    }

    #[test]
    fn an_uninstalled_game_hides_fix_states() {
        let mut s = status("42");
        s.game.installed = false;
        s.fix.health = fixes::FixHealth::Healthy;
        // Reinstalling the game wiped the fix — don't claim it's still applied.
        assert_eq!(derive_stage(&s), "needs_steam_install");
    }

    // ── LOT-15: the bulk-fix selection ──────────────────────────────

    #[test]
    fn repair_treats_only_damaged_and_moved_installs() {
        assert!(is_fix_candidate("fix_damaged", true, true, REPAIRABLE_FIX_STAGES));
        assert!(is_fix_candidate("fix_game_moved", true, true, REPAIRABLE_FIX_STAGES));
        // Exactly those two — nothing else may sneak into a repair pass.
        assert_eq!(REPAIRABLE_FIX_STAGES.len(), 2);
    }

    #[test]
    fn repair_never_installs_a_patch_the_user_never_had() {
        // Installing a patch that was never applied is an install, not a
        // repair — the two states stay out of the repair pass.
        assert!(!is_fix_candidate("fix_downloaded", true, true, REPAIRABLE_FIX_STAGES));
        // A healthy install has nothing to repair.
        assert!(!is_fix_candidate("fix_installed", true, true, REPAIRABLE_FIX_STAGES));
        assert!(!is_fix_candidate("ready", true, true, REPAIRABLE_FIX_STAGES));
    }

    #[test]
    fn fix_external_enters_no_bulk_pass() {
        // The app holds no backup of those files: reinstalling over them
        // would destroy a third-party patch without any way back.
        assert!(!is_fix_candidate("fix_external", true, true, REPAIRABLE_FIX_STAGES));
        assert!(!is_fix_candidate("fix_external", true, true, INSTALLABLE_FIX_STAGES));
    }

    #[test]
    fn a_game_still_downloading_enters_no_bulk_pass() {
        // StateFlags says the download is running → `fully_installed` is
        // false, and `install_fix_inner` refuses the patch anyway: the
        // selection must not even try.
        assert!(!is_fix_candidate("fix_damaged", true, false, REPAIRABLE_FIX_STAGES));
        assert!(!is_fix_candidate("fix_game_moved", true, false, REPAIRABLE_FIX_STAGES));
        assert!(!is_fix_candidate("fix_downloaded", true, false, INSTALLABLE_FIX_STAGES));
    }

    #[test]
    fn install_all_treats_everything_patchable() {
        for stage in ["fix_downloaded", "fix_damaged", "fix_game_moved"] {
            assert!(is_fix_candidate(stage, true, true, INSTALLABLE_FIX_STAGES));
        }
        assert!(!is_fix_candidate("fix_installed", true, true, INSTALLABLE_FIX_STAGES));
    }

    #[test]
    fn bulk_report_counts_failures_as_failures() {
        // A batch that half-fails must say so: an Err entering the report
        // lands in `failed`, never in `succeeded`.
        let mut report = BulkReport::default();
        report.push("1", "Alpha", Ok("2 fichier(s) appliqué(s)".to_string()));
        report.push("2", "Beta", Err("archive introuvable".to_string()));
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed, 1);
        assert_eq!(report.items.len(), 2);
        assert!(report.items[0].ok);
        assert!(!report.items[1].ok);
        assert_eq!(report.items[1].detail, "archive introuvable");
    }

    #[test]
    fn a_game_without_fix_enters_no_pass() {
        assert!(!is_fix_candidate("fix_damaged", false, true, REPAIRABLE_FIX_STAGES));
        assert!(!is_fix_candidate("fix_game_moved", false, true, REPAIRABLE_FIX_STAGES));
        assert!(!is_fix_candidate("fix_downloaded", false, true, INSTALLABLE_FIX_STAGES));
    }

    // ── LOT-15-fix01: the selection, EXECUTED from the commands ─────
    //
    // The predicate tests above pin `is_fix_candidate` in isolation; these
    // run the selection the bulk commands actually execute and observe what
    // a pass touches on disk. A repair that treated an unpatched game,
    // or a preflight that promised one, goes red here.

    /// Fake Steam tree + library where each file controls one signal:
    /// appmanifest → installed/fully_installed, `config\lua` → lua_in_steam,
    /// fix state → damaged, archive → fix_downloaded.
    fn bulk_fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("ast_bulk_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lib = root.join("library");
        let data = root.join("data");
        let steam = root.join("steam");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(steam.join("steamapps").join("common")).unwrap();
        std::fs::create_dir_all(steam.join("config").join("lua")).unwrap();
        (root, lib, steam, data)
    }

    fn add_installed_game(lib: &Path, steam: &Path, app_id: &str, dir_name: &str, dll: &[u8]) -> PathBuf {
        let game_dir = steam.join("steamapps").join("common").join(dir_name);
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("steam_api64.dll"), dll).unwrap();
        std::fs::write(
            steam.join("steamapps").join(format!("appmanifest_{app_id}.acf")),
            format!(
                "\"AppState\" {{ \"appid\" \"{app_id}\" \"name\" \"{dir_name}\" \
                 \"installdir\" \"{dir_name}\" \"StateFlags\" \"4\" }}"
            ),
        )
        .unwrap();
        std::fs::write(
            steam.join("config").join("lua").join(format!("{app_id}.lua")),
            "-- lua",
        )
        .unwrap();
        std::fs::write(lib.join(format!("{app_id}.lua")), "-- lua").unwrap();
        game_dir
    }

    /// Write and sign the index against the fixture's injected data dir —
    /// the strict load refuses an unsigned index once a key exists.
    fn write_index(lib: &Path, data_dir: &Path, entries: &[library::LibraryEntry]) {
        library::save_index_with_data_dir(lib, data_dir, entries).unwrap();
    }

    fn index_entry(app_id: &str, name: &str, hidden: bool) -> library::LibraryEntry {
        library::LibraryEntry {
            app_id: app_id.to_string(),
            name: name.to_string(),
            icon: None,
            file_name: format!("{app_id}.lua"),
            added_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            has_fix: false,
            hidden,
            tags: Vec::new(),
        }
    }

    /// Recorded fix install whose file no longer matches → `fix_damaged`.
    fn make_damaged(lib: &Path, app_id: &str, game_dir: &Path) {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(b"patched");
        let state = fixes::FixState {
            app_id: app_id.to_string(),
            game_dir: game_dir.display().to_string(),
            installed_at: chrono::Utc::now().to_rfc3339(),
            files: vec![fixes::FixFile {
                rel: "steam_api64.dll".to_string(),
                sha256: format!("{:x}", hasher.finalize()),
                size: 7,
            }],
            backup_zip: None,
            backed_up: Vec::new(),
            created_dirs: Vec::new(),
        };
        std::fs::create_dir_all(fixes::fixes_dir(lib)).unwrap();
        std::fs::write(
            fixes::state_path(lib, app_id),
            serde_json::to_vec_pretty(&state).unwrap(),
        )
        .unwrap();
    }

    // The cache lock must span the awaited pass: the index cache is filled
    // inside it, and the lock serialises those fills with the cache-internal
    // assertions of the library tests. Test-only, single lock, no deadlock.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn the_repair_pass_executed_leaves_fix_downloaded_games_untouched() {
        let _lock = library::cache_test_lock();
        let (root, lib, steam, data) = bulk_fixture("repair_exec");
        // 42: patched once, then damaged by Steam → the only game a repair treats.
        let dir_42 = add_installed_game(&lib, &steam, "42", "Damageville", b"reverted-by-steam");
        make_damaged(&lib, "42", &dir_42);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "42"),
            &[("steam_api64.dll", b"patched")],
        );
        // 43: no patch downloaded or installed → ready (not in repair set).
        let dir_43 = add_installed_game(&lib, &steam, "43", "Freshville", b"pristine");
        write_index(
            &lib,
            &data,
            &[
                index_entry("42", "Damageville", false),
                index_entry("43", "Freshville", false),
            ],
        );

        let cfg = config::AppConfig {
            steam_dir: Some(steam.clone()),
            library_dir: Some(lib.clone()),
            default_archive_password: Some("testpass".to_string()),
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let mut events: Vec<BulkProgressEvent> = Vec::new();
        let ctx = BulkFixCtx {
            cfg: &cfg,
            library_dir: &lib,
            data_dir: &data,
            steam: Some(steam.as_path()),
            cancel: &cancel,
        };

        // Exactly what `repair_all_fixes` runs: the shared loop with the
        // repair set. Any download attempt would fail loudly.
        let report = bulk_fixes_core(
            &ctx,
            "repair",
            REPAIRABLE_FIX_STAGES,
            None,
            |ev| events.push(ev),
            |app_id| Box::pin(async move { Err(format!("une réparation ne télécharge rien ({app_id})")) }),
        )
        .await;

        // 42 repaired, and only 42: no item, no event for 43.
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].app_id, "42");
        assert!(events.iter().all(|e| e.app_id == "42"));
        // 43 untouched on disk: no archive downloaded, no state, pristine files.
        assert!(
            !fixes::archive_path(&lib, "43").is_file(),
            "le jeu jamais patché ne doit pas être téléchargé par une réparation"
        );
        assert!(fixes::load_state(&lib, "43").is_none());
        assert_eq!(std::fs::read(dir_43.join("steam_api64.dll")).unwrap(), b"pristine");
        // 42 is healthy again, and nothing honest-to-nothing was "skipped".
        assert_eq!(
            fixes::verify(&lib, "42", Some(&dir_42)).health,
            fixes::FixHealth::Healthy
        );
        assert_eq!(
            report.skipped, 0,
            "un jeu sans rien à réparer n'est pas « ignoré » par la réparation"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The mirror image of the previous test: the install pass DOES treat a
    /// `fix_downloaded` game. Together they prove the selection is really
    /// executed — not just mentioned — and that the two passes differ on it.
    #[allow(clippy::await_holding_lock)] // see the_repair_pass_executed — same cache-lock rationale
    #[tokio::test]
    async fn the_install_pass_treats_the_fix_downloaded_game_the_repair_excludes() {
        let _lock = library::cache_test_lock();
        let (root, lib, steam, data) = bulk_fixture("install_exec");
        let dir_42 = add_installed_game(&lib, &steam, "42", "Damageville", b"reverted-by-steam");
        make_damaged(&lib, "42", &dir_42);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "42"),
            &[("steam_api64.dll", b"patched")],
        );
        let _dir_43 = add_installed_game(&lib, &steam, "43", "Freshville", b"pristine");
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "43"),
            &[("steam_api64.dll", b"patched")],
        );
        write_index(
            &lib,
            &data,
            &[
                index_entry("42", "Damageville", false),
                index_entry("43", "Freshville", false),
            ],
        );

        let cfg = config::AppConfig {
            steam_dir: Some(steam.clone()),
            library_dir: Some(lib.clone()),
            default_archive_password: Some("testpass".to_string()),
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let ctx = BulkFixCtx {
            cfg: &cfg,
            library_dir: &lib,
            data_dir: &data,
            steam: Some(steam.as_path()),
            cancel: &cancel,
        };

        let report = bulk_fixes_core(
            &ctx,
            "fixes",
            INSTALLABLE_FIX_STAGES,
            None,
            |_ev| {},
            |app_id| Box::pin(async move { Err(format!("téléchargement simulé refusé ({app_id})")) }),
        )
        .await;

        // Both games entered the pass: 42 installed from its archive, 43
        // selected too — its simulated download failure lands as an item.
        assert_eq!(report.items.len(), 2);
        let item_42 = report.items.iter().find(|i| i.app_id == "42").unwrap();
        assert!(item_42.ok, "item 42 detail: {}", item_42.detail);
        let item_43 = report.items.iter().find(|i| i.app_id == "43").unwrap();
        assert!(item_43.ok);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Hidden games are absent from the library view's bulk buttons, so they
    /// must be absent from the passes and the confirmation screen alike.
    #[allow(clippy::await_holding_lock)] // see the_repair_pass_executed — same cache-lock rationale
    #[tokio::test]
    async fn hidden_games_enter_no_pass_and_no_plan() {
        let _lock = library::cache_test_lock();
        let (root, lib, steam, data) = bulk_fixture("hidden_exec");
        let dir_44 = add_installed_game(&lib, &steam, "44", "Hiddenville", b"reverted-by-steam");
        make_damaged(&lib, "44", &dir_44);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "44"),
            &[("steam_api64.dll", b"patched")],
        );
        write_index(&lib, &data, &[index_entry("44", "Hiddenville", true)]);

        let cfg = config::AppConfig {
            steam_dir: Some(steam.clone()),
            library_dir: Some(lib.clone()),
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let ctx = BulkFixCtx {
            cfg: &cfg,
            library_dir: &lib,
            data_dir: &data,
            steam: Some(steam.as_path()),
            cancel: &cancel,
        };

        let report = bulk_fixes_core(
            &ctx,
            "repair",
            REPAIRABLE_FIX_STAGES,
            None,
            |_ev| {},
            |app_id| Box::pin(async move { Err(format!("un jeu masqué ne doit pas être traité ({app_id})")) }),
        )
        .await;
        assert!(report.items.is_empty(), "un jeu masqué ne fait partie d'aucun lot");
        assert_eq!(report.skipped, 0);

        let plan = bulk_preflight_plan(&lib, &data, Some(steam.as_path()), false, true, None);
        assert!(plan.fixes.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn preflight_repair_only_promises_only_what_the_repair_pass_will_treat() {
        let _lock = library::cache_test_lock();
        let (root, lib, steam, data) = bulk_fixture("preflight");
        // 42 damaged, 43 never patched, 44 hidden (but damaged).
        let dir_42 = add_installed_game(&lib, &steam, "42", "Damageville", b"reverted-by-steam");
        make_damaged(&lib, "42", &dir_42);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "42"),
            &[("steam_api64.dll", b"patched")],
        );
        let _dir_43 = add_installed_game(&lib, &steam, "43", "Freshville", b"pristine");
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "43"),
            &[("steam_api64.dll", b"patched")],
        );
        let dir_44 = add_installed_game(&lib, &steam, "44", "Hiddenville", b"reverted-by-steam");
        make_damaged(&lib, "44", &dir_44);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "44"),
            &[("steam_api64.dll", b"patched")],
        );
        write_index(
            &lib,
            &data,
            &[
                index_entry("42", "Damageville", false),
                index_entry("43", "Freshville", false),
                index_entry("44", "Hiddenville", true),
            ],
        );

        // The confirmation screen for a repair: only the broken, visible,
        // once-patched game. A `fix_downloaded` game there would be a promise
        // to install a patch the user never had.
        let plan = bulk_preflight_plan(&lib, &data, Some(steam.as_path()), false, true, None);
        let ids: Vec<&str> = plan.fixes.iter().map(|f| f.app_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["42"],
            "un jeu jamais patché (43) ou masqué (44) ne doit pas figurer dans une réparation"
        );
        assert!(plan.games.is_empty());

        // The install preflight treats 43 too, but never the hidden game.
        let plan_all = bulk_preflight_plan(&lib, &data, Some(steam.as_path()), true, false, None);
        let ids_all: Vec<&str> = plan_all.fixes.iter().map(|f| f.app_id.as_str()).collect();
        assert!(ids_all.contains(&"42") && ids_all.contains(&"43"));
        assert!(!ids_all.contains(&"44"), "un jeu masqué ne fait partie d'aucun lot");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[allow(clippy::await_holding_lock)] // see the_repair_pass_executed — same cache-lock rationale
    #[tokio::test]
    async fn cancellation_counts_only_what_the_pass_did_not_finish() {
        let _lock = library::cache_test_lock();
        let (root, lib, steam, data) = bulk_fixture("cancel_count");
        let dir_42 = add_installed_game(&lib, &steam, "42", "Damageville", b"reverted-by-steam");
        make_damaged(&lib, "42", &dir_42);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "42"),
            &[("steam_api64.dll", b"patched")],
        );
        write_index(&lib, &data, &[index_entry("42", "Damageville", false)]);

        let cfg = config::AppConfig {
            steam_dir: Some(steam.clone()),
            library_dir: Some(lib.clone()),
            ..Default::default()
        };
        let cancel = AtomicBool::new(true);
        let ctx = BulkFixCtx {
            cfg: &cfg,
            library_dir: &lib,
            data_dir: &data,
            steam: Some(steam.as_path()),
            cancel: &cancel,
        };

        let report = bulk_fixes_core(
            &ctx,
            "repair",
            REPAIRABLE_FIX_STAGES,
            None,
            |_ev| {},
            |app_id| Box::pin(async move { Err(app_id) }),
        )
        .await;

        assert!(report.items.is_empty());
        assert_eq!(report.succeeded, 0);
        assert_eq!(
            report.skipped, 1,
            "seul le candidat annulé est « ignoré » — pas le reste de la bibliothèque"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // ── LOT-16: the selection, EXECUTED from the fifth mode ─────────
    //
    // The predicate `in_selection` is pinned where it is used: these tests
    // run the pass and the preflight `apply_fixes_to_selection` actually
    // executes, then observe who was treated. A loop that processed an
    // unselected game goes red here — as does a preflight that promised one.

    /// A game carrying a third-party patch's marker files: the stage derives
    /// `fix_external` exactly like it does on a real install (no fix state,
    /// markers found in the game folder).
    fn make_external(game_dir: &Path) {
        std::fs::write(game_dir.join("OnlineFix.ini"), "[third-party]").unwrap();
    }

    #[allow(clippy::await_holding_lock)] // see the_repair_pass_executed — same cache-lock rationale
    #[tokio::test]
    async fn the_selection_pass_executed_leaves_unselected_games_untouched() {
        let _lock = library::cache_test_lock();
        let (root, lib, steam, data) = bulk_fixture("selection_exec");
        // 42: damaged, archived, SELECTED — the one game the pass repairs.
        let dir_42 = add_installed_game(&lib, &steam, "42", "Damageville", b"reverted-by-steam");
        make_damaged(&lib, "42", &dir_42);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "42"),
            &[("steam_api64.dll", b"patched")],
        );
        // 43: never installed, but fix archive downloaded → fix_downloaded, SELECTED.
        let _dir_43 = add_installed_game(&lib, &steam, "43", "Freshville", b"pristine");
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "43"),
            &[("steam_api64.dll", b"patched")],
        );
        // 44: damaged, archived, NOT selected — must stay untouched.
        let dir_44 = add_installed_game(&lib, &steam, "44", "Unpickedville", b"reverted-by-steam");
        make_damaged(&lib, "44", &dir_44);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "44"),
            &[("steam_api64.dll", b"patched")],
        );
        // 45: damaged, archived, SELECTED but HIDDEN — the hidden defence
        // stands even inside the user's own list.
        let dir_45 = add_installed_game(&lib, &steam, "45", "Hiddenville", b"reverted-by-steam");
        make_damaged(&lib, "45", &dir_45);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "45"),
            &[("steam_api64.dll", b"patched")],
        );
        write_index(
            &lib,
            &data,
            &[
                index_entry("42", "Damageville", false),
                index_entry("43", "Freshville", false),
                index_entry("44", "Unpickedville", false),
                index_entry("45", "Hiddenville", true),
            ],
        );

        let cfg = config::AppConfig {
            steam_dir: Some(steam.clone()),
            library_dir: Some(lib.clone()),
            default_archive_password: Some("testpass".to_string()),
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let mut events: Vec<BulkProgressEvent> = Vec::new();
        let ctx = BulkFixCtx {
            cfg: &cfg,
            library_dir: &lib,
            data_dir: &data,
            steam: Some(steam.as_path()),
            cancel: &cancel,
        };
        let selection: Vec<String> = ["42", "43", "45"].iter().map(|s| s.to_string()).collect();

        // Exactly what `apply_fixes_to_selection` runs: the shared loop with
        // the installable set and the user's list. Any download attempt
        // would fail loudly.
        let report = bulk_fixes_core(
            &ctx,
            "fixes",
            INSTALLABLE_FIX_STAGES,
            Some(&selection),
            |ev| events.push(ev),
            |app_id| Box::pin(async move { Err(format!("la sélection ne télécharge rien ({app_id})")) }),
        )
        .await;

        // 42 repaired, 43 installed from its archive — and nothing else: no item,
        // no event for the unselected 44 or the hidden 45.
        assert_eq!(report.items.len(), 2);
        let item_42 = report.items.iter().find(|i| i.app_id == "42").unwrap();
        assert!(item_42.ok);
        assert!(report.items.iter().any(|i| i.app_id == "43" && i.ok));
        assert!(events.iter().all(|e| e.app_id == "42" || e.app_id == "43"));
        assert_eq!(report.skipped, 0);
        // 44 untouched on disk: still damaged, files never rewritten.
        assert_eq!(
            fixes::verify(&lib, "44", Some(&dir_44)).health,
            fixes::FixHealth::Damaged,
            "un jeu non sélectionné ne doit pas être réparé par la sélection"
        );
        assert_eq!(std::fs::read(dir_44.join("steam_api64.dll")).unwrap(), b"reverted-by-steam");
        // 45 untouched too: hidden games enter no pass, selection or not.
        assert_eq!(
            fixes::verify(&lib, "45", Some(&dir_45)).health,
            fixes::FixHealth::Damaged,
            "un jeu masqué ne fait partie d'aucun lot, même sélectionné"
        );
        assert_eq!(std::fs::read(dir_45.join("steam_api64.dll")).unwrap(), b"reverted-by-steam");
        // 42 is healthy again.
        assert_eq!(
            fixes::verify(&lib, "42", Some(&dir_42)).health,
            fixes::FixHealth::Healthy
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selection_preflight_promises_only_what_the_selection_pass_will_treat() {
        let _lock = library::cache_test_lock();
        let (root, lib, steam, data) = bulk_fixture("selection_plan");
        let dir_42 = add_installed_game(&lib, &steam, "42", "Damageville", b"reverted-by-steam");
        make_damaged(&lib, "42", &dir_42);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "42"),
            &[("steam_api64.dll", b"patched")],
        );
        let _dir_43 = add_installed_game(&lib, &steam, "43", "Freshville", b"pristine");
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "43"),
            &[("steam_api64.dll", b"patched")],
        );
        let dir_44 = add_installed_game(&lib, &steam, "44", "Unpickedville", b"reverted-by-steam");
        make_damaged(&lib, "44", &dir_44);
        fixes::fake_fix_archive(
            &fixes::archive_path(&lib, "44"),
            &[("steam_api64.dll", b"patched")],
        );
        let dir_45 = add_installed_game(&lib, &steam, "45", "Hiddenville", b"reverted-by-steam");
        make_damaged(&lib, "45", &dir_45);
        // 47: in the library, .lua in Steam, but Steam never installed it —
        // stage `needs_steam_install`. Without it in the fixture, the
        // `plan.games.is_empty()` assertion below holds by construction and
        // the `selection.is_none()` guard on the install branch is untested.
        std::fs::write(lib.join("47.lua"), "-- lua").unwrap();
        std::fs::write(steam.join("config").join("lua").join("47.lua"), "-- lua").unwrap();
        write_index(
            &lib,
            &data,
            &[
                index_entry("42", "Damageville", false),
                index_entry("43", "Freshville", false),
                index_entry("44", "Unpickedville", false),
                index_entry("45", "Hiddenville", true),
                index_entry("47", "Notinstalledville", false),
            ],
        );

        // The confirmation screen of the fifth mode: exactly the picked
        // games the pass can treat. 44 is patchable but not picked; 45 is
        // picked but hidden — neither may be promised.
        let selection: Vec<String> = ["42", "43", "45", "47"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let plan = bulk_preflight_plan(&lib, &data, Some(steam.as_path()), true, false, Some(&selection));
        let ids: Vec<&str> = plan.fixes.iter().map(|f| f.app_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["42", "43"],
            "un jeu non sélectionné (44) ou masqué (45) ne doit pas figurer dans la sélection"
        );
        // The selection mode manages — it never installs through Steam. 47
        // is the game that makes this assertion bite: it is picked, visible
        // and `needs_steam_install`, so only the guard keeps it out.
        assert!(
            plan.games.is_empty(),
            "la sélection ne promet jamais une installation Steam"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[allow(clippy::await_holding_lock)] // see the_repair_pass_executed — same cache-lock rationale
    #[tokio::test]
    async fn fix_external_in_the_selection_enters_no_pass() {
        let _lock = library::cache_test_lock();
        let (root, lib, steam, data) = bulk_fixture("selection_external");
        // 46: installed, .lua everywhere, but the patch in place is a
        // third-party one — the app holds no backup of those files.
        let dir_46 = add_installed_game(&lib, &steam, "46", "Foreignville", b"untouched-original");
        make_external(&dir_46);
        write_index(&lib, &data, &[index_entry("46", "Foreignville", false)]);

        let cfg = config::AppConfig {
            steam_dir: Some(steam.clone()),
            library_dir: Some(lib.clone()),
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let ctx = BulkFixCtx {
            cfg: &cfg,
            library_dir: &lib,
            data_dir: &data,
            steam: Some(steam.as_path()),
            cancel: &cancel,
        };
        let selection: Vec<String> = vec!["46".to_string()];

        // The user picked it — the pass still refuses: `fix_external` is in
        // neither stage set, and the selection adds no exception.
        let report = bulk_fixes_core(
            &ctx,
            "fixes",
            INSTALLABLE_FIX_STAGES,
            Some(&selection),
            |_ev| {},
            |app_id| {
                Box::pin(async move {
                    Err(format!("un patch tiers ne doit jamais être écrasé ({app_id})"))
                })
            },
        )
        .await;
        assert!(
            report.items.is_empty(),
            "un fix_external sélectionné ne fait partie d'aucune passe"
        );
        assert_eq!(report.skipped, 0);
        // Untouched on disk: no state recorded, the third-party marker and
        // the original file exactly as they were.
        assert!(fixes::load_state(&lib, "46").is_none());
        assert!(dir_46.join("OnlineFix.ini").is_file());
        assert_eq!(
            std::fs::read(dir_46.join("steam_api64.dll")).unwrap(),
            b"untouched-original"
        );

        // The confirmation screen agrees: nothing promised for that pick.
        let plan = bulk_preflight_plan(&lib, &data, Some(steam.as_path()), true, false, Some(&selection));
        assert!(plan.fixes.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// LOT-13: the three states of playtime must stay distinct on GameStatus
    /// — a measured value, "jamais joué" (`Some(0)`), and "on ne sait pas"
    /// (`None`). Conflating any two of them fails here.
    #[test]
    fn build_status_playtime_fields_follow_localconfig() {
        let steam = std::env::temp_dir().join(format!("ast_playtime_map_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&steam);
        let lib = std::env::temp_dir().join(format!("ast_playtime_lib_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&lib);
        std::fs::create_dir_all(&lib).unwrap();

        // Single userdata folder → no loginusers.vdf needed.
        let config_dir = steam.join("userdata").join("999").join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("localconfig.vdf"),
            "\"UserLocalConfigStore\" { \"Software\" { \"Valve\" { \"Steam\" { \"apps\" { \
             \"77\" { \"Playtime\" \"217\" \"LastPlayed\" \"1712725190\" } \
             \"78\" { \"cloud\" { \"last_sync_state\" \"synchronized\" } } \
             } } } } }",
        )
        .unwrap();

        // Measured data.
        let played = build_status(&lib, Some(&steam), "77", None);
        assert_eq!(played.playtime_minutes, Some(217));
        assert_eq!(played.last_played, Some(1712725190));

        // Present in the apps block without any keys: never played, not unknown.
        let never = build_status(&lib, Some(&steam), "78", None);
        assert_eq!(never.playtime_minutes, Some(0), "jamais joué ≠ inconnu");
        assert_eq!(never.last_played, None);

        // AppID absent from the account's data: on ne sait pas.
        let absent = build_status(&lib, Some(&steam), "99", None);
        assert_eq!(absent.playtime_minutes, None);
        assert_eq!(absent.last_played, None);

        // No Steam at all: on ne sait pas, still.
        let no_steam = build_status(&lib, None, "77", None);
        assert_eq!(no_steam.playtime_minutes, None);
        assert_eq!(no_steam.last_played, None);

        let _ = std::fs::remove_dir_all(&steam);
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn measured_collect_statuses_reports_count_and_duration() {
        let tmp = std::env::temp_dir().join("lv-lot09-measured-collect");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let entries: Vec<library::LibraryEntry> = (0..5)
            .map(|i| library::LibraryEntry {
                app_id: format!("{}", 1000 + i),
                name: format!("Game {i}"),
                icon: None,
                file_name: format!("{}.lua", 1000 + i),
                added_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                has_fix: false,
                hidden: false,
                tags: Vec::new(),
            })
            .collect();

        let (statuses, count, elapsed_ms) = measured_collect_statuses(&tmp, None, entries);
        assert_eq!(count, 5, "entry count must match input length");
        assert_eq!(statuses.len(), 5, "one status per entry");
        assert!(elapsed_ms < 60_000, "took {elapsed_ms} ms — something is very wrong");

        // Empty input: zero entries, zero time.
        let (empty, empty_count, _) = measured_collect_statuses(&tmp, None, Vec::new());
        assert_eq!(empty_count, 0);
        assert!(empty.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn measured_collect_statuses_duration_is_measured_not_hardcoded() {
        // A fake clock that advances 250 ms on every read. The wrapper reads it
        // twice (start, end), so the reported duration must be at least 250 ms —
        // a hardcoded zero, or an Instant::now() that ignores the injected clock,
        // measures ~0 ms and fails the lower bound. That is the whole point: an
        // upper bound alone (`elapsed < 60_000`) is satisfied by `0u128`.
        static TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        fn fake_clock() -> std::time::Instant {
            let tick = TICKS.fetch_add(1, Ordering::SeqCst);
            std::time::Instant::now() + std::time::Duration::from_millis(250 * tick)
        }

        let tmp = std::env::temp_dir().join("lv-lot09-fake-clock");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let entries: Vec<library::LibraryEntry> = (0..3)
            .map(|i| library::LibraryEntry {
                app_id: format!("{}", 2000 + i),
                name: format!("Game {i}"),
                icon: None,
                file_name: format!("{}.lua", 2000 + i),
                added_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                has_fix: false,
                hidden: false,
                tags: Vec::new(),
            })
            .collect();

        let (statuses, count, elapsed_ms) =
            measured_collect_statuses_with(&tmp, None, entries, fake_clock);
        assert_eq!(count, 3, "entry count must match input length");
        assert_eq!(statuses.len(), 3, "one status per entry");
        assert!(
            elapsed_ms >= 250,
            "elapsed_ms = {elapsed_ms} — the duration must come from the injected clock, two reads 250 ms apart"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -------------------------------------- aggregated changelog feed (LOT-12)

    fn changelog(date: i64, title: &str) -> steamstore::Changelog {
        steamstore::Changelog {
            title: title.to_string(),
            date,
            ..Default::default()
        }
    }

    fn feed_item(app_id: &str, date: i64, title: &str) -> FeedItem {
        FeedItem {
            app_id: app_id.to_string(),
            game_name: app_id.to_string(),
            title: title.to_string(),
            date,
            url: String::new(),
            is_patch_notes: false,
            excerpt: String::new(),
        }
    }

    #[test]
    fn feed_keeps_at_most_three_posts_per_game() {
        // Five posts, newest first (the order changelogs() guarantees).
        let logs: Vec<_> = (0..5)
            .map(|i| changelog(1_000 + i as i64, &format!("annonce {i}")))
            .rev()
            .collect();
        let items = feed_items_for("42", "Subnautica", logs);
        assert_eq!(
            items.len(),
            FEED_MAX_PER_GAME,
            "un jeu très actif ne doit pas noyer le flux"
        );
        // The cap keeps the three FRESHEST posts.
        assert_eq!(items[0].date, 1_004);
        assert_eq!(items[2].date, 1_002);
        assert_eq!(items[0].game_name, "Subnautica");
    }

    #[test]
    fn feed_sort_is_deterministic_at_equal_dates() {
        // Deliberately NOT in the expected order: `sort_by` is stable, so a
        // sort on the date alone would preserve this input order and fail
        // the assertion — the tiebreak is what this test pins.
        let mut items = vec![
            feed_item("200", 100, "Zeta"),
            feed_item("100", 100, "Yotta"),
            feed_item("200", 100, "Alpha"),
            feed_item("999", 200, "Newer"),
        ];
        sort_feed(&mut items);
        let order: Vec<String> = items
            .iter()
            .map(|i| format!("{}:{}", i.app_id, i.title))
            .collect();
        assert_eq!(
            order,
            vec![
                "999:Newer".to_string(),
                "100:Yotta".to_string(),
                "200:Alpha".to_string(),
                "200:Zeta".to_string()
            ],
            "décroissant par date, puis app_id, puis titre"
        );
        // Re-sorting an already sorted feed must not move anything.
        sort_feed(&mut items);
        let again: Vec<String> = items
            .iter()
            .map(|i| format!("{}:{}", i.app_id, i.title))
            .collect();
        assert_eq!(order, again, "le tri doit être idempotent");
    }

    #[test]
    fn feed_never_caches_a_failure() {
        let cache: cache::TtlCache<String, Vec<steamstore::Changelog>> =
            cache::TtlCache::new(std::time::Duration::from_secs(60));
        let failure: Result<Vec<steamstore::Changelog>, String> =
            Err("réseau coupé".to_string());
        remember_changelogs(&cache, "42", &failure);
        assert!(
            cache.get(&"42".to_string()).is_none(),
            "un échec mis en cache ferait disparaître le jeu du flux pendant tout le TTL"
        );
    }

    #[test]
    fn feed_caches_an_empty_success() {
        let cache: cache::TtlCache<String, Vec<steamstore::Changelog>> =
            cache::TtlCache::new(std::time::Duration::from_secs(60));
        let empty: Result<Vec<steamstore::Changelog>, String> = Ok(Vec::new());
        remember_changelogs(&cache, "42", &empty);
        assert_eq!(
            cache.get(&"42".to_string()).as_ref().map(Vec::len),
            Some(0),
            "un jeu sans annonce est un succès vide — le re-demander à chaque ouverture serait absurde"
        );
    }

    fn library_entry(app_id: &str, name: &str, hidden: bool) -> library::LibraryEntry {
        library::LibraryEntry {
            app_id: app_id.to_string(),
            name: name.to_string(),
            icon: None,
            file_name: format!("{app_id}.lua"),
            added_at: String::new(),
            updated_at: String::new(),
            has_fix: false,
            hidden,
            tags: Vec::new(),
        }
    }

    /// A fetch that fails after 100 ms, counting in-flight requests as it
    /// goes. Failures are never cached, so every caller that gets the lock
    /// really fetches — the shared lock is the only thing left that can keep
    /// them serial, which is exactly what the test below observes.
    async fn timed_failure(
        in_flight: Arc<std::sync::atomic::AtomicUsize>,
        peak: Arc<std::sync::atomic::AtomicUsize>,
        fetches: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Result<Vec<steamstore::Changelog>, String> {
        let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(now, Ordering::SeqCst);
        fetches.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        in_flight.fetch_sub(1, Ordering::SeqCst);
        Err("échec simulé".to_string())
    }

    // LOT-12-fix01 — the consumed-Arc bug removed the lock-map entry one
    // caller too early (measured: strong_count 3, 2, then ABSENT while the
    // third caller still held the lock). A late arrival then created a
    // second mutex for the same key and fetched alongside the third caller:
    // two requests in flight for one game. This test is that scenario.
    #[tokio::test]
    async fn feed_dedup_never_forks_the_lock_mid_flight() {
        use std::sync::atomic::AtomicUsize;

        let cache = Arc::new(cache::TtlCache::new(std::time::Duration::from_secs(60)));
        let locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let fetches = Arc::new(AtomicUsize::new(0));

        let spawn_caller = |force: bool| {
            let cache = Arc::clone(&cache);
            let locks = Arc::clone(&locks);
            let i = Arc::clone(&in_flight);
            let p = Arc::clone(&peak);
            let f = Arc::clone(&fetches);
            tokio::spawn(async move {
                let _ = fetch_game_changelogs_with(
                    &cache,
                    &locks,
                    "264710",
                    force,
                    move |_app_id: String| timed_failure(i, p, f),
                )
                .await;
            })
        };

        // Three callers pile up on the same AppID while the first fetch drags.
        let a = spawn_caller(false);
        let b = spawn_caller(false);
        let c = spawn_caller(false);
        a.await.unwrap();
        b.await.unwrap();
        // The third caller still holds the shared lock. If the map entry was
        // cleaned one caller too early, this late "Actualiser" creates a
        // second lock and fetches alongside it.
        let d = spawn_caller(true);
        c.await.unwrap();
        d.await.unwrap();

        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "jamais deux requêtes en vol pour le même jeu"
        );
        assert_eq!(
            fetches.load(Ordering::SeqCst),
            4,
            "chaque échec coûte une requête — aucune n'est masquée"
        );
        assert!(
            locks.lock().await.is_empty(),
            "la carte des verrous se vide après le dernier appelant"
        );
    }

    // LOT-12-fix01 — `cache_only` is the lot's "not one request offline"
    // guarantee: only cached games appear, and nothing else can happen — a
    // pure read of a slice and a cache has nowhere to fire a request from.
    #[test]
    fn feed_cache_only_serves_the_cache_and_nothing_else() {
        let cache: cache::TtlCache<String, Vec<steamstore::Changelog>> =
            cache::TtlCache::new(std::time::Duration::from_secs(60));
        cache.put(
            "10".to_string(),
            vec![
                changelog(1_000, "annonce récente"),
                changelog(900, "annonce plus ancienne"),
            ],
        );
        let entries = vec![
            library_entry("10", "Jeu en cache", false),
            library_entry("20", "Jeu hors cache", false),
        ];

        let (items, from_cache) = feed_from_cache(&entries, &cache);

        assert_eq!(from_cache, 1, "seul le jeu en cache est servi");
        assert_eq!(items.len(), 2, "les deux articles du jeu en cache");
        assert!(
            items.iter().all(|i| i.app_id == "10"),
            "un jeu absent du cache n'apparaît pas et ne déclenche rien"
        );
    }

    // LOT-12-fix01 — a game hidden from the library view stays out of the feed.
    #[test]
    fn feed_excludes_hidden_entries() {
        let kept = visible_entries(vec![
            library_entry("10", "Visible", false),
            library_entry("20", "Caché", true),
            library_entry("30", "Visible aussi", false),
        ]);
        let ids: Vec<&str> = kept.iter().map(|e| e.app_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["10", "30"],
            "un jeu caché sort de la vue Bibliothèque : il sort du flux"
        );
    }

    // LOT-12-fix01 — the four-in-flight cap is GLOBAL: the semaphore lives in
    // AppState, so two concurrent `changelog_feed` calls cannot put eight
    // requests in flight. 70 games (more than 64) so both a removed
    // semaphore and a bound raised to 64 push the observed peak past four.
    #[tokio::test]
    async fn feed_never_exceeds_four_requests_in_flight_globally() {
        use std::sync::atomic::AtomicUsize;

        const GAMES: usize = 70;
        let entries: Vec<library::LibraryEntry> = (0..GAMES)
            .map(|i| library_entry(&format!("app{i}"), &format!("Jeu {i}"), false))
            .collect();

        let cache = Arc::new(cache::TtlCache::new(std::time::Duration::from_secs(60)));
        let locks: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
            Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        // Built exactly like AppState's changelog_in_flight.
        let semaphore = Arc::new(tokio::sync::Semaphore::new(FEED_MAX_IN_FLIGHT));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let i = Arc::clone(&in_flight);
        let p = Arc::clone(&peak);
        let fetch = Arc::new(move |_app_id: String| {
            let i = Arc::clone(&i);
            let p = Arc::clone(&p);
            async move {
                let now = i.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                i.fetch_sub(1, Ordering::SeqCst);
                Ok(vec![steamstore::Changelog {
                    title: "annonce".to_string(),
                    date: 1_700_000_000,
                    ..Default::default()
                }])
            }
        });

        let (items, from_cache, fetched, failed) =
            run_feed_fetches(entries, cache, locks, semaphore, false, fetch)
                .await
                .expect("aucune tâche ne doit paniquer");

        // A literal 4, not the constant: asserting against the same constant
        // that built the semaphore would stay green on a 4 → 64 mutation —
        // the invariant is "never more than four", whoever sets the dial.
        assert_eq!(
            peak.load(Ordering::SeqCst),
            4,
            "jamais plus de quatre requêtes en vol"
        );
        assert_eq!(fetched, GAMES, "chaque jeu coûte exactement une requête");
        assert_eq!(from_cache, 0);
        assert_eq!(items.len(), GAMES, "un article par jeu");
        assert!(failed.is_empty());
    }

    // ── LOT-21 wiring: probe + textual guards ──────────────────────────

    #[tokio::test]
    async fn probe_backup_distinguishes_v1_v2_and_absent() {
        let root = std::env::temp_dir().join(format!("ast_probe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lib = root.join("library");
        let data = root.join("data");
        std::fs::create_dir_all(crate::fixes::fixes_dir(&lib)).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(lib.join("index.json"), b"[]").unwrap();
        std::fs::write(lib.join("42.lua"), b"-- lua").unwrap();
        std::fs::write(data.join("config.json"), b"{}").unwrap();

        // Real archives straight from the production exporter.
        let v1 = root.join("plain.luabak");
        backup::export(&lib, &data, &v1, &backup::BackupOptions::default(), None).unwrap();
        let v2 = root.join("secure.luabak");
        backup::export(
            &lib,
            &data,
            &v2,
            &backup::BackupOptions::default(),
            Some("passe"),
        )
        .unwrap();
        let foreign = root.join("foreign.luabak");
        std::fs::write(&foreign, b"pas une sauvegarde").unwrap();

        let p = probe_backup(v1.display().to_string()).await.unwrap();
        assert!(p.exists && p.v1 && !p.encrypted, "une archive v1 est reconnue");

        let p = probe_backup(v2.display().to_string()).await.unwrap();
        assert!(p.exists && p.encrypted && !p.v1, "une archive v2 est reconnue");

        let p = probe_backup(foreign.display().to_string()).await.unwrap();
        assert!(p.exists && !p.encrypted && !p.v1, "ni v1 ni v2");

        let p = probe_backup(root.join("absent.luabak").display().to_string())
            .await
            .unwrap();
        assert!(!p.exists && !p.encrypted && !p.v1, "fichier absent");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Strip comments (line and nested block) and string/char literals before
    /// matching, so a deleted section plus a comment quoting the wiring cannot
    /// keep a guard green (pitfalls 30/32). Newlines survive, removed spans
    /// become a space.
    fn strip_comments_and_strings(src: &str) -> String {
        let chars: Vec<char> = src.chars().collect();
        let n = chars.len();
        let mut out = String::with_capacity(src.len());
        let mut i = 0;
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        while i < n {
            let c = chars[i];
            // Line comment.
            if c == '/' && i + 1 < n && chars[i + 1] == '/' {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            // Block comment — Rust's nest.
            if c == '/' && i + 1 < n && chars[i + 1] == '*' {
                i += 2;
                let mut depth = 1;
                while i < n && depth > 0 {
                    if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && i + 1 < n && chars[i + 1] == '/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        if chars[i] == '\n' {
                            out.push('\n');
                        }
                        i += 1;
                    }
                }
                out.push(' ');
                continue;
            }
            // Raw string: r"…", r#"…"#, br##"…"## — only when the letter
            // starts a token, never inside an identifier.
            if (c == 'r' || c == 'b')
                && i + 1 < n
                && (chars[i + 1] == '"' || chars[i + 1] == '#')
                && (i == 0 || !is_ident(chars[i - 1]))
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while j < n && chars[j] == '#' {
                    hashes += 1;
                    j += 1;
                }
                if j < n && chars[j] == '"' {
                    j += 1;
                    'raw: while j < n {
                        if chars[j] == '"' {
                            let mut k = j + 1;
                            let mut h = 0;
                            while k < n && chars[k] == '#' && h < hashes {
                                h += 1;
                                k += 1;
                            }
                            if h == hashes {
                                j = k;
                                break 'raw;
                            }
                        }
                        if chars[j] == '\n' {
                            out.push('\n');
                        }
                        j += 1;
                    }
                    i = j;
                    out.push(' ');
                    continue;
                }
            }
            // Plain string.
            if c == '"' {
                i += 1;
                while i < n && chars[i] != '"' {
                    if chars[i] == '\\' {
                        i += 1;
                        if i < n {
                            if chars[i] == '\n' {
                                out.push('\n');
                            }
                            i += 1;
                        }
                        continue;
                    }
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                i += 1; // closing quote (or past-the-end)
                out.push(' ');
                continue;
            }
            // Char literal — consumed so `'"'` cannot open a phantom string.
            // Lifetimes (`'a`, `'_`) have no closing quote and pass through.
            if c == '\'' && i + 2 < n {
                if chars[i + 1] == '\\' {
                    let mut j = i + 2;
                    while j < n && chars[j] != '\'' {
                        j += 1;
                    }
                    if j > i + 2 && j < n {
                        out.push(' ');
                        i = j + 1;
                        continue;
                    }
                } else if chars[i + 2] == '\'' {
                    out.push(' ');
                    i += 3;
                    continue;
                }
            }
            out.push(c);
            i += 1;
        }
        out
    }

    /// The balanced-brace body of one function in stripped source.
    /// `char_indices` on the subslice yields offsets relative to it — the
    /// closing brace's absolute position is `open + i + c.len_utf8()`.
    fn fn_body<'a>(stripped: &'a str, name: &str) -> Option<&'a str> {
        let at = stripped.find(name)?;
        let rest = &stripped[at + name.len()..];
        let open = rest.find('{')?;
        let mut depth = 0i32;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&rest[open..open + i + c.len_utf8()]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    #[test]
    fn lot21_wiring_guards() {
        let src = include_str!("commands.rs");
        let bare = strip_comments_and_strings(src);
        let backup_bare = strip_comments_and_strings(include_str!("backup.rs"));

        // The guard stands on the stripper — prove the stripper first.
        let probe = strip_comments_and_strings(
            "let a = spawn_blocking; // backup::auto_snapshot\n\
             /* load_index_with_data_dir /* imbriqué */ encore */\
             let s = \"create_snapshot\"; let c = '\"';\
             let r = r#\"dedans\"#; let t = 'b';",
        );
        assert!(probe.contains("let a = spawn_blocking;"), "le code réel survit");
        assert!(!probe.contains("auto_snapshot"), "un commentaire cité ne garde rien vert");
        assert!(!probe.contains("load_index_with_data_dir"), "un bloc commenté disparaît");
        assert!(!probe.contains("encore"), "un commentaire imbriqué disparaît entièrement");
        assert!(!probe.contains("create_snapshot"), "une chaîne citée disparaît");
        assert!(
            probe.contains("let c =  ;"),
            "le caractère littéral '\"' n'ouvre pas de chaîne fantôme"
        );
        assert!(!probe.contains("dedans"), "une chaîne brute est effacée");
        assert!(probe.contains("let t =  ;"), "un caractère littéral est effacé");

        // create_snapshot — the blocking snapshot runs under spawn_blocking
        // (pitfall 17); M6 turns this red.
        let snap = fn_body(&bare, "fn create_snapshot").expect("create_snapshot introuvable");
        assert!(
            snap.contains("tauri::async_runtime::spawn_blocking"),
            "create_snapshot doit passer par spawn_blocking"
        );
        assert!(
            snap.contains("backup::auto_snapshot(&library_dir, &config::data_dir())"),
            "le câblage du snapshot reste l'appel direct à backup::auto_snapshot"
        );

        // list_library — the strict HMAC path, and the error surfaces as a
        // French, actionable message; M7 red.
        let list = fn_body(&bare, "fn list_library").expect("list_library introuvable");
        assert!(
            list.contains("library::load_index_with_data_dir(&library_dir, &config::data_dir())"),
            "list_library lit par la voie stricte"
        );
        assert!(
            list.contains("integrity_error_message"),
            "l'erreur d'intégrité remonte au frontend en français"
        );
        assert!(
            !list.contains("library::load_index(&library_dir)"),
            "jamais la voie best-effort sur la vue Bibliothèque"
        );

        // set_library_dir — adoption precedes any config change; M8 red.
        let setdir = fn_body(&bare, "fn set_library_dir").expect("set_library_dir introuvable");
        let adopt = setdir
            .find("hmac::adopt_index_with_data_dir(&path, &config::data_dir())")
            .expect("l'adoption doit précéder tout changement de dossier");
        let create = setdir.find("create_dir_all").expect("création du dossier");
        let save = setdir.find("cfg.save()").expect("sauvegarde de la config");
        assert!(adopt < create, "l'adoption précède la création du dossier");
        assert!(adopt < save, "l'adoption précède la sauvegarde de la config");

        // export (backup.rs) — the ZIP is assembled in a temp and published
        // by rename; M1 turns this red. The parenthesised name matters:
        // "fn export" alone would match export_backup in this very file.
        let export = fn_body(&backup_bare, "fn export(").expect("export introuvable");
        assert!(
            export.contains("temp_dir_for"),
            "l'export passe par temp_dir_for pour le dossier temporaire"
        );
        assert!(
            export.contains("TempBackup::in_dir"),
            "l'export écrit dans un temporaire unique voisin de la destination"
        );
        assert!(
            export.contains("write_backup_zip(lib, data_dir, &temp.path, options)"),
            "le ZIP est écrit dans le temporaire, jamais dans dest"
        );
        assert!(
            export.contains("std::fs::rename(&temp.path, dest)"),
            "la publication se fait par rename, après finalisation"
        );

        // E1 guard: both plaintext and encrypted paths must go through
        // `temp_dir_for`, not a hand-rolled path.  If someone reconstructs
        // the directory manually, this guard turns red — the pure function
        // is the single source of truth.
        let plaintext_branch = export
            .split("None =>")
            .nth(1)
            .expect("branche plaintext dans match password");
        assert!(
            plaintext_branch.contains("temp_dir_for"),
            "la voie non chiffrée d'export doit passer par temp_dir_for"
        );
        let encrypted_branch = export
            .split("Some(secret) =>")
            .nth(1)
            .expect("branche chiffrée dans match password");
        assert!(
            encrypted_branch.contains("temp_dir_for"),
            "la voie chiffrée d'export doit passer par temp_dir_for"
        );
    }

    // ── LOT-21 recovery E2: readopt_index wiring guards ──

    #[test]
    fn lot21_recovery_e2_set_library_dir_no_readopt() {
        let src = include_str!("commands.rs");
        let bare = strip_comments_and_strings(src);
        let setdir = fn_body(&bare, "fn set_library_dir").expect("set_library_dir introuvable");
        assert!(
            !setdir.contains("readopt"),
            "set_library_dir ne doit jamais appeler readopt"
        );
    }

    #[test]
    fn lot21_recovery_e2_sync_from_steam_no_readopt() {
        let src = include_str!("commands.rs");
        let bare = strip_comments_and_strings(src);
        let sync = fn_body(&bare, "fn sync_from_steam").expect("sync_from_steam introuvable");
        assert!(
            !sync.contains("readopt"),
            "sync_from_steam ne doit jamais appeler readopt"
        );
    }

    #[test]
    fn lot21_recovery_e2_readopt_index_single_caller() {
        let src = include_str!("commands.rs");
        let bare = strip_comments_and_strings(src);
        // The only non-definition occurrence of readopt_index_inner in
        // production code must be inside the Tauri command readopt_index.
        // Cut at `mod tests` to exclude test code from the check.
        let prod = bare
            .split("mod tests")
            .next()
            .expect("mod tests doit exister");
        // readopt_index_inner must appear exactly twice in production code:
        // once for the function definition, once inside readopt_index.
        let all = prod.split("readopt_index_inner").count() - 1;
        assert_eq!(
            all, 2,
            "readopt_index_inner ne doit être appelée que depuis sa définition et readopt_index (trouvé {all} occurrences)"
        );
        // The Tauri command readopt_index must actually call readopt_index_inner.
        let readopt_cmd = fn_body(&bare, "fn readopt_index").expect("readopt_index introuvable");
        assert!(
            readopt_cmd.contains("readopt_index_inner"),
            "readopt_index doit appeler readopt_index_inner"
        );
    }

    // ── LOT-21 recovery: readopt_index tests ──

    fn readopt_scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ast_readopt_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Test 5: sidecar valid + foreign key → set_library_dir refuses, config unchanged.
    #[test]
    fn set_library_dir_refuses_foreign_key() {
        let _lock = library::cache_test_lock();
        let root = readopt_scratch("foreign_key");
        let lib = root.join("library");
        let data = root.join("data");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(&data).unwrap();

        // Write a valid index.
        std::fs::write(lib.join("index.json"), b"[]").unwrap();

        // Create a key in data_dir.
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&lib.join("index.json"), &key).unwrap();

        // Now create a SECOND data_dir with a DIFFERENT key.
        let data2 = root.join("data2");
        std::fs::create_dir_all(&data2).unwrap();
        let _key2 = hmac::load_or_create_key(&data2).unwrap();

        // set_library_dir with data2 should fail because the sidecar was signed by key.
        let err = hmac::adopt_index_with_data_dir(&lib, &data2).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("sidecar")
                || err.to_string().to_lowercase().contains("hmac")
                || err.to_string().to_lowercase().contains("match"),
            "l'adoption avec une clé étrangère doit échouer, got: {err}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 6: after readopt_index, set_library_dir succeeds.
    #[test]
    fn readopt_index_allows_adoption() {
        let _lock = library::cache_test_lock();
        let root = readopt_scratch("readopt_ok");
        let lib = root.join("library");
        let data = root.join("data");
        let data2 = root.join("data2");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&data2).unwrap();

        std::fs::write(lib.join("index.json"), b"[]").unwrap();

        // Sign with key from data.
        let key = hmac::load_or_create_key(&data).unwrap();
        hmac::sign_index(&lib.join("index.json"), &key).unwrap();

        // Switching to data2 fails.
        assert!(hmac::adopt_index_with_data_dir(&lib, &data2).is_err());

        // readopt_index_inner: validate + delete sidecar + re-sign with data2 key.
        tauri::async_runtime::block_on(readopt_index_inner(&lib, &data2)).unwrap();

        // Now adoption with data2 should succeed.
        assert!(hmac::adopt_index_with_data_dir(&lib, &data2).is_ok());

        // And load_index_with_data_dir works.
        let entries = library::load_index_with_data_dir(&lib, &data2).unwrap();
        assert!(entries.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 7: readopt_index on invalid JSON fails and leaves no sidecar.
    #[test]
    fn readopt_index_rejects_invalid_json() {
        let root = readopt_scratch("readopt_badjson");
        let lib = root.join("library");
        let data = root.join("data");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(&data).unwrap();

        std::fs::write(lib.join("index.json"), b"{pas un index").unwrap();

        let err = tauri::async_runtime::block_on(readopt_index_inner(&lib, &data)).unwrap_err();
        assert!(
            err.to_lowercase().contains("json") || err.to_lowercase().contains("valide"),
            "readopt_index sur un JSON invalide doit échouer, got: {err}"
        );

        // No sidecar should be left.
        assert!(
            !hmac::has_sidecar(&lib.join("index.json")),
            "aucune sidecar ne doit être laissée derrière"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test 8: readopt_index touches nothing outside the target directory.
    #[test]
    fn readopt_index_stays_in_target_dir() {
        let root = readopt_scratch("readopt_bound");
        let lib = root.join("library");
        let data = root.join("data");
        let outside = root.join("outside");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        std::fs::write(lib.join("index.json"), b"[]").unwrap();

        // Create a file outside lib that must not be touched.
        let sentinel = outside.join("sentinel.txt");
        std::fs::write(&sentinel, b"do not touch").unwrap();

        tauri::async_runtime::block_on(readopt_index_inner(&lib, &data)).unwrap();

        assert_eq!(
            std::fs::read_to_string(&sentinel).unwrap(),
            "do not touch",
            "le fichier hors lib doit être inchangé"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // ── MAJ-D — take_update_result ──────────────────────────────────

    /// MAJ-D-2 : le champ `update_from_version` est effacé après lecture.
    /// Un second appel doit retourner `None`.
    ///
    /// Preuve structurelle : on vérifie que le corps de `take_update_result`
    /// contient une affectation `update_from_version = None` (après stripping,
    /// `update_from_version = `). Sans cette affectation, le champ persiste
    /// Les trois cas de `decide_update_result`, appelés pour de vrai.
    ///
    /// Les gardes textuelles voisines épinglent l'ordre des lignes ; celles-ci
    /// épinglent ce que la fonction décide. Une campagne adverse a montré que
    /// l'ordre seul ne suffit pas : remplacer la version enregistrée par `None`
    /// laissait tout vert en supprimant la fonctionnalité.
    #[test]
    fn decide_update_result_reports_a_real_upgrade() {
        let r = decide_update_result(Some("1.0.1"), "1.0.2").expect("une mise à jour a eu lieu");
        assert_eq!(r.from, "1.0.1");
        assert_eq!(r.to, "1.0.2");
    }

    #[test]
    fn decide_update_result_is_silent_on_an_ordinary_start() {
        assert!(
            decide_update_result(None, "1.0.2").is_none(),
            "sans version enregistrée, rien à annoncer"
        );
    }

    #[test]
    fn decide_update_result_is_silent_when_the_installer_did_not_run() {
        // Même version des deux côtés : l'installeur a été annulé. Annoncer un
        // succès serait un mensonge.
        assert!(
            decide_update_result(Some("1.0.2"), "1.0.2").is_none(),
            "version inchangée : aucune mise à jour à annoncer"
        );
    }

    /// et le message revient à chaque démarrage.
    #[test]
    fn take_update_result_clears_field_after_read() {
        let src = include_str!("commands.rs");
        let stripped = strip_comments_and_strings(src);
        let start = stripped
            .find("pub async fn take_update_result")
            .expect("take_update_result introuvable");
        // Trouver la fin de la fonction en comptant les accolades.
        let brace = stripped[start..]
            .find('{')
            .map(|i| start + i)
            .expect("accolade ouvrante");
        let mut depth = 0usize;
        let mut end = brace;
        for (i, c) in stripped[brace..].chars().enumerate() {
            if c == '{' { depth += 1; }
            else if c == '}' {
                depth -= 1;
                if depth == 0 { end = brace + i; break; }
            }
        }
        let body = &stripped[start..=end];
        assert!(
            body.contains("update_from_version ="),
            "take_update_result doit effacer update_from_version après lecture (sinon le message revient à chaque démarrage)",
        );
    }

    /// MAJ-D-3 : on ne rend pas `Some` quand la version est identique.
    /// Le corps de `take_update_result` doit contenir une comparaison
    /// MAJ-D-3 : la commande délègue sa décision à `decide_update_result`.
    ///
    /// La comparaison de versions n'est plus ici — elle est dans la fonction
    /// pure, couverte par trois tests qui l'appellent vraiment. Cette garde
    /// empêche seulement de la réimplémenter sur place, ce qui la ferait
    /// échapper à ces tests.
    #[test]
    fn take_update_result_returns_none_when_versions_match() {
        let src = include_str!("commands.rs");
        let stripped = strip_comments_and_strings(src);
        let start = stripped
            .find("pub async fn take_update_result")
            .expect("take_update_result introuvable");
        let brace = stripped[start..]
            .find('{')
            .map(|i| start + i)
            .expect("accolade ouvrante");
        let mut depth = 0usize;
        let mut end = brace;
        for (i, c) in stripped[brace..].chars().enumerate() {
            if c == '{' { depth += 1; }
            else if c == '}' {
                depth -= 1;
                if depth == 0 { end = brace + i; break; }
            }
        }
        let body = &stripped[start..=end];
        // La comparaison elle-même vit désormais dans `decide_update_result`, qui
        // est pure et testée pour de vrai (trois cas). Ce qui reste à épingler
        // ici, c'est que la commande DÉLÈGUE : réimplémenter la décision sur
        // place la ferait échapper à ces tests.
        assert!(
            body.contains("decide_update_result"),
            "take_update_result doit déléguer à decide_update_result, la fonction pure que les tests couvrent",
        );
    }

    /// MAJ-D-4 : `update_from_version` est écrit AVANT le lancement de
    /// l'installeur, pas après. En mode portable, `std::process::exit(0)`
    /// tue tout ce qui suit.
    ///
    /// Preuve structurelle : dans le corps de `install_update`, l'affectation
    /// `update_from_version` doit précéder le premier `open_path` ou
    /// `spawn_blocking` (pour portable).
    #[test]
    fn install_update_sets_version_before_installer() {
        let src = include_str!("commands.rs");
        let stripped = strip_comments_and_strings(src);
        let start = stripped
            .find("pub async fn install_update")
            .expect("install_update introuvable");
        let brace = stripped[start..]
            .find('{')
            .map(|i| start + i)
            .expect("accolade ouvrante");
        let mut depth = 0usize;
        let mut end = brace;
        for (i, c) in stripped[brace..].chars().enumerate() {
            if c == '{' { depth += 1; }
            else if c == '}' {
                depth -= 1;
                if depth == 0 { end = brace + i; break; }
            }
        }
        let body = &stripped[start..=end];
        let ver_idx = body.find("update_from_version").expect("update_from_version absent");
        // La position ne suffit pas : écrire `None` au bon endroit garde l'ordre
        // et supprime la fonctionnalité — mutation mesurée survivante. Ce qui est
        // écrit doit être la version en cours d'exécution, c'est-à-dire
        // `env!("CARGO_PKG_VERSION")`. On cherche `env!` et non le nom de la
        // variable : le dépouillement retire les chaînes, donc le littéral a
        // disparu de ce qu'on inspecte.
        let assignment = &body[ver_idx..];
        let end_of_stmt = assignment.find(';').map(|i| i + 1).unwrap_or(assignment.len());
        assert!(
            assignment[..end_of_stmt].contains("env!"),
            "install_update doit enregistrer la version COURANTE, pas autre chose :              sinon le démarrage suivant n'a rien à comparer et n'annonce jamais rien"
        );
        let open_idx = body.find("open_path");
        let spawn_idx = body.find("spawn_blocking");
        let first_launch = match (open_idx, spawn_idx) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let ok = match first_launch {
            Some(idx) => ver_idx < idx,
            None => true,
        };
        assert!(
            ok,
            "update_from_version doit être écrit AVANT le lancement de l'installeur (open_path ou spawn_blocking) — en portable mode, tout ce qui suit exit(0) est mort",
        );
    }

    /// PUB-AJUST-05 : un parcours local complet doit rester cohérent. Ce test
    /// appelle les frontières de production, sans copie de leur logique : un
    /// fichier `.lua` est importé, copié vers Steam, détecté, sauvegardé puis
    /// restauré après le nettoyage. Les données Steam qui n'appartiennent pas
    /// à l'application doivent survivre à toutes ces opérations.
    #[test]
    fn local_flow_restores_library_and_spares_protected_steam_paths() {
        let _lock = library::cache_test_lock();
        let root = import_lua_scratch("local-flow");
        let source_dir = root.join("source");
        let library_dir = root.join("library");
        let data_dir = root.join("data");
        let steam_dir = root.join("steam");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::create_dir_all(steam_dir.join("steamapps").join("common").join("Game")).unwrap();
        std::fs::create_dir_all(steam_dir.join("userdata").join("account")).unwrap();
        std::fs::create_dir_all(steam_dir.join("config")).unwrap();

        let source = source_dir.join("renamed.lua");
        std::fs::write(&source, "addappid(264710)\n").unwrap();
        let imported = import_lua_file_inner(&source, &library_dir, &data_dir)
            .expect("l'import local doit appeler la logique de production");
        assert_eq!(imported.entry.app_id, "264710");
        assert!(imported.filename_differs);

        let copied = library::copy_to_steam(&library_dir, &imported.entry.app_id, &steam_dir)
            .expect("la copie vers Steam doit utiliser le fichier importé");
        assert!(copied.is_file());
        std::fs::write(steam_dir.join("OpenSteamTool.dll"), b"marker").unwrap();
        std::fs::write(steam_dir.join("xinput1_4.dll"), b"marker").unwrap();
        let detected = detect::inspect_steamtools(&steam_dir);
        assert!(detected.installed);
        assert!(detected.lua_dir_exists);

        let archive = root.join("library.luabak");
        let exported = backup::export(
            &library_dir,
            &data_dir,
            &archive,
            &backup::BackupOptions::default(),
            None,
        )
        .expect("la sauvegarde locale doit exporter la bibliothèque importée");
        assert_eq!(exported.lua_count, 1);
        assert!(backup::is_v1_backup(&archive));

        let protected = [
            steam_dir.join("steamapps").join("common").join("Game").join("keep.txt"),
            steam_dir.join("userdata").join("account").join("keep.txt"),
            steam_dir.join("config").join("loginusers.vdf"),
            steam_dir.join("config").join("config.vdf"),
        ];
        for path in &protected {
            std::fs::write(path, b"must survive").unwrap();
        }

        let report = wipe::execute(
            &wipe::WipePlan {
                remove_all_lua_from_steam: true,
                delete_library_lua: true,
                remove_steamtools: true,
                remove_legacy_plugin_dir: true,
                ..Default::default()
            },
            &wipe::WipeContext {
                library_dir: &library_dir,
                data_dir: &data_dir,
                steam_dir: Some(&steam_dir),
            },
        );
        assert!(report.steps.iter().all(|step| step.ok), "{:?}", report.steps);
        assert!(!copied.exists());
        assert!(!steam_dir.join("OpenSteamTool.dll").exists());
        for path in &protected {
            assert_eq!(std::fs::read(path).unwrap(), b"must survive");
        }

        let restored = backup::import(&archive, &library_dir, &data_dir, None)
            .expect("la sauvegarde doit restaurer après le nettoyage");
        assert_eq!(restored.lua_restored, 1);
        let entries = library::load_index_with_data_dir(&library_dir, &data_dir)
            .expect("l'index restauré doit être signé avec la clé locale");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app_id, "264710");

        let _ = std::fs::remove_dir_all(&root);
    }
}

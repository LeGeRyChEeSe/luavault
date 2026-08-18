mod archive;
mod artwork;
mod backup;
mod cache;
mod commands;
mod config;
mod defender;
mod detect;
mod discover;
mod fixes;
pub mod hmac;
mod i18n_log;
mod install;
mod library;
mod reachability;
mod stats;
mod steamstore;
pub use update::RELEASE_PUBLIC_KEYS;
mod update;
mod vdf;
mod wipe;
mod exchange;
mod encrypted_backup;

use log::info;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};

pub struct AppState {
    /// General-purpose client used only for public Steam endpoints and installers.
    pub http: reqwest::Client,
    /// Diagnostic state for explicit Steam reachability probes. No background
    /// task reads or updates this state.
    pub reachability: Mutex<reachability::ReachabilityState>,
    /// Dedicated client for the update server — no LuaVault headers, no
    /// decompression (see `update::build_http_client`).
    pub update_http: update::UpdateClient,
    /// Dedicated client for the artwork CDNs (LOT-14).
    pub artwork_http: artwork::ArtworkClient,
    pub config: Mutex<config::AppConfig>,
    /// Set to `true` to ask the running bulk operation to stop after the current game.
    pub bulk_cancel: AtomicBool,
    /// Cached Steam store details — `app_id:lang` key, 5-minute TTL.
    pub steam_details: cache::TtlCache<String, steamstore::SteamDetails>,
    /// Per-key deduplication locks for concurrent Steam details fetches.
    /// Wrapped in an `Arc` so callers can clone the handle and clean up the
    /// entry when the last reference is dropped.
    pub steam_details_locks:
        Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Aggregated changelog feed (LOT-12): per-AppID posts, 30-minute TTL.
    /// Only successes enter — an empty list included (see
    /// `commands::remember_changelogs`).
    pub changelog_cache: Arc<cache::TtlCache<String, Vec<steamstore::Changelog>>>,
    /// Per-key deduplication locks for changelog fetches (LOT-12), same
    /// shape as `steam_details_locks`: `force` bypasses the cache but never
    /// an in-flight request.
    pub changelog_locks:
        Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// The four-in-flight cap on changelog requests, shared by EVERY
    /// `changelog_feed` invocation: a per-call semaphore would let two
    /// concurrent refreshes put eight requests in flight together.
    pub changelog_in_flight: Arc<tokio::sync::Semaphore>,
    /// Artefact produced by `download_update`: `(path, expected SHA-256)`.
    /// `install_update` re-hashes the file against this pair before opening —
    /// the download directory is world-writable for the account, the path
    /// alone proves nothing.
    pub verified_update: Mutex<Option<(String, String)>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir {
                        file_name: Some("luavault".into()),
                    }),
                    Target::new(TargetKind::Webview),
                ])
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let cfg = config::AppConfig::load().unwrap_or_else(|e| {
                eprintln!("config load failed, using defaults: {e}");
                config::AppConfig::default()
            });

            // Remove the stale .old executable left by a previous update.
            // A failure is harmless — it's just a leftover file.
            let exe_dir = config::data_dir();
            let old_exe = exe_dir.join("LuaVault.exe.old");
            if old_exe.exists() {
                let _ = std::fs::remove_file(&old_exe);
            }

            // LOT-14: the webview serves cached artwork through the asset
            // protocol. Its scope is granted HERE, once the path is known —
            // `data_dir()` resolves to %LocalAppData% when installed and to
            // the exe's folder in portable mode, and no static scope in
            // tauri.conf.json can express both. Exactly one folder, not
            // recursive: the webview renders third-party content, a wider
            // scope would be disk read access handed to it.
            let artwork_dir = artwork::cache_dir();
            std::fs::create_dir_all(&artwork_dir).map_err(|e| {
                format!("dossier de cache des images : {e}")
            })?;
            app.asset_protocol_scope()
                .allow_directory(&artwork_dir, false)
                .map_err(|e| format!("scope du protocole asset : {e}"))?;

            app.manage(AppState {
                http: reqwest::Client::new(),
                reachability: Mutex::new(reachability::ReachabilityState::default()),
                update_http: update::UpdateClient::new(),
                artwork_http: artwork::ArtworkClient::new(),
                config: Mutex::new(cfg),
                bulk_cancel: AtomicBool::new(false),
                steam_details: cache::TtlCache::new(std::time::Duration::from_secs(300)),
                steam_details_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
                changelog_cache: Arc::new(cache::TtlCache::new(std::time::Duration::from_secs(
                    30 * 60,
                ))),
                changelog_locks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
                changelog_in_flight: Arc::new(tokio::sync::Semaphore::new(
                    commands::FEED_MAX_IN_FLIGHT,
                )),
                verified_update: Mutex::new(None),
            });
            info!(
                "LuaVault started (portable={}, data_dir={})",
                config::is_portable(),
                config::data_dir().display()
            );
            // Clean up orphaned temporary files left by previous runs.
            backup::cleanup_orphan_temps(&config::data_dir());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::sync_from_steam,
            commands::install_all_fixes,
            commands::repair_all_fixes,
            commands::apply_fixes_to_selection,
            commands::bulk_preflight,
            commands::cancel_bulk,
            commands::get_steam_details,
            commands::get_reachability,
            commands::changelog_feed,
            commands::library_stats,
            commands::set_appearance,
            commands::set_locale,
            commands::list_library,
            commands::library_status,
            commands::game_status,
            commands::remove_library_entry,
            commands::set_library_hidden,
            commands::set_library_display,
            commands::set_library_tags,
            commands::import_lua_file,
            commands::import_patch_archive,
            commands::copy_to_steam,
            commands::sync_library_to_steam,
            commands::remove_lua_from_steam,
            commands::install_game_via_steam,
            commands::launch_game,
            commands::restart_steam,
            commands::install_online_fix,
            commands::verify_online_fix,
            commands::uninstall_online_fix,
            commands::defender_status,
            commands::setup_defender_exclusions,
            commands::set_defender_choice,
            commands::set_default_archive_password,
            commands::verify_defender_exclusions,
            commands::list_backups,
            commands::create_snapshot,
            commands::export_backup,
            commands::import_backup,
            commands::delete_backup,
            commands::probe_backup,
            commands::wipe_preview,
            commands::wipe_execute,
            commands::wipe_protected_paths,
            commands::detect_all,
            commands::set_steam_dir,
            commands::set_library_dir,
            commands::readopt_index,
            commands::install_steam,
            commands::install_steamtools,
            commands::mark_onboarding_done,
            commands::get_log_dir,
            commands::get_app_info,
            commands::export_library,
            commands::preview_import,
            commands::check_update,
            commands::download_update,
            commands::install_update,
            commands::mark_update_notified,
            commands::get_update_notified,
            commands::take_update_result,
            commands::artwork_cached,
            commands::artwork_fetch,
            commands::artwork_cache_info,
            commands::artwork_cache_clear,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

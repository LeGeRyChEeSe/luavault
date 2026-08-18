import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n.svelte";

export interface LibraryEntry {
  app_id: string;
  name: string;
  icon?: string | null;
  file_name: string;
  added_at: string;
  updated_at: string;
  /** The online-fix archive has been downloaded into the library. */
  has_fix: boolean;
  /** Hidden from the library view without deleting anything. */
  hidden: boolean;
  /** User-defined tags for categorising games. */
  tags: string[];
}

export interface GameInstall {
  app_id: string;
  known_to_steam: boolean;
  installed: boolean;
  fully_installed: boolean;
  install_dir?: string | null;
  steam_name?: string | null;
  state_flags: number;
  size_on_disk: number;
}

export type FixHealth = "not_installed" | "healthy" | "damaged" | "game_moved";

export interface FixReport {
  app_id: string;
  health: FixHealth;
  installed_at?: string | null;
  game_dir?: string | null;
  file_count: number;
  missing: string[];
  modified: string[];
  has_backup: boolean;
  /** Online-fix files found in the game folder that we never installed. */
  foreign: string[];
}

export interface UninstallReport {
  removed: number;
  restored: number;
  game_dir: string;
}

/**
 * One game's complete state. `stage` is the single value the UI colour-codes on.
 */
export type GameStage =
  | "no_lua"
  | "lua_not_in_steam"
  | "needs_steam_install"
  | "installing"
  | "ready"
  | "fix_downloaded"
  | "fix_installed"
  | "fix_damaged"
  | "fix_game_moved"
  | "fix_external";

export interface GameStatus {
  app_id: string;
  name: string;
  icon?: string | null;
  updated_at?: string | null;
  added_at?: string | null;
  in_library: boolean;
  lua_in_steam: boolean;
  fix_downloaded: boolean;
  /** Hidden from the library view without deleting anything. */
  hidden: boolean;
  /** User-defined tags for categorising games. */
  tags: string[];
  game: GameInstall;
  /** LOT-13 — minutes played per Steam's local records. null = "on ne sait
   *  pas" (no readable data), never a measured zero; 0 with no last session
   *  = "jamais joué". */
  playtime_minutes?: number | null;
  /** Unix seconds of the last recorded session, when Steam has one. */
  last_played?: number | null;
  fix: FixReport;
  stage: GameStage;
}

export interface BackupOptions {
  include_lua: boolean;
  include_fix_archives: boolean;
  include_fix_states: boolean;
  include_config: boolean;
}

export interface BackupSummary {
  path: string;
  bytes: number;
  lua_count: number;
  fix_archive_count: number;
  fix_state_count: number;
}

export interface SnapshotInfo {
  path: string;
  name: string;
  bytes: number;
  created_at?: string | null;
  lua_count: number;
  fix_archive_count: number;
  automatic: boolean;
  encrypted: boolean;
}

export interface BackupProbe {
  exists: boolean;
  encrypted: boolean;
  v1: boolean;
}

export interface ImportSummary {
  lua_restored: number;
  fix_archives_restored: number;
  fix_states_restored: number;
  config_restored: boolean;
  entries_skipped: number;
  config_kept_local: string[];
}

/** Every switch defaults to `false` — nothing is destroyed unless asked for. */
export interface WipePlan {
  remove_managed_lua_from_steam?: boolean;
  remove_all_lua_from_steam?: boolean;
  uninstall_online_fixes?: boolean;
  delete_fix_archives?: boolean;
  delete_fix_backups?: boolean;
  delete_library_lua?: boolean;
  delete_app_backups?: boolean;
  reset_app_config?: boolean;
  remove_steamtools?: boolean;
  remove_steamtools_conflicts?: boolean;
  remove_legacy_plugin_dir?: boolean;
}

export type WipeLevel = "safe" | "moderate" | "destructive";

export interface WipeAction {
  id: string;
  level: WipeLevel;
  count: number;
}

export type WipeStepDetail =
  | { code: "uninstall_online_fixes_ok"; count: number }
  | { code: "uninstall_online_fixes_partial"; ok_count: number; ignored: number; problems: string }
  | { code: "deleted_with_failures"; done: number; failed: number }
  | { code: "deleted_folder" }
  | { code: "deleted_folder_failed"; e: string }
  | { code: "remove_steamtools_locked"; done: number; failed: number }
  | { code: "deleted_files"; done: number }
  | { code: "steam_missing" }
  | { code: "deleted_archives"; done: number }
  | { code: "deleted_backups"; done: number }
  | { code: "deleted_lua_and_index"; done: number }
  | { code: "deleted_snapshots"; done: number }
  | { code: "config_reset" }
  | { code: "config_missing" };

export interface WipeStep {
  id: string;
  ok: boolean;
  detail: WipeStepDetail;
}

export interface WipeReport {
  steps: WipeStep[];
  needs_elevation: boolean;
}

/** One local `.lua` import, with its AppID read from the file contents. */
export interface LuaImportResult {
  entry: LibraryEntry;
  /** The source stem was not the AppID declared through `addappid`. */
  filename_differs: boolean;
}

/** One patch archive stored locally for a confirmed library game. */
export interface PatchImportResult {
  app_id: string;
  archive_path: string;
  /** True only when the backend read `Name (AppID)` from the filename. */
  app_id_inferred: boolean;
}

export interface SteamStatus {
  path: string;
  source: string;
}

export interface SteamToolsStatus {
  steam_path: string;
  installed: boolean;
  has_open_steam_tool: boolean;
  has_xinput: boolean;
  has_dwmapi: boolean;
  lua_dir_exists: boolean;
  legacy_plugin_dir: boolean;
  conflicts: string[];
}

export interface DetectionReport {
  portable: boolean;
  data_dir: string;
  library_dir: string;
  library_count: number;
  steam: SteamStatus | null;
  steamtools: SteamToolsStatus | null;
  first_run_done: boolean;
  theme?: string | null;
  dark_mode?: boolean | null;
  locale?: string | null;
  /** Defender exclusion choice: null = the app should ask once. */
  defender_exclusions?: boolean | null;
  /** Mot de passe d'archive retenu, proposé par défaut aux prochaines archives chiffrées. */
  default_archive_password?: string | null;
}

export interface DefenderStatus {
  /** False when Defender cannot be queried. */
  available: boolean;
  /** False when a third-party antivirus owns real-time protection. */
  active: boolean;
}

export interface DefenderVerifyReport {
  /** Required folders that were already excluded. */
  already_present: string[];
  /** Required folders that were missing and got added. */
  added: string[];
  /** Malformed/duplicate exclusions that got removed. */
  removed: string[];
}

export interface AppInfo {
  version: string;
  portable: boolean;
  data_dir: string;
}

export interface ImportReport {
  imported: string[];
  fix_checked: number;
  new_fixes: string[];
  errors: string[];
}

export interface StageCount {
  stage: string;
  count: number;
}

/** LOT-13 — the most-played game, among those with recorded data. */
export interface MostPlayedGame {
  app_id: string;
  name: string;
  minutes: number;
}

export interface LibraryStats {
  total: number;
  hidden: number;
  by_stage: StageCount[];
  fixes_installed: number;
  fixes_downloaded: number;
  lua_bytes: number;
  fix_archive_bytes: number;
  backup_bytes: number;
  games_on_disk_bytes: number;
  /** Sum of every KNOWN playtime (minutes) — games without data excluded. */
  playtime_total_minutes: number;
  /** The most-played game (minutes > 0), when any exists. */
  most_played?: MostPlayedGame | null;
  /** Games with no readable playtime data — counted, not treated as zero. */
  playtime_unknown: number;
}

export interface BulkItem {
  app_id: string;
  name: string;
  ok: boolean;
  detail: string;
}

export interface BulkReport {
  items: BulkItem[];
  succeeded: number;
  failed: number;
  skipped: number;
}

export interface BulkPlanItem {
  app_id: string;
  name: string;
  action:
    | "steam_install"
    | "copy_lua"
    | "archive_missing"
    | "install_fix"
    | "verify_fix"
    | "add_tag"
    | "remove_tag"
    | "hide";
  label: string;
  warning?: string | null;
}

export interface BulkPlan {
  steam_detected: boolean;
  steam_running: boolean;
  games: BulkPlanItem[];
  fixes: BulkPlanItem[];
  /** Fifth mode — local selection actions (verify, copy, tag, hide) build
      their confirmation list here. The backend leaves it empty. */
  selection: BulkPlanItem[];
  warnings: string[];
}

export interface BulkProgressEvent {
  phase: "games" | "fixes" | "repair" | "selection";
  current: number;
  total: number;
  app_id: string;
  name: string;
  status: "working" | "ok" | "error" | "skipped";
  detail: string;
  cancelled: boolean;
}

export interface Changelog {
  title: string;
  /** Unix seconds. */
  date: number;
  body: string;
  url: string;
  is_patch_notes: boolean;
}

/** One screenshot in both sizes: the strip shows the thumbnail, the viewer
 *  opens the full one. A shot Steam returns without a full size carries its
 *  thumbnail in both fields — never an empty string. */
export interface Shot {
  thumbnail: string;
  full: string;
}

export interface SteamDetails {
  app_id: string;
  name: string;
  short_description: string;
  header_image?: string | null;
  background?: string | null;
  capsule?: string | null;
  developers: string[];
  publishers: string[];
  genres: string[];
  release_date?: string | null;
  coming_soon: boolean;
  metacritic?: number | null;
  price?: string | null;
  website?: string | null;
  screenshots: Shot[];
  changelog?: Changelog | null;
}

/** One post in the aggregated changelog feed (LOT-12). */
export interface FeedItem {
  app_id: string;
  /** From the library index — never an appdetails call. */
  game_name: string;
  title: string;
  /** Unix seconds. */
  date: number;
  url: string;
  is_patch_notes: boolean;
  /** 400 characters at most — the full body stays on the backend. */
  excerpt: string;
}

/** One game whose announcements could not be fetched. */
export interface FeedFailure {
  app_id: string;
  game_name: string;
  error: string;
}

/** The feed plus what building it actually did — never a silent lie. */
export interface FeedReport {
  items: FeedItem[];
  from_cache: number;
  fetched: number;
  failed: FeedFailure[];
}

export interface Reachability {
  online: boolean;
  consecutive_failures: number;
  last_failure_secs_ago?: number | null;
  tip?: string | null;
}

export const listLibrary = () => invoke<LibraryEntry[]>("list_library");
export const libraryStatus = () => invoke<GameStatus[]>("library_status");
export const libraryStats = () => invoke<LibraryStats>("library_stats");
export const gameStatus = (appId: string) => invoke<GameStatus>("game_status", { appId });
export const removeLibraryEntry = async (appId: string, force = false) => {
  await invoke<void>("remove_library_entry", { appId, force });
};
export const setLibraryHidden = (appId: string, hidden: boolean) =>
  invoke<void>("set_library_hidden", { appId, hidden });
export const setLibraryDisplay = (appId: string, name: string, icon: string | null) =>
  invoke<void>("set_library_display", { appId, name, icon });
export const setLibraryTags = (appId: string, tags: string[]) =>
  invoke<void>("set_library_tags", { appId, tags });
export const importLuaFile = async (path: string) => {
  return await invoke<LuaImportResult>("import_lua_file", { path });
};
export const importPatchArchive = (path: string, appId?: string) =>
  invoke<PatchImportResult>("import_patch_archive", { path, appId: appId ?? null });
export const copyToSteam = (appId: string) => invoke<string>("copy_to_steam", { appId });
export const syncLibraryToSteam = () => invoke<number>("sync_library_to_steam");
export const removeLuaFromSteam = (appId: string) =>
  invoke<boolean>("remove_lua_from_steam", { appId });

export const installGameViaSteam = (appId: string) =>
  invoke<string>("install_game_via_steam", { appId });
export const launchGame = (appId: string) => invoke<string>("launch_game", { appId });
export const restartSteam = () => invoke<string>("restart_steam");

export const installOnlineFix = (appId: string, password?: string) =>
  invoke<FixReport>("install_online_fix", { appId, password });
export const verifyOnlineFix = (appId: string) =>
  invoke<FixReport>("verify_online_fix", { appId });
export const uninstallOnlineFix = (appId: string) =>
  invoke<UninstallReport>("uninstall_online_fix", { appId });

export const defenderStatus = () => invoke<DefenderStatus>("defender_status");
export const setupDefenderExclusions = () => invoke<string[]>("setup_defender_exclusions");
export const setDefenderChoice = (choice: boolean) =>
  invoke<void>("set_defender_choice", { choice });
export const setDefaultArchivePassword = (password: string | null) =>
  invoke<void>("set_default_archive_password", { password });

export function shouldRememberArchivePassword(
  submitted: string | null | undefined,
  current: string | null | undefined,
): boolean {
  return Boolean(submitted) && submitted !== current;
}

export const verifyDefenderExclusions = () =>
  invoke<DefenderVerifyReport>("verify_defender_exclusions");

export const listBackups = () => invoke<SnapshotInfo[]>("list_backups");
export const createSnapshot = () => invoke<BackupSummary>("create_snapshot");
export const exportBackup = (
  path: string,
  options?: BackupOptions,
  password?: string,
) =>
  invoke<BackupSummary>("export_backup", { path, options, password });
export const importBackup = async (
  path: string,
  password?: string,
) => {
  return await invoke<ImportSummary>("import_backup", { path, password });
};
export const deleteBackup = (path: string) => invoke<void>("delete_backup", { path });
export const probeBackup = (path: string) =>
  invoke<BackupProbe>("probe_backup", { path });

export const readoptIndex = (path: string) =>
  invoke<void>("readopt_index", { path });

export const wipePreview = (plan: WipePlan) => invoke<WipeAction[]>("wipe_preview", { plan });
export const wipeExecute = async (plan: WipePlan, snapshotFirst = true) => {
  return await invoke<WipeReport>("wipe_execute", { plan, snapshotFirst });
};
export const wipeProtectedPaths = () => invoke<string[]>("wipe_protected_paths");
export const detectAll = () => invoke<DetectionReport>("detect_all");
export const setSteamDir = (path: string) => invoke<DetectionReport>("set_steam_dir", { path });
export const setLibraryDir = async (path: string) => {
  return await invoke<DetectionReport>("set_library_dir", { path });
};
export const markOnboardingDone = () => invoke<void>("mark_onboarding_done");
export const installSteam = () => invoke<string>("install_steam");
export const installSteamtools = () => invoke<string>("install_steamtools");
export const getLogDir = () => invoke<string>("get_log_dir");
export const getAppInfo = () => invoke<AppInfo>("get_app_info");

export const syncFromSteam = async (): Promise<ImportReport> => {
  return await invoke<ImportReport>("sync_from_steam");
};
/** With `repairOnly`, the plan covers only the broken installs
 *  (`fix_damaged` / `fix_game_moved`) — what the repair pass will treat. */
export const bulkPreflight = (repairOnly = false, selection?: string[]) =>
  invoke<BulkPlan>("bulk_preflight", { repairOnly, selection: selection ?? null });
export const cancelBulk = () => invoke<void>("cancel_bulk");
export const installAllFixes = () => invoke<BulkReport>("install_all_fixes");
/** LOT-15 — re-apply only the fixes that broke; never a first install. */
export const repairAllFixes = () => invoke<BulkReport>("repair_all_fixes");
/** LOT-16 — apply patches to exactly the AppIDs the user selected. */
export const applyFixesToSelection = (appIds: string[]) =>
  invoke<BulkReport>("apply_fixes_to_selection", { appIds });

export const getSteamDetails = (appId: string, lang: string) =>
  invoke<SteamDetails>("get_steam_details", { appId, lang });
/** Aggregated changelog feed (LOT-12). `cacheOnly` never touches the
 *  network — it serves whatever the 30-minute cache still holds. */
export const changelogFeed = (force = false, cacheOnly = false) =>
  invoke<FeedReport>("changelog_feed", { force, cacheOnly });

export const setAppearance = (theme: string, dark: boolean) =>
  invoke<void>("set_appearance", { theme, dark });
export const setLocale = (locale: string) => invoke<void>("set_locale", { locale });

// ── Exchange ──

export interface ImportCandidate {
  app_id: string;
  name?: string | null;
  known: boolean;
}

export interface ImportPreview {
  candidates: ImportCandidate[];
  skipped: string[];
  skipped_total: number;
  total_lines: number;
}

export const exportLibrary = (path: string, format: "csv" | "json") =>
  invoke<number>("export_library", { path, format });
export const previewImport = (path: string) =>
  invoke<ImportPreview>("preview_import", { path });

// ── Update ──

export interface UpdateArtifact {
  kind: string;
  file: string;
  size: number;
  sha256: string;
}

export interface UpdateAvailable {
  version: string;
  published_at: string;
  notes: string | null;
  notes_i18n?: Record<string, string> | null;
  artifacts: UpdateArtifact[];
  /** Versions missed since the installed one, most recent first. */
  changes: { version: string; published_at: string | null; notes: string | null; notes_i18n?: Record<string, string> | null }[];
  /** True when the local version is below `minimum_upgradable_from`: the view
   *  shows the information but hides the download buttons. */
  upgrade_blocked: boolean;
  minimum_upgradable_from?: string | null;
}

export const checkUpdate = () => invoke<UpdateAvailable | null>("check_update");
export const downloadUpdate = (version: string, file: string, sha256: string, size: number) =>
  invoke<string>("download_update", { version, file, sha256, size });
export const installUpdate = (path: string) => invoke<void>("install_update", { path });
export const markUpdateNotified = (version: string) =>
  invoke<void>("mark_update_notified", { version });
export const getUpdateNotified = () => invoke<string | null>("get_update_notified");

/** Result of a completed update, consumed once at startup. */
export interface UpdateResult {
  from: string;
  to: string;
}

export const takeUpdateResult = () => invoke<UpdateResult | null>("take_update_result");

// ── Reachability (offline mode, LOT-11) ──
// The command only seeds the initial state; transitions arrive as
// `reachability://changed` events so the UI never polls.
export const getReachability = () => invoke<Reachability>("get_reachability");

// ── Artwork cache (LOT-14) ──
// Images live on disk and reach the webview through the asset protocol:
// these commands return PATHS (feed them to convertFileSrc), never bytes.

export interface ArtworkCacheInfo {
  bytes: number;
  file_count: number;
}

/** Cache lookup only — never touches the network. */
export const artworkCached = (url: string) =>
  invoke<string | null>("artwork_cached", { url });
/** The cached image's path, downloading it first when needed. */
export const artworkFetch = (url: string) => invoke<string>("artwork_fetch", { url });
export const artworkCacheInfo = () => invoke<ArtworkCacheInfo>("artwork_cache_info");
export const artworkCacheClear = () => invoke<ArtworkCacheInfo>("artwork_cache_clear");

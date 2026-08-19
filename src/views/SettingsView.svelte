<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import Artwork from "../components/Artwork.svelte";
  import Icon from "../components/Icons.svelte";
  import ActionButton from "../components/ActionButton.svelte";
  import ConfirmButton from "../components/ConfirmButton.svelte";
  import { appState } from "../lib/app-state.svelte";
  import {
    artworkCacheClear,
    artworkCacheInfo,
    checkUpdate,
    createSnapshot,
    deleteBackup,
    downloadUpdate,
    probeBackup,
    exportBackup,
    exportLibrary,
    getAppInfo,
    importBackup,
    installUpdate,
    listBackups,
    previewImport,
    setLibraryDir,
    setLibraryHidden,
    setSteamDir,
    setDefaultArchivePassword,
    verifyDefenderExclusions,
    wipeExecute,
    wipePreview,
    wipeProtectedPaths,
  } from "../lib/api";
  import type { BackupProbe, ImportCandidate, ImportPreview } from "../lib/api";
  import type {
    AppInfo,
    ArtworkCacheInfo,
    BackupOptions,
    SnapshotInfo,
    UpdateAvailable,
    WipeAction,
    WipeLevel,
    WipePlan,
    WipeStep,
  } from "../lib/api";
  import { formatBytes, formatDateTime } from "../lib/format";
  import { checkExportPassword } from "../lib/backup-password";
  import { focusTrap } from "../lib/focus-trap";
  import { openFolder, openSteamtoolsFolder } from "../lib/open-folder";
  import { t } from "../lib/i18n.svelte";
  import ThemePicker from "../components/ThemePicker.svelte";
  import LanguagePicker from "../components/LanguagePicker.svelte";

  /** Import toasts mention entries the backend refused to restore, if any. */
  function skippedSuffix(n: number): string {
    if (n <= 0) return "";
    return n === 1
      ? t("settings.import.skipped.one")
      : t("settings.import.skipped.many", { n });
  }

  let appInfo = $state<AppInfo | null>(null);
  let importPassword = $state("");
  let showPasswordDialog = $state(false);
  let importPendingPath = $state<string | null>(null);
  let importError = $state<string | null>(null);
  const report = $derived(appState.report);
  /** SteamTools lives inside the Steam folder — that's the directory to open. */
  const steamtoolsDir = $derived(
    report?.steamtools?.steam_path ?? report?.steam?.path ?? null,
  );
  let busy = $state<string | null>(null);

  function openSteamtoolsDir() {
    if (!steamtoolsDir) return;
    void openSteamtoolsFolder(
      steamtoolsDir,
      t("settings.folders.steamtools.open-failed"),
      (kind, message) => appState.toast(kind, message),
    );
  }

  // --------------------------------------------------------- hidden games
  /** Hidden games stay in the library; this is the only place to bring them back. */
  const hiddenGames = $derived(appState.statuses.filter((s) => s.hidden));
  let showHidden = $state(false);
  let toReveal = $state<Set<string>>(new Set());

  function toggleReveal(appId: string) {
    const next = new Set(toReveal);
    if (next.has(appId)) next.delete(appId);
    else next.add(appId);
    toReveal = next;
  }

  async function revealSelected() {
    if (toReveal.size === 0) return;
    busy = "reveal";
    try {
      for (const appId of toReveal) {
        await setLibraryHidden(appId, false);
      }
      await appState.refreshLibrary();
      appState.toast("success", t("settings.hidden.revealed", { count: toReveal.size }));
      toReveal = new Set();
      showHidden = false;
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  // ---------------------------------------------------------------- backups
  let snapshots = $state<SnapshotInfo[]>([]);
  let exportOptions = $state<BackupOptions>({
    include_lua: true,
    include_fix_archives: true,
    include_fix_states: true,
    include_config: true,
  });
  let encryptExport = $state(false);
  let exportPassword = $state("");
  let exportPasswordConfirm = $state("");

  // ------------------------------------------------------------------ wipe
  let plan = $state<WipePlan>({});
  let preview = $state<WipeAction[]>([]);
  let lastRun = $state<WipeStep[] | null>(null);
  let snapshotFirst = $state(true);
  let protectedPaths = $state<string[]>([]);

  // ────────────────────────────────────────────────────────────────────────
  // Library export / import preview
  let exportBusy = $state<string | null>(null);
  let importPreview = $state<ImportPreview | null>(null);
  let importPreviewError = $state<string | null>(null);

  // ────────────────────────────────────────────────────────────────────────
  // Artwork cache (LOT-14)
  let artworkCache = $state<ArtworkCacheInfo>({ bytes: 0, file_count: 0 });

  async function refreshArtworkCache() {
    try {
      artworkCache = await artworkCacheInfo();
    } catch {
      // The folder may not exist yet — zero is the honest answer.
    }
  }

  const clearArtworkCache = () =>
    run("clear-artwork", async () => {
      const freed = await artworkCacheClear();
      await refreshArtworkCache();
      return t("settings.artwork.cleared", { files: freed.file_count, size: formatBytes(freed.bytes) });
    });

  // ────────────────────────────────────────────────────────────────────────
  // Update
  let updateInfo = $state<UpdateAvailable | null>(null);
  let updateChecked = $state(false);
  let downloadedPath = $state<string | null>(null);

  const updateAvailable = $derived(appState.updateAvailable ?? updateInfo);

  async function doCheckUpdate() {
    busy = "check-update";
    try {
      const result = await checkUpdate();
      updateChecked = true;
      if (result) {
        updateInfo = result;
        appState.updateAvailable = result;
      } else {
        appState.toast("info", t("settings.update.none"));
      }
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function doDownloadUpdate(artifact: { file: string; sha256: string; size: number }) {
    if (!updateAvailable) return;
    busy = "download-update";
    try {
      downloadedPath = await downloadUpdate(
        updateAvailable.version,
        artifact.file,
        artifact.sha256,
        artifact.size,
      );
      appState.toast("success", t("settings.update.downloaded"));
    } catch (e) {
      downloadedPath = null;
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function doInstallUpdate() {
    if (!downloadedPath) return;
    busy = "install-update";
    try {
      await installUpdate(downloadedPath);
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  type Group = { id: "steam" | "steamtools" | "fixes" | "app"; icon: string; items: { key: keyof WipePlan }[] };

  const GROUPS: Group[] = [
    {
      id: "steam",
      icon: "steam",
      items: [
        { key: "remove_managed_lua_from_steam" },
        { key: "remove_all_lua_from_steam" },
        { key: "remove_legacy_plugin_dir" },
      ],
    },
    {
      id: "steamtools",
      icon: "tools",
      items: [
        { key: "remove_steamtools" },
        { key: "remove_steamtools_conflicts" },
      ],
    },
    {
      id: "fixes",
      icon: "patch",
      items: [
        { key: "uninstall_online_fixes" },
        { key: "delete_fix_archives" },
        { key: "delete_fix_backups" },
      ],
    },
    {
      id: "app",
      icon: "save",
      items: [
        { key: "delete_library_lua" },
        { key: "delete_app_backups" },
        { key: "reset_app_config" },
      ],
    },
  ];

  const PRESETS: {
    id: "none" | "light" | "standard" | "deep" | "factory";
    icon: string;
    plan: WipePlan;
  }[] = [
    {
      id: "none",
      icon: "x",
      plan: {},
    },
    {
      id: "light",
      icon: "sparkle",
      plan: { remove_steamtools_conflicts: true, remove_legacy_plugin_dir: true },
    },
    {
      id: "standard",
      icon: "broom",
      plan: {
        remove_steamtools_conflicts: true,
        remove_legacy_plugin_dir: true,
        remove_managed_lua_from_steam: true,
        uninstall_online_fixes: true,
      },
    },
    {
      id: "deep",
      icon: "wrench",
      plan: {
        remove_steamtools_conflicts: true,
        remove_legacy_plugin_dir: true,
        remove_all_lua_from_steam: true,
        uninstall_online_fixes: true,
        delete_fix_archives: true,
        delete_library_lua: true,
        remove_steamtools: true,
      },
    },
    {
      id: "factory",
      icon: "alert",
      plan: {
        remove_managed_lua_from_steam: true,
        remove_all_lua_from_steam: true,
        uninstall_online_fixes: true,
        delete_fix_archives: true,
        delete_fix_backups: true,
        delete_library_lua: true,
        delete_app_backups: true,
        reset_app_config: true,
        remove_steamtools: true,
        remove_steamtools_conflicts: true,
        remove_legacy_plugin_dir: true,
      },
    },
  ];

  const LEVEL_CLASS: Record<WipeLevel, string> = {
    safe: "border-mint/30 bg-mint-soft/50 text-mint-deep",
    moderate: "border-peach/30 bg-peach-soft/50 text-peach-deep",
    destructive: "border-rose/30 bg-rose-soft/55 text-rose-deep",
  };

  const anySelected = $derived(Object.values(plan).some(Boolean));

  $effect(() => {
    void getAppInfo()
      .then((i) => (appInfo = i))
      .catch(() => {});
    void listBackups()
      .then((b) => (snapshots = b))
      .catch(() => {});
    void wipeProtectedPaths()
      .then((p) => (protectedPaths = p))
      .catch(() => {});
    void refreshArtworkCache();
  });

  // Keep the preview in step with the checkboxes.
  $effect(() => {
    const snapshotOfPlan = { ...plan };
    if (!Object.values(snapshotOfPlan).some(Boolean)) {
      preview = [];
      return;
    }
    let cancelled = false;
    void wipePreview(snapshotOfPlan)
      .then((actions) => {
        if (!cancelled) preview = actions;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  });

  async function run(id: string, action: () => Promise<string>) {
    busy = id;
    try {
      appState.toast("success", await action());
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function pickSteam() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    await run("steam", async () => {
      appState.report = await setSteamDir(sel);
      await appState.refreshStatuses();
      return t("settings.folder.steam.saved");
    });
  }

  async function pickLibrary() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    await run("library", async () => {
      appState.report = await setLibraryDir(sel);
      await appState.refreshLibrary();
      return t("settings.folder.library.saved");
    });
  }

  const verifyDefender = () =>
    run("defender-verify", async () => {
      const report = await verifyDefenderExclusions();
      await appState.refresh();
      const parts: string[] = [];
      if (report.added.length > 0) parts.push(t("settings.defender.added", { n: report.added.length }));
      if (report.removed.length > 0) parts.push(t("settings.defender.removed", { n: report.removed.length }));
      parts.push(t("settings.defender.present", { n: report.already_present.length }));
      return t("settings.defender.result", { parts: parts.join(", ") });
    });

  async function clearDefaultArchivePassword() {
    busy = "default-archive-password";
    try {
      await setDefaultArchivePassword(null);
      await appState.refresh();
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function refreshSnapshots() {
    try {
      snapshots = await listBackups();
    } catch {
      // Backup folder may not exist yet.
    }
  }

  const makeSnapshot = () =>
    run("snapshot", async () => {
      const summary = await createSnapshot();
      await refreshSnapshots();
      return t("settings.snapshot.created", { count: summary.lua_count, size: formatBytes(summary.bytes) });
    });

  async function doExport() {
    const check = checkExportPassword(encryptExport, exportPassword, exportPasswordConfirm);
    if (!check.ok) {
      appState.toast(
        "error",
        check.reason === "required"
          ? t("settings.backup.passphrase.required")
          : t("settings.backup.passphrase.mismatch"),
      );
      return;
    }
    const target = await save({
      title: t("settings.backup.export.title"),
      defaultPath: "LuaVault-sauvegarde.luabak",
      filters: [{ name: t("settings.backup.filter"), extensions: ["luabak"] }],
    });
    if (!target) {
      exportPassword = "";
      exportPasswordConfirm = "";
      return;
    }
    try {
      await run("export", async () => {
        const summary = await exportBackup(target, exportOptions, check.password);
        return t("settings.backup.exported", { lua: summary.lua_count, archives: summary.fix_archive_count, size: formatBytes(summary.bytes) });
      });
    } finally {
      exportPassword = "";
      exportPasswordConfirm = "";
    }
  }

  async function doImport() {
    const source = await open({
      title: t("settings.backup.import.title"),
      multiple: false,
      filters: [{ name: t("settings.backup.filter"), extensions: ["luabak"] }],
    });
    if (typeof source !== "string") return;
    await runImportWithPassword(source);
  }

  async function restoreSnapshot(snapshot: SnapshotInfo) {
    await runImportWithPassword(snapshot.path, snapshot.name);
  }

  /** Common door: probe, then restore — asking for a password when needed. */
  async function runImportWithPassword(path: string, displayName?: string) {
    const probe = await probeBackup(path);
    if (!probe.exists) {
      appState.toast("error", t("settings.backup.invalid"));
      return;
    }
    if (probe.encrypted) {
      importPendingPath = path;
      importError = null;
      showPasswordDialog = true;
      return;
    }
    await doRestore(path, displayName);
  }

  function configKeptSuffix(kept: string[]): string {
    if (kept.length === 0) return "";
    const names = kept.map((k) =>
      k === "steam_dir"
        ? t("settings.backup.kept.steam_dir")
        : k === "library_dir"
          ? t("settings.backup.kept.library_dir")
          : k,
    );
    return t("settings.backup.kept.suffix", { names: names.join(", ") });
  }

  async function doRestore(path: string, displayName?: string) {
    const label = displayName || "import";
    await run(`restore-${label}`, async () => {
      const summary = await importBackup(path, importPassword || undefined);
      await appState.refresh();
      await refreshSnapshots();
      if (displayName) {
        return t("settings.backup.restored.named", { name: displayName, lua: summary.lua_restored });
      }
      let msg = summary.config_restored
        ? t("settings.backup.restored.config", { lua: summary.lua_restored, archives: summary.fix_archives_restored })
        : t("settings.backup.restored", { lua: summary.lua_restored, archives: summary.fix_archives_restored });
      msg += configKeptSuffix(summary.config_kept_local);
      msg += skippedSuffix(summary.entries_skipped);
      return msg;
    });
  }

  async function handlePasswordSubmit() {
    if (busy === "import") return;
    busy = "import";
    try {
      if (!importPendingPath) return;
      const path = importPendingPath;
      const pwd = importPassword;
      importError = null;
      const summary = await importBackup(path, pwd || undefined);
      await appState.refresh();
      await refreshSnapshots();
      appState.toast(
        "success",
        (summary.config_restored
          ? t("settings.backup.restored.config", { lua: summary.lua_restored, archives: summary.fix_archives_restored })
          : t("settings.backup.restored", { lua: summary.lua_restored, archives: summary.fix_archives_restored })) + configKeptSuffix(summary.config_kept_local),
      );
      showPasswordDialog = false;
      importPendingPath = null;
      importPassword = "";
    } catch (e) {
      importError = String(e);
    } finally {
      busy = null;
    }
  }

  function handleClosePasswordDialog() {
    if (busy === "import") return;
    showPasswordDialog = false;
    importPendingPath = null;
    importPassword = "";
    importError = null;
  }

  async function dropSnapshot(snapshot: SnapshotInfo) {
    await run(`drop-${snapshot.name}`, async () => {
      await deleteBackup(snapshot.path);
      await refreshSnapshots();
      return t("settings.snapshot.deleted", { name: snapshot.name });
    });
  }

  // ────────────────────────────────────────────────────────────────────────
  // Library export / import
  async function doExportLibraryCsv() {
    exportBusy = "csv";
    try {
      const target = await save({
        title: t("settings.export.title"),
        defaultPath: "LuaVault-bibliotheque.csv",
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (!target) return;
      const count = await exportLibrary(target, "csv");
      appState.toast("success", t("settings.export.csv.done", { n: count }));
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      exportBusy = null;
    }
  }

  async function doExportLibraryJson() {
    exportBusy = "json";
    try {
      const target = await save({
        title: t("settings.export.title"),
        defaultPath: "LuaVault-bibliotheque.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!target) return;
      const count = await exportLibrary(target, "json");
      appState.toast("success", t("settings.export.json.done", { n: count }));
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      exportBusy = null;
    }
  }

  async function doImportList() {
    exportBusy = "import-list";
    try {
      const source = await open({
        title: t("settings.import.list.title"),
        multiple: false,
        filters: [
          { name: "CSV", extensions: ["csv"] },
          { name: t("settings.import.filter.text"), extensions: ["txt"] },
          { name: t("settings.import.filter.all"), extensions: ["*"] },
        ],
      });
      if (typeof source !== "string") return;
      importPreviewError = null;
      importPreview = null;
      const preview = await previewImport(source);
      importPreview = preview;
    } catch (e) {
      importPreviewError = String(e);
    } finally {
      exportBusy = null;
    }
  }

  function applyPreset(preset: WipePlan) {
    plan = { ...preset };
    lastRun = null;
  }

  function toggle(key: keyof WipePlan) {
    plan = { ...plan, [key]: !plan[key] };
    lastRun = null;
  }

  async function executeWipe() {
    busy = "wipe";
    try {
      const result = await wipeExecute(plan, snapshotFirst);
      lastRun = result.steps;
      await appState.refresh();
      await refreshSnapshots();
      const failures = result.steps.filter((s) => !s.ok).length;
      if (failures > 0) {
        appState.toast(
          "error",
          result.needs_elevation
            ? t("settings.wipe.blocked", { count: failures })
            : t("settings.wipe.failed", { count: failures }),
        );
      } else {
        appState.toast("success", t("settings.wipe.done"));
      }
      plan = {};
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }
</script>

<div class="flex h-full flex-col gap-4 overflow-y-auto p-1 pr-2">
  <header class="glass enter-up rounded-xl2 p-5">
    <h2 class="flex items-center gap-2 text-lg font-semibold">
      <Icon name="settings" size={20} />
      {t("settings.title")}
    </h2>
    <p class="mt-0.5 text-sm text-azure-900/60">
      {t("settings.subtitle")}
    </p>
  </header>

  <!-- --------------------------------------------------------- appearance -->
  <section class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
    <div>
      <h3 class="flex items-center gap-2 text-sm font-semibold">
        <Icon name="palette" size={16} />
        {t("settings.appearance.title")}
      </h3>
      <p class="mt-0.5 text-xs text-azure-900/55">
        {t("settings.appearance.hint")}
      </p>
    </div>
    <ThemePicker />
  </section>

  <!-- ------------------------------------------------------------ language -->
  <section class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
    <div>
      <h3 class="flex items-center gap-2 text-sm font-semibold">
        <Icon name="globe" size={16} />
        {t("settings.language.title")}
      </h3>
      <p class="mt-0.5 text-xs text-azure-900/55">
        {t("settings.language.hint")}
      </p>
    </div>
    <LanguagePicker />
  </section>

  <!-- -------------------------------------------------------- hidden games -->
  {#if hiddenGames.length > 0}
    <section class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
      <div class="flex flex-wrap items-center gap-3">
        <div class="min-w-0 flex-1">
          <h3 class="flex items-center gap-2 text-sm font-semibold">
            <Icon name="eye-off" size={16} />
            {t("settings.hidden.title")}
          </h3>
          <p class="mt-0.5 text-xs text-azure-900/55">
            {t("settings.hidden.count", { count: hiddenGames.length })}
            {t("settings.hidden.hint")}
          </p>
        </div>
        <ActionButton
          label={showHidden ? t("settings.hidden.close") : t("settings.hidden.open")}
          icon={showHidden ? "x" : "eye"}
          disabled={busy !== null}
          onclick={() => (showHidden = !showHidden)}
        />
      </div>

      {#if showHidden}
        <ul class="enter-fade flex flex-col gap-1.5">
          {#each hiddenGames as game (game.app_id)}
            <li>
              <label
                class="flex cursor-pointer items-center gap-3 rounded-xl bg-surface/55 px-4 py-2.5 text-sm transition hover:bg-surface/75"
              >
                <input
                  type="checkbox"
                  checked={toReveal.has(game.app_id)}
                  onchange={() => toggleReveal(game.app_id)}
                  class="accent-azure-500"
                />
                <Artwork
                  url={game.icon}
                  class="h-8 w-16 shrink-0 rounded-md object-cover shadow-sm"
                  iconSize={14}
                />
                <span class="min-w-0 flex-1 truncate font-medium">{game.name}</span>
                <span class="shrink-0 text-xs text-azure-900/45">{game.app_id}</span>
              </label>
            </li>
          {/each}
        </ul>

        <div class="flex justify-end">
          <ActionButton
            label={t("settings.hidden.reveal", { count: toReveal.size })}
            icon="eye"
            variant="primary"
            disabled={busy !== null || toReveal.size === 0}
            busy={busy === "reveal"}
            busyLabel={t("settings.hidden.reveal.busy")}
            onclick={revealSelected}
            tip={t("settings.hidden.reveal.tip")}
          />
        </div>
      {/if}
    </section>
  {/if}

  <!-- ------------------------------------------------------------ folders -->
  <section class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
    <h3 class="flex items-center gap-2 text-sm font-semibold">
      <Icon name="folder" size={16} />
      {t("settings.folders.title")}
    </h3>

    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <div class="text-sm font-semibold">{t("settings.folders.steam.label")}</div>
        <div class="mt-0.5 break-all text-sm text-azure-900/60">
          {report?.steam?.path ?? t("settings.folders.undetected")}
        </div>
      </div>
      <ActionButton
        label={t("settings.folders.edit")}
        icon="edit"
        disabled={busy !== null}
        onclick={pickSteam}
        tip={t("settings.folders.steam.tip")}
      />
    </div>

    <div class="h-px bg-surface/60"></div>

    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-2 text-sm font-semibold">
          {t("settings.folders.archive-password.label")}
          {#if report?.default_archive_password}
            <span
              class="rounded-full border border-mint/30 bg-mint-soft/70 px-2 py-px text-[0.66rem] font-bold text-mint-deep"
            >
              {t("settings.folders.archive-password.saved")}
            </span>
          {:else}
            <span
              class="rounded-full border border-peach/30 bg-peach-soft/60 px-2 py-px text-[0.66rem] font-bold text-peach-deep"
            >
              {t("settings.folders.archive-password.none")}
            </span>
          {/if}
        </div>
      </div>
      <ActionButton
        label={t("settings.folders.archive-password.clear")}
        icon="trash"
        disabled={busy !== null || !report?.default_archive_password}
        busy={busy === "default-archive-password"}
        busyLabel="…"
        onclick={clearDefaultArchivePassword}
      />
    </div>

    <div class="h-px bg-surface/60"></div>

    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-2 text-sm font-semibold">
          SteamTools
          {#if report?.steamtools?.installed}
            <span
              class="rounded-full border border-mint/30 bg-mint-soft/70 px-2 py-px text-[0.66rem] font-bold text-mint-deep"
            >
              {t("settings.folders.steamtools.installed")}
            </span>
          {:else if report?.steam}
            <span
              class="rounded-full border border-peach/30 bg-peach-soft/60 px-2 py-px text-[0.66rem] font-bold text-peach-deep"
            >
              {t("settings.folders.steamtools.absent")}
            </span>
          {/if}
        </div>
        <div class="mt-0.5 break-all text-sm text-azure-900/60">
          {steamtoolsDir ?? t("settings.folders.undetected")}
          <span class="text-azure-900/40">
            {t("settings.folders.steamtools.note")}
          </span>
        </div>
      </div>
      <ActionButton
        label={t("settings.folders.open")}
        icon="folder"
        disabled={!steamtoolsDir}
        onclick={openSteamtoolsDir}
        tip={t("settings.folders.steamtools.tip")}
      />
    </div>

    <div class="h-px bg-surface/60"></div>

    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-2 text-sm font-semibold">
          {t("settings.folders.defender.label")}
          {#if report?.defender_exclusions}
            <span
              class="rounded-full border border-mint/30 bg-mint-soft/70 px-2 py-px text-[0.66rem] font-bold text-mint-deep"
            >
              {t("settings.folders.defender.on")}
            </span>
          {:else}
            <span
              class="rounded-full border border-peach/30 bg-peach-soft/60 px-2 py-px text-[0.66rem] font-bold text-peach-deep"
            >
              {t("settings.folders.defender.off")}
            </span>
          {/if}
        </div>
        <div class="mt-0.5 text-sm text-azure-900/60">
          {t("settings.folders.defender.hint")}
        </div>
      </div>
      <ActionButton
        label={t("settings.folders.defender.action")}
        icon="shield"
        disabled={busy !== null || !report?.steam}
        busy={busy === "defender-verify"}
        busyLabel={t("settings.busy.verifying")}
        onclick={verifyDefender}
        tip={t("settings.folders.defender.tip")}
      />
    </div>

    <div class="h-px bg-surface/60"></div>

    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <div class="text-sm font-semibold">{t("settings.folders.library.label")}</div>
        <div class="mt-0.5 break-all text-sm text-azure-900/60">
          {report?.library_dir ?? "…"}
        </div>
      </div>
      <ActionButton
        label={t("settings.folders.open")}
        icon="folder"
        onclick={() => report && void openFolder(report.library_dir)}
      />
      <ActionButton label={t("settings.folders.edit")} icon="edit" disabled={busy !== null} onclick={pickLibrary} />
    </div>
  </section>

  <!-- ------------------------------------------------------------ backups -->
  <section class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <h3 class="flex items-center gap-2 text-sm font-semibold">
          <Icon name="archive" size={16} />
          {t("settings.backup.section.title")}
        </h3>
        <p class="mt-0.5 text-sm text-azure-900/60">
          {t("settings.backup.section.hint")}
        </p>
      </div>
      <ActionButton
        label={t("settings.backup.snapshot.action")}
        icon="save"
        disabled={busy !== null}
        busy={busy === "snapshot"}
        busyLabel={t("settings.backup.snapshot.busy")}
        onclick={makeSnapshot}
        tip={t("settings.backup.snapshot.tip")}
      />
      <ActionButton
        label={t("settings.backup.import.action")}
        icon="upload"
        disabled={busy !== null}
        busy={busy === "import"}
        busyLabel={t("settings.backup.import.busy")}
        onclick={doImport}
        tip={t("settings.backup.import.tip")}
      />
      <ActionButton
        label={t("settings.backup.export.action")}
        icon="download"
        variant="primary"
        disabled={busy !== null}
        busy={busy === "export"}
        busyLabel={t("settings.backup.export.busy")}
        onclick={doExport}
        tip={t("settings.backup.export.tip")}
      />
    </div>

    <div class="flex flex-wrap gap-x-5 gap-y-2 rounded-xl bg-surface/50 px-4 py-3 text-sm">
      <span class="text-azure-900/55">{t("settings.backup.contents.label")}</span>
      <label class="flex cursor-pointer items-center gap-2">
        <input type="checkbox" bind:checked={exportOptions.include_lua} class="accent-azure-500" />
        {t("settings.backup.contents.lua")}
      </label>
      <label class="flex cursor-pointer items-center gap-2" data-tip={t("settings.backup.contents.archives.tip")}>
        <input
          type="checkbox"
          bind:checked={exportOptions.include_fix_archives}
          class="accent-azure-500"
        />
        {t("settings.backup.contents.archives")}
      </label>
      <label class="flex cursor-pointer items-center gap-2">
        <input
          type="checkbox"
          bind:checked={exportOptions.include_fix_states}
          class="accent-azure-500"
        />
        {t("settings.backup.contents.states")}
      </label>
      <label class="flex cursor-pointer items-center gap-2">
        <input
          type="checkbox"
          bind:checked={exportOptions.include_config}
          class="accent-azure-500"
        />
        {t("settings.backup.contents.config")}
      </label>
      <label class="flex cursor-pointer items-center gap-2">
        <input
          type="checkbox"
          bind:checked={encryptExport}
          class="accent-azure-500"
        />
        {t("settings.backup.contents.encrypt")}
      </label>
    </div>

    {#if encryptExport}
      <div class="mt-2 space-y-2 rounded-xl bg-surface/50 px-4 py-3">
        <div class="text-xs text-peach-deep">
          {t("settings.backup.encrypt.warning")}
        </div>
        <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
          <label for="export-password" class="text-sm">{t("settings.backup.passphrase.label")}</label>
          <input
            id="export-password"
            type="password"
            bind:value={exportPassword}
            class="flex-1 rounded-lg border border-surface/65 bg-surface px-3 py-1.5 text-sm outline-none placeholder-azure-400 focus:border-azure-500"
            placeholder={t("settings.backup.passphrase.placeholder")}
          />
        </div>
        <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
          <label for="export-password-confirm" class="text-sm">{t("settings.backup.passphrase.confirm")}</label>
          <input
            id="export-password-confirm"
            type="password"
            bind:value={exportPasswordConfirm}
            class="flex-1 rounded-lg border border-surface/65 bg-surface px-3 py-1.5 text-sm outline-none placeholder-azure-400 focus:border-azure-500"
            placeholder={t("settings.backup.passphrase.confirm.placeholder")}
          />
        </div>
      </div>
    {/if}

    {#if snapshots.length === 0}
      <p class="text-sm text-azure-900/50">{t("settings.backup.snapshots.empty")}</p>
    {:else}
      <ul class="flex flex-col gap-1.5">
        {#each snapshots as snapshot (snapshot.path)}
          <li
            class="lift flex flex-wrap items-center gap-3 rounded-xl bg-surface/55 px-4 py-2.5 text-sm hover:bg-surface/75"
          >
            <Icon name={snapshot.automatic ? "clock" : "save"} size={16} />
            {#if snapshot.encrypted}
              <span data-tip={t("settings.backup.snapshot.sealed.tip")}>
                <Icon name="lock" size={14} />
              </span>
            {/if}
            <div class="min-w-0 flex-1">
              <div class="truncate font-medium">{snapshot.name}</div>
              <div class="text-xs text-azure-900/50">
                {formatDateTime(snapshot.created_at)}
                {#if snapshot.encrypted}
                  · {t("settings.backup.snapshot.sealed")}
                {:else}
                  · {snapshot.lua_count} .lua
                  {#if snapshot.fix_archive_count > 0}
                    · {t("settings.backup.snapshot.archives", { n: snapshot.fix_archive_count })}
                  {/if}
                {/if}
                · {formatBytes(snapshot.bytes)}
              </div>
            </div>
            <ActionButton
              label={t("settings.backup.restore.action")}
              icon="refresh"
              size="sm"
              disabled={busy !== null}
              busy={busy === `restore-${snapshot.name}`}
              busyLabel="…"
              onclick={() => restoreSnapshot(snapshot)}
              tip={t("settings.backup.restore.tip")}
            />
            <ActionButton
              label={t("settings.backup.delete.action")}
              icon="trash"
              variant="danger"
              size="sm"
              disabled={busy !== null}
              busy={busy === `drop-${snapshot.name}`}
              busyLabel="…"
              onclick={() => dropSnapshot(snapshot)}
            />
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <!-- ──────────────────────────────────────────────────────────────── password dialog -->
  {#if showPasswordDialog}
    <div
      class="fixed inset-0 z-[300] lv-veil"
      onclick={handleClosePasswordDialog}
      role="presentation"
    ></div>
    <div
      class="glass-strong enter-fade fixed z-[301] w-full max-w-md rounded-xl border border-surface/60 p-5 shadow-xl"
      style="top: 50%; left: 50%; transform: translate(-50%, -50%);"
      use:focusTrap={{ returnFocus: null }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="import-password-title"
      tabindex="-1"
      onkeydown={(e: KeyboardEvent) => {
        if (e.key === "Escape") {
          e.preventDefault();
          handleClosePasswordDialog();
        } else if (e.key === "Enter") {
          e.preventDefault();
          handlePasswordSubmit();
        }
      }}
    >
      <div id="import-password-title" class="sr-only">{t("settings.backup.dialog.aria")}</div>
      <h3 class="text-base font-semibold mb-3">{t("settings.backup.dialog.title")}</h3>
      <p class="text-sm text-azure-900/70 mb-4">
        {t("settings.backup.dialog.hint")}
      </p>
      {#if importError}
        <div class="mb-3 rounded-lg bg-rose/10 px-3 py-2 text-sm text-rose">
          {importError}
        </div>
      {/if}
      <div class="flex flex-col gap-3">
        <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
          <label for="import-password" class="text-sm">{t("settings.backup.passphrase.label")}</label>
          <input
            id="import-password"
            type="password"
            bind:value={importPassword}
            class="flex-1 rounded-lg border border-surface/65 bg-surface px-3 py-1.5 text-sm outline-none placeholder-azure-400 focus:border-azure-500"
            placeholder={t("settings.backup.passphrase.placeholder")}
            autocomplete="off"
          />
        </div>
        <div class="flex justify-end gap-2">
          <button
            onclick={handleClosePasswordDialog}
            class="lift rounded-md px-3 py-1.5 text-sm text-azure-900/70 hover:bg-surface/70 hover:text-azure-900"
          >
            {t("settings.backup.dialog.cancel")}
          </button>
          <button
            onclick={handlePasswordSubmit}
            disabled={busy === "import"}
            class="lift rounded-md px-3 py-1.5 text-sm font-medium text-white bg-azure-600 hover:bg-azure-700 disabled:cursor-not-allowed disabled:opacity-45"
          >
            {busy === "import" ? t("settings.backup.dialog.busy") : t("settings.backup.restore.action")}
          </button>
        </div>
      </div>
    </div>
  {/if}

  <!-- ──────────────────────────────────────────────────────────────── library export/import -->
  <section class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
    <div>
      <h3 class="flex items-center gap-2 text-sm font-semibold">
        <Icon name="layers" size={16} />
        {t("settings.data.title")}
      </h3>
      <p class="mt-0.5 text-sm text-azure-900/60">
        {t("settings.data.hint")}
      </p>
    </div>

    <div class="flex flex-wrap gap-2">
      <ActionButton
        label={t("settings.data.csv.action")}
        icon="download"
        disabled={exportBusy !== null}
        busy={exportBusy === "csv"}
        busyLabel={t("settings.data.csv.busy")}
        onclick={doExportLibraryCsv}
        tip={t("settings.data.csv.tip")}
      />
      <ActionButton
        label={t("settings.data.json.action")}
        icon="download"
        disabled={exportBusy !== null}
        busy={exportBusy === "json"}
        busyLabel={t("settings.data.json.busy")}
        onclick={doExportLibraryJson}
        tip={t("settings.data.json.tip")}
      />
      <ActionButton
        label={t("settings.import.list.title")}
        icon="upload"
        disabled={exportBusy !== null}
        busy={exportBusy === "import-list"}
        busyLabel={t("settings.backup.import.busy")}
        onclick={doImportList}
        tip={t("settings.data.import.tip")}
      />
    </div>

    {#if importPreviewError}
      <div
        class="flex items-start gap-2 rounded-xl border border-rose/30 bg-rose-soft/50 px-4 py-3 text-sm text-rose-deep"
      >
        <Icon name="error" size={16} class="mt-0.5 shrink-0" />
        <div class="min-w-0 flex-1">
          <p class="font-semibold">{t("settings.data.error.title")}</p>
          <p class="mt-0.5 text-xs opacity-85">{importPreviewError}</p>
        </div>
        <ActionButton
          label={t("settings.data.close")}
          icon="x"
          size="sm"
          onclick={() => { importPreviewError = null; importPreview = null; }}
        />
      </div>
    {/if}

    {#if importPreview}
      <div class="enter-fade flex flex-col gap-3 rounded-xl bg-surface/50 p-4">
        <div class="flex flex-wrap items-center gap-3 text-sm">
          <span class="text-azure-900/55">{t("settings.data.preview.lines")}</span>
          <span class="font-semibold">{importPreview.total_lines}</span>
          <span class="text-azure-900/30">|</span>
          <span class="text-azure-900/55">{t("settings.data.preview.candidates")}</span>
          <span class="font-semibold">{importPreview.candidates.length}</span>
          <span class="text-azure-900/30">|</span>
          <span class="text-azure-900/55">{t("settings.data.preview.known")}</span>
          <span class="font-semibold">{importPreview.candidates.filter((c) => c.known).length}</span>
          <span class="text-azure-900/30">|</span>
          <span class="text-azure-900/55">{t("settings.data.preview.new")}</span>
          <span class="font-semibold">{importPreview.candidates.filter((c) => !c.known).length}</span>
          {#if importPreview.skipped_total > 0}
            <span class="text-azure-900/30">|</span>
            <span class="text-azure-900/55">{t("settings.data.preview.skipped")}</span>
            <span class="font-semibold">{importPreview.skipped_total}</span>
          {/if}
        </div>

        {#if importPreview.candidates.some((c) => !c.known)}
          <p class="text-xs text-azure-900/55">
            {t("settings.data.preview.hint")}
          </p>
        {/if}

        {#if importPreview.candidates.length > 0}
          <ul class="flex max-h-64 flex-col gap-1 overflow-y-auto">
            {#each importPreview.candidates.slice(0, 200) as candidate (candidate.app_id)}
              <li
                class="flex items-center gap-3 rounded-lg bg-surface/45 px-3 py-2 text-sm transition hover:bg-surface/65"
              >
                <span
                  class="shrink-0 rounded-full border border-azure-200/50 bg-surface/60 px-2 py-0.5 text-[0.68rem] font-bold uppercase tracking-wide"
                  data-tip={candidate.known ? t("settings.data.badge.known.tip") : t("settings.data.badge.new.tip")}
                >
                  {candidate.known ? t("settings.data.badge.known") : t("settings.data.badge.new")}
                </span>
                <span class="font-mono font-semibold">{candidate.app_id}</span>
                {#if candidate.name}
                  <span class="min-w-0 flex-1 truncate text-azure-900/65">{candidate.name}</span>
                {/if}
              </li>
            {/each}
            {#if importPreview.candidates.length > 200}
              <li class="px-3 py-2 text-xs italic text-azure-900/45">
                {t("settings.data.more", { n: importPreview.candidates.length - 200 })}
              </li>
            {/if}
          </ul>
        {/if}

        {#if importPreview.skipped.length > 0}
          <details class="rounded-lg bg-surface/45 p-3">
            <summary class="cursor-pointer text-xs font-semibold text-azure-900/55">
              {importPreview.skipped_total > importPreview.skipped.length
                ? t("settings.data.skipped.partial", { shown: importPreview.skipped.length, total: importPreview.skipped_total })
                : t("settings.data.skipped.all", { n: importPreview.skipped.length })}
            </summary>
            <ul class="mt-2 flex flex-col gap-1 text-xs text-azure-900/50">
              {#each importPreview.skipped as reason (reason)}
                <li>{reason}</li>
              {/each}
            </ul>
          </details>
        {/if}
      </div>
    {/if}

    {#if importPreview && importPreview.candidates.length === 0}
      <p class="text-xs text-azure-900/55">
        {#if importPreview.total_lines === 0}
          {t("settings.data.empty.file")}
        {:else}
          {t("settings.data.empty.none")}
        {/if}
      </p>
    {/if}
  </section>

  <!-- ------------------------------------------------- artwork cache (LOT-14) -->
  <section class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <h3 class="flex items-center gap-2 text-sm font-semibold">
          <Icon name="image" size={16} />
          {t("settings.artwork.title")}
        </h3>
        <p class="mt-0.5 text-sm text-azure-900/60">
          {t("settings.artwork.hint")}
          {artworkCache.file_count > 1
            ? t("settings.artwork.usage.many", { size: formatBytes(artworkCache.bytes), n: artworkCache.file_count })
            : t("settings.artwork.usage.one", { size: formatBytes(artworkCache.bytes), n: artworkCache.file_count })}
          {t("settings.artwork.reassure")}
        </p>
      </div>
      <ConfirmButton
        label={t("settings.artwork.clear.action")}
        confirmLabel={t("settings.artwork.clear.confirm")}
        disabled={busy !== null || artworkCache.file_count === 0}
        onconfirm={clearArtworkCache}
        title={t("settings.artwork.clear.tip")}
      />
    </div>
  </section>

  <!-- --------------------------------------------------------------- wipe -->
  <section class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
    <div>
      <h3 class="flex items-center gap-2 text-sm font-semibold">
        <Icon name="broom" size={16} />
        {t("settings.wipe.title")}
      </h3>
      <p class="mt-0.5 text-sm text-azure-900/60">
        {t("settings.wipe.hint")}
      </p>
    </div>

    <div class="flex flex-wrap gap-1.5">
      {#each PRESETS as preset (preset.id)}
        <button
          onclick={() => applyPreset(preset.plan)}
          data-tip={t(`settings.wipe.preset.${preset.id}.tip`)}
          class="lift inline-flex items-center gap-1.5 rounded-full border border-surface/65 bg-surface/50 px-3 py-1.5 text-xs font-semibold text-azure-800 hover:border-azure-200 hover:bg-surface/80"
        >
          <Icon name={preset.icon} size={13} />
          {t(`settings.wipe.preset.${preset.id}.label`)}
        </button>
      {/each}
    </div>

    <div class="grid grid-cols-2 gap-3 max-xl:grid-cols-1">
      {#each GROUPS as group (group.id)}
        <div class="rounded-xl bg-surface/45 p-4">
          <div class="mb-2 flex items-center gap-2 text-sm font-semibold">
            <Icon name={group.icon} size={15} />
            {t(`settings.wipe.group.${group.id}`)}
          </div>
          <div class="flex flex-col gap-1.5">
            {#each group.items as item (item.key)}
              <label
                data-tip={t(`settings.wipe.item.${item.key}.hint`)}
                class="flex cursor-pointer items-start gap-2.5 rounded-lg px-2 py-1.5 text-sm transition hover:bg-surface/70"
              >
                <input
                  type="checkbox"
                  checked={plan[item.key] ?? false}
                  onchange={() => toggle(item.key)}
                  class="mt-0.5 accent-azure-500"
                />
                <span>{t(`settings.wipe.item.${item.key}.label`)}</span>
              </label>
            {/each}
          </div>
        </div>
      {/each}
    </div>

    {#if protectedPaths.length > 0}
      <div
        class="flex items-start gap-2 rounded-xl border border-mint/25 bg-mint-soft/45 px-4 py-3 text-sm text-mint-deep"
      >
        <Icon name="lock" size={16} class="mt-0.5 shrink-0" />
        <div class="min-w-0">
          <p class="font-semibold">{t("settings.wipe.protected.title")}</p>
          <ul class="mt-1 list-inside list-disc">
            {#each protectedPaths as path (path)}
              <li class="break-all">{path}</li>
            {/each}
          </ul>
        </div>
      </div>
    {/if}

    {#if preview.length > 0}
      <div class="enter-fade flex flex-col gap-1.5">
        <p class="text-sm font-semibold">{t("settings.wipe.preview.title")}</p>
        {#each preview as action (action.id)}
          <div
            class="flex flex-wrap items-center gap-2 rounded-xl border px-4 py-2.5 text-sm {LEVEL_CLASS[
              action.level
            ]}"
          >
            <span class="font-semibold">{t(`settings.wipe.item.${action.id}.label` as import("../lib/i18n.svelte").Key)}</span>
            <span
              class="rounded-full bg-surface/60 px-2 py-0.5 text-[0.68rem] font-bold uppercase tracking-wide"
            >
              {t(`settings.wipe.level.${action.level}`)}
            </span>
            <span
              class="rounded-full bg-surface/60 px-2 py-0.5 text-[0.68rem] font-bold"
              data-tip={t("settings.wipe.count.tip")}
            >
              {action.count}
            </span>
            <span class="w-full text-xs opacity-85">{t(`settings.wipe.item.${action.id}.hint` as import("../lib/i18n.svelte").Key)}</span>
          </div>
        {/each}
      </div>
    {/if}

    <div class="flex flex-wrap items-center gap-3">
      <label
        class="flex cursor-pointer items-center gap-2 text-sm"
        data-tip={t("settings.wipe.snapshot.tip")}
      >
        <input type="checkbox" bind:checked={snapshotFirst} class="accent-azure-500" />
        {t("settings.wipe.snapshot.label")}
      </label>
      <div class="flex-1"></div>
      <ConfirmButton
        label={t("settings.wipe.run.action")}
        confirmLabel={t("settings.wipe.run.confirm")}
        disabled={!anySelected || busy !== null}
        onconfirm={executeWipe}
        primary
        title={t("settings.wipe.run.tip")}
      />
    </div>

    {#if lastRun}
      <div class="enter-fade flex flex-col gap-1.5">
        <p class="text-sm font-semibold">{t("settings.wipe.result.title")}</p>
        {#each lastRun as step (step.id)}
          <div
            class="flex items-start gap-2 rounded-xl px-4 py-2.5 text-sm {step.ok
              ? 'bg-mint-soft/50 text-mint-deep'
              : 'bg-rose-soft/55 text-rose-deep'}"
          >
            <Icon name={step.ok ? "check" : "error"} size={15} class="mt-0.5 shrink-0" />
            <span><span class="font-semibold">{t(`settings.wipe.item.${step.id}.label` as import("../lib/i18n.svelte").Key)}</span> — {t(`settings.wipe.detail.${step.detail.code}` as import("../lib/i18n.svelte").Key, step.detail as Record<string, unknown>)}</span>
          </div>
        {/each}
      </div>
    {/if}
  </section>

  <!-- --------------------------------------------------------------- update -->
  <section class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <h3 class="flex items-center gap-2 text-sm font-semibold">
          <Icon name="update" size={16} />
          {t("settings.update.title")}
        </h3>
        <p class="mt-0.5 text-xs text-azure-900/55">
          {t("settings.update.installed", { version: appInfo?.version ?? "…" })}
          {#if updateAvailable}
            {t("settings.update.available", { version: updateAvailable.version })}
          {:else if updateChecked}
            {t("settings.update.idle")}
          {/if}
        </p>
      </div>
      <ActionButton
        label={t("settings.update.check.action")}
        icon="refresh"
        disabled={busy !== null}
        busy={busy === "check-update"}
        busyLabel={t("settings.busy.verifying")}
        onclick={doCheckUpdate}
      />
    </div>

    {#if updateAvailable}
      {#if updateAvailable.upgrade_blocked}
        <div
          class="flex items-start gap-2 rounded-xl border border-peach/30 bg-peach-soft/50 px-4 py-3 text-sm text-peach-deep"
        >
          <Icon name="info" size={16} class="mt-0.5 shrink-0" />
          <div class="min-w-0 flex-1">
            <p class="font-semibold">{t("settings.update.blocked.title")}</p>
            <p class="mt-0.5 text-xs opacity-85">
              {updateAvailable.minimum_upgradable_from
                ? t("settings.update.blocked.detail.known", {
                    version: updateAvailable.version,
                    step: updateAvailable.minimum_upgradable_from,
                  })
                : t("settings.update.blocked.detail.unknown", { version: updateAvailable.version })}
            </p>
          </div>
        </div>
      {/if}

      {#if updateAvailable.notes}
        <div class="rounded-xl bg-surface/50 px-4 py-3 text-sm whitespace-pre-line">
          {updateAvailable.notes}
        </div>
      {/if}

      {#if !updateAvailable.upgrade_blocked}
        <div class="flex flex-col gap-2">
          {#if updateAvailable.artifacts[0]}
            <div class="flex flex-wrap items-center gap-3 rounded-xl bg-surface/55 px-4 py-2.5 text-sm">
              <Icon name={updateAvailable.artifacts[0].kind === "portable" ? "archive" : "download"} size={15} />
              <span class="min-w-0 flex-1 truncate font-medium">{updateAvailable.artifacts[0].file}</span>
              <span class="text-xs text-azure-900/50">{formatBytes(updateAvailable.artifacts[0].size)}</span>
              {#if !downloadedPath}
                <ActionButton
                  label={t("settings.update.download.action")}
                  icon="download"
                  size="sm"
                  disabled={busy !== null}
                  busy={busy === "download-update"}
                  busyLabel={t("settings.update.download.busy")}
                  onclick={() => doDownloadUpdate(updateAvailable.artifacts[0])}
                />
              {/if}
            </div>
          {/if}
        </div>

        {#if downloadedPath}
          <div class="flex items-center gap-3">
            <span class="flex items-center gap-1.5 text-sm font-medium text-mint-deep">
              <Icon name="check" size={15} />
              {t("settings.update.verified")}
            </span>
            <div class="flex-1"></div>
            <ActionButton
              label={t("update.install")}
              icon="play"
              variant="primary"
              disabled={busy !== null}
              busy={busy === "install-update"}
              busyLabel={t("settings.update.install.busy")}
              onclick={doInstallUpdate}
            />
          </div>
        {/if}
      {/if}
    {/if}
  </section>

  <!-- --------------------------------------------------------------- info -->
  <section class="glass enter-up grid grid-cols-2 gap-3 rounded-xl2 p-5 text-sm max-lg:grid-cols-1">
    <div class="rounded-xl bg-surface/50 px-4 py-3">
      <div class="text-azure-900/50">{t("settings.about.edition.label")}</div>
      <div class="flex items-center gap-2 font-semibold">
        <Icon name={(appInfo?.portable ?? report?.portable) ? "save" : "archive"} size={15} />
        {(appInfo?.portable ?? report?.portable)
          ? t("settings.about.edition.portable")
          : t("settings.about.edition.installer")}
      </div>
    </div>
    <div class="rounded-xl bg-surface/50 px-4 py-3">
      <div class="text-azure-900/50">{t("settings.about.version.label")}</div>
      <div class="font-semibold">{appInfo?.version ?? "…"}</div>
    </div>
    <div class="col-span-2 rounded-xl bg-surface/50 px-4 py-3 max-lg:col-span-1">
      <div class="text-azure-900/50">{t("settings.about.data_dir.label")}</div>
      <div class="break-all font-semibold">
        {appInfo?.data_dir ?? report?.data_dir ?? "…"}
      </div>
    </div>
  </section>
</div>

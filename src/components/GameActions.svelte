<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import ActionButton from "./ActionButton.svelte";
  import Icon from "./Icons.svelte";
  import { appState } from "../lib/app-state.svelte";
  import {
    copyToSteam,
    installGameViaSteam,
    launchGame,
    removeLibraryEntry,
    removeLuaFromSteam,
    setDefaultArchivePassword,
    shouldRememberArchivePassword,
    uninstallOnlineFix,
  } from "../lib/api";
  import type { GameStatus } from "../lib/api";
  import { installFixWithRepair } from "../lib/defender";
  import { focusTrap } from "../lib/focus-trap";
  import { t } from "../lib/i18n.svelte";

  let {
    status,
    /** Full-width primary action — the library card uses it, the spotlight doesn't. */
    full = false,
    /** Called after each action so a parent holding its own copy can refresh it. */
    onAfterAction,
  }: {
    status: GameStatus;
    full?: boolean;
    onAfterAction?: () => Promise<void> | void;
  } = $props();

  const fix = $derived(status.fix);
  /** True while the stage itself already speaks about the patch. */
  const stageIsAboutFix = $derived(status.stage.startsWith("fix_"));

  /** Only one action runs at a time, so the label can say which. */
  let busy = $state<string | null>(null);
  let passwordAction = $state<"install" | null>(null);
  let archivePassword = $state("");

  async function run(
    id: string,
    action: () => Promise<string | null>,
    retryWithPassword?: "install",
    submittedArchivePassword?: string | null,
  ) {
    busy = id;
    try {
      const message = await action();
      await appState.refreshLibrary();
      if (shouldRememberArchivePassword(
        submittedArchivePassword,
        appState.report?.default_archive_password,
      )) {
        // A failed password must never replace the saved default: this branch
        // is reached only after the archive operation itself has succeeded.
        await setDefaultArchivePassword(submittedArchivePassword ?? null);
        await appState.refresh();
      }
      if (message) appState.toast("success", message);
      await onAfterAction?.();
    } catch (e) {
      if (retryWithPassword && String(e).includes("PasswordIncorrect:")) {
        archivePassword = appState.report?.default_archive_password ?? "";
        passwordAction = retryWithPassword;
      } else {
        appState.toast("error", String(e));
      }
    } finally {
      busy = null;
    }
  }

  const copyLua = () =>
    run("copy", async () => {
      await copyToSteam(status.app_id);
      return t("actions.toast.copy-lua", { name: status.name });
    });

  const steamInstall = () => run("steam", async () => await installGameViaSteam(status.app_id));

  const play = () => run("play", async () => await launchGame(status.app_id));

  const applyFix = (password?: string | null) =>
    run("fix-install", async () => {
      const report = await installFixWithRepair(status.app_id, password);
      return report.health === "healthy"
        ? t("actions.toast.install-fix", { name: status.name, count: report.file_count })
        : t("actions.toast.install-fix-unhealthy", { name: status.name });
    }, "install", password);

  function closePasswordDialog() {
    passwordAction = null;
    archivePassword = "";
  }

  function submitPassword() {
    const action = passwordAction;
    const password = archivePassword;
    closePasswordDialog();
    if (action === "install") void applyFix(password);
  }

  const removeFix = () =>
    run("fix-remove", async () => {
      const report = await uninstallOnlineFix(status.app_id);
      return t("actions.toast.remove-fix", { removed: report.removed, restored: report.restored });
    });

  const unlink = () =>
    run("unlink", async () => {
      const removed = await removeLuaFromSteam(status.app_id);
      return removed
        ? t("actions.toast.remove-lua", { name: status.name })
        : t("actions.toast.remove-lua-none");
    });

  const forget = () =>
    run("forget", async () => {
      await removeLibraryEntry(status.app_id);
      return t("actions.toast.remove", { name: status.name });
    });

  function reveal() {
    const dir = fix.game_dir ?? status.game.install_dir;
    if (dir) void revealItemInDir(dir);
  }
</script>

<div class="flex flex-col gap-1.5">
  <!-- Primary action: exactly one, matching the current stage. -->
  {#if !status.in_library}
    <div
      class="flex items-center justify-center gap-2 rounded-xl border border-white/70 bg-white/60 px-3 py-1.5 text-xs font-semibold text-azure-900/60"
    >
      <Icon name="info" size={13} />
      {t("stage.no_lua.tip")}
    </div>
  {:else if status.stage === "lua_not_in_steam"}
    <ActionButton
      label={t("actions.copy-lua.label")}
      icon="copy"
      variant="primary"
      size="sm"
      {full}
      busy={busy === "copy"}
      busyLabel={t("actions.copy-lua.busy")}
      disabled={busy !== null}
      onclick={copyLua}
      tip={t("actions.copy-lua.tip")}
    />
  {:else if status.stage === "needs_steam_install"}
    <ActionButton
      label={t("actions.steam-install.label")}
      icon="steam"
      variant="primary"
      size="sm"
      {full}
      busy={busy === "steam"}
      busyLabel={t("actions.steam-install.busy")}
      disabled={busy !== null}
      onclick={steamInstall}
      tip={t("actions.steam-install.tip")}
    />
  {:else if status.stage === "fix_downloaded"}
    <ActionButton
      label={t("actions.install-fix.label")}
      icon="patch"
      variant="primary"
      size="sm"
      {full}
      busy={busy === "fix-install"}
      busyLabel={t("actions.install-fix.busy")}
      disabled={busy !== null}
      onclick={applyFix}
      tip={t("actions.install-fix.tip")}
    />
  {:else if status.stage === "fix_damaged" || status.stage === "fix_game_moved"}
    <ActionButton
      label={t("actions.repair-fix.label")}
      icon="wrench"
      variant="primary"
      size="sm"
      {full}
      busy={busy === "fix-install"}
      busyLabel={t("actions.repair-fix.busy")}
      disabled={busy !== null}
      onclick={applyFix}
      tip={t("actions.repair-fix.tip")}
    />
  {:else if status.stage === "installing"}
    <div
      class="flex items-center justify-center gap-2 rounded-xl border border-lilac/25 bg-lilac-soft/50 px-3 py-1.5 text-xs font-semibold text-lilac-deep"
    >
      <Icon name="clock" size={13} />
      {t("actions.installing")}
    </div>
  {:else}
    <ActionButton
      label={t("actions.play.label")}
      icon="play"
      variant="soft"
      size="sm"
      {full}
      busy={busy === "play"}
      busyLabel={t("actions.play.busy")}
      disabled={busy !== null}
      onclick={play}
      tip={t("actions.play.tip")}
    />
  {/if}

  <!-- Secondary actions, always available but visually quieter. -->
  <div class="flex flex-wrap gap-1.5">
    {#if status.stage === "fix_installed"}
      <ActionButton
        label={t("actions.remove-fix.label")}
        icon="broom"
        variant="danger"
        size="sm"
        busy={busy === "fix-remove"}
        busyLabel={t("actions.remove-fix.busy")}
        disabled={busy !== null}
        onclick={removeFix}
        tip={t("actions.remove-fix.tip")}
      />
    {/if}
    {#if status.fix_downloaded && !stageIsAboutFix}
      <ActionButton
        label={t("actions.downloaded-fix.label")}
        icon="patch"
        size="sm"
        disabled={busy !== null || !status.game.fully_installed}
        busy={busy === "fix-install"}
        busyLabel="…"
        onclick={applyFix}
        tip={status.game.fully_installed
          ? t("actions.downloaded-fix.tip")
          : t("actions.downloaded-fix.not-installed-tip")}
      />
    {/if}
    {#if status.lua_in_steam}
      <ActionButton
        label={t("actions.remove-lua.label")}
        icon="x"
        size="sm"
        busy={busy === "unlink"}
        busyLabel="…"
        disabled={busy !== null}
        onclick={unlink}
        tip={t("actions.remove-lua.tip")}
      />
    {/if}
    {#if status.game.install_dir}
      <ActionButton
        label={t("actions.open-folder.label")}
        icon="folder"
        size="sm"
        onclick={reveal}
        tip={status.game.install_dir}
      />
    {/if}
    {#if status.in_library}
      <ActionButton
        label={t("actions.remove.label")}
        icon="trash"
        variant="danger"
        size="sm"
        busy={busy === "forget"}
        busyLabel="…"
        disabled={busy !== null}
        onclick={forget}
        tip={t("actions.remove.tip")}
      />
    {/if}
  </div>
</div>

{#if passwordAction}
  <!-- svelte-ignore a11y_click_events_have_key_events backdrop: Escape closes the dialog. -->
  <div class="fixed inset-0 z-[300] lv-veil" role="presentation" onclick={closePasswordDialog}></div>
  <div
    class="glass-strong enter-fade fixed z-[301] w-full max-w-md rounded-xl border border-surface/60 p-5 shadow-xl"
    style="top: 50%; left: 50%; transform: translate(-50%, -50%);"
    use:focusTrap
    role="dialog"
    aria-modal="true"
    aria-labelledby="archive-password-title"
    tabindex="-1"
    onkeydown={(event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closePasswordDialog();
      } else if (event.key === "Enter") {
        event.preventDefault();
        submitPassword();
      }
    }}
  >
    <h3 id="archive-password-title" class="mb-3 text-base font-semibold">
      {t("actions.password-dialog.title")}
    </h3>
    <p class="mb-4 text-sm text-azure-900/70">{t("actions.password-dialog.hint")}</p>
    <label for="archive-password" class="mb-2 block text-sm">{t("actions.password-dialog.label")}</label>
    <input
      id="archive-password"
      data-autofocus
      type="password"
      bind:value={archivePassword}
      autocomplete="off"
      class="w-full rounded-lg border border-surface/65 bg-surface px-3 py-1.5 text-sm outline-none placeholder-azure-400 focus:border-azure-500"
      placeholder={t("actions.password-dialog.placeholder")}
    />
    <div class="mt-4 flex justify-end gap-2">
      <button class="rounded-lg px-3 py-1.5 text-sm" onclick={closePasswordDialog}>
        {t("actions.password-dialog.cancel")}
      </button>
      <button class="rounded-lg bg-lilac px-3 py-1.5 text-sm font-semibold text-white" onclick={submitPassword}>
        {t("actions.password-dialog.submit")}
      </button>
    </div>
  </div>
{/if}

<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Icon from "../components/Icons.svelte";
  import ActionButton from "../components/ActionButton.svelte";
  import ConfirmButton from "../components/ConfirmButton.svelte";
  import StatusBadge from "../components/StatusBadge.svelte";
  import { appState } from "../lib/app-state.svelte";
  import { installSteam, installSteamtools, restartSteam, setSteamDir } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  import { openFolder } from "../lib/open-folder";

  const report = $derived(appState.report);
  const steam = $derived(report?.steam ?? null);
  const st = $derived(report?.steamtools ?? null);
  let busy = $state<string | null>(null);

  const checks = $derived(
    st
      ? [
          { ok: st.has_open_steam_tool, label: "OpenSteamTool.dll" },
          { ok: st.has_xinput, label: "xinput1_4.dll" },
          { ok: st.lua_dir_exists, label: t("tools.marker.lua_dir") },
        ]
      : [],
  );

  async function run(id: string, action: () => Promise<string>) {
    busy = id;
    try {
      appState.toast("info", await action());
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function pickSteamFolder() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    await run("pick", async () => {
      appState.report = await setSteamDir(sel);
      await appState.refreshStatuses();
      return t("tools.steam.saved");
    });
  }

  const doInstallSteam = () => run("steam", installSteam);
  const doRestartSteam = () => run("restart", restartSteam);

  async function doInstallSteamtools() {
    await run("st", installSteamtools);
    // The script closes Steam and swaps DLLs — give it a moment before re-reading.
    await new Promise((r) => setTimeout(r, 3000));
    await appState.refresh();
  }
</script>

<div class="flex h-full flex-col gap-4 p-1">
  <header class="glass enter-up flex flex-wrap items-center gap-3 rounded-xl2 p-5">
    <div class="min-w-0 flex-1">
      <h2 class="flex items-center gap-2 text-lg font-semibold">
        <Icon name="tools" size={20} />
        {t("nav.tools")}
      </h2>
      <p class="mt-0.5 text-sm text-azure-900/60">
        {t("tools.subtitle")}
      </p>
    </div>
    <ActionButton
      label={t("tools.reread.label")}
      icon="refresh"
      onclick={() => appState.refresh()}
      tip={t("tools.reread.tip")}
    />
  </header>

  <div class="grid min-h-0 flex-1 grid-cols-2 gap-4 overflow-y-auto max-xl:grid-cols-1">
    <!-- Steam -->
    <div class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
      <div class="flex flex-wrap items-center gap-2">
        <h3 class="flex items-center gap-2 font-semibold">
          <Icon name="steam" size={18} />
          Steam
        </h3>
        <StatusBadge
          label={steam ? t("tools.steam.detected") : t("tools.steam.missing")}
          icon={steam ? "check" : "alert"}
          tone={steam ? "good" : "action"}
          compact
          tip={steam ? steam.source : t("tools.steam.install_tip")}
        />
      </div>

      {#if steam}
        <p class="break-all rounded-xl bg-surface/55 px-4 py-3 text-sm">{steam.path}</p>
        <div class="mt-auto flex flex-wrap gap-2">
          <ActionButton
            label={t("tools.open_folder.label")}
            icon="folder"
            onclick={() => void openFolder(steam.path)}
          />
          <ActionButton
            label={t("tools.change_folder.label")}
            icon="edit"
            disabled={busy !== null}
            onclick={pickSteamFolder}
          />
          <ActionButton
            label={t("tools.restart.label")}
            icon="refresh"
            disabled={busy !== null}
            busy={busy === "restart"}
            busyLabel={t("tools.restart.busy")}
            onclick={doRestartSteam}
            tip={t("tools.restart.tip")}
          />
          <ConfirmButton
            label={t("tools.reinstall.label")}
            confirmLabel={t("tools.reinstall.confirm")}
            onconfirm={doInstallSteam}
            title={t("tools.reinstall.title")}
          />
        </div>
      {:else}
        <p class="text-sm text-azure-900/60">
          {t("tools.steam.not_found")}
        </p>
        <div class="mt-auto flex flex-wrap gap-2">
          <ActionButton
            label={t("tools.install_steam.label")}
            icon="download"
            variant="primary"
            disabled={busy !== null}
            busy={busy === "steam"}
            busyLabel={t("tools.install_steam.busy")}
            onclick={doInstallSteam}
          />
          <ActionButton
            label={t("tools.choose_folder.label")}
            icon="edit"
            disabled={busy !== null}
            onclick={pickSteamFolder}
          />
        </div>
      {/if}
    </div>

    <!-- SteamTools -->
    <div class="glass enter-up flex flex-col gap-4 rounded-xl2 p-5">
      <div class="flex flex-wrap items-center gap-2">
        <h3 class="flex items-center gap-2 font-semibold">
          <Icon name="tools" size={18} />
          SteamTools
        </h3>
        {#if st?.installed}
          <StatusBadge
            label={t("tools.steamtools.installed")}
            icon="check"
            tone="good"
            compact
            tip={t("tools.steamtools.dlls_tip")}
          />
        {:else if steam}
          <StatusBadge
            label={t("tools.steamtools.absent")}
            icon="alert"
            tone="action"
            compact
            tip={t("tools.steamtools.absent_tip")}
          />
        {/if}
      </div>

      {#if !steam}
        <p class="text-sm text-azure-900/60">{t("tools.steamtools.steam_required")}</p>
      {:else if st}
        <ul class="flex flex-col gap-1.5 text-sm">
          {#each checks as check (check.label)}
            <li
              class="flex items-center gap-2 rounded-lg px-3 py-2 {check.ok
                ? 'bg-mint-soft/45 text-mint-deep'
                : 'bg-rose-soft/45 text-rose-deep'}"
            >
              <Icon name={check.ok ? "check" : "error"} size={15} />
              {check.label}
            </li>
          {/each}
          {#if st.legacy_plugin_dir}
            <li
              class="flex items-start gap-2 rounded-lg bg-peach-soft/55 px-3 py-2 text-peach-deep"
            >
              <Icon name="alert" size={15} class="mt-0.5 shrink-0" />
              {t("tools.steamtools.legacy_plugin")}
            </li>
          {/if}
        </ul>

        {#if st.conflicts.length > 0}
          <div class="rounded-xl border border-rose/30 bg-rose-soft/55 px-4 py-3 text-sm text-rose-deep">
            <p class="flex items-center gap-2 font-semibold">
              <Icon name="error" size={15} />
              {t("tools.conflicts.title")}
            </p>
            <ul class="mt-1 list-inside list-disc">
              {#each st.conflicts as conflict (conflict)}
                <li class="break-all">{conflict}</li>
              {/each}
            </ul>
            <p class="mt-1 text-xs">
              {t("tools.conflicts.action")}
            </p>
          </div>
        {/if}

        <div class="mt-auto flex flex-wrap gap-2">
          <ConfirmButton
            label={st.installed ? t("tools.steamtools.repair") : t("tools.steamtools.install")}
            confirmLabel={t("tools.steamtools.confirm")}
            onconfirm={doInstallSteamtools}
            primary
            title={t("tools.steamtools.title")}
          />
        </div>
        <p class="text-xs text-azure-900/40">
          {t("tools.steamtools.command")}
        </p>
      {/if}
    </div>
  </div>
</div>

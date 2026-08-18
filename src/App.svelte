<script lang="ts">
  import { onMount, tick } from "svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import LibraryView from "./views/LibraryView.svelte";
  import NewsView from "./views/NewsView.svelte";
  import StatsView from "./views/StatsView.svelte";
  import ToolsView from "./views/ToolsView.svelte";
  import SettingsView from "./views/SettingsView.svelte";
  import LogView from "./views/LogView.svelte";
  import CreditsView from "./views/CreditsView.svelte";
  import Onboarding from "./components/Onboarding.svelte";
  import Toasts from "./components/Toasts.svelte";
  import GameSpotlight from "./components/GameSpotlight.svelte";
  import Icon from "./components/Icons.svelte";
  import UpdateModal from "./components/UpdateModal.svelte";
  import { appState } from "./lib/app-state.svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    defenderStatus,
    setDefenderChoice,
    setupDefenderExclusions,
    type Reachability,
  } from "./lib/api";
  import { mountTooltips } from "./lib/tooltip";
  import { originOf, themeStore } from "./lib/theme.svelte";
  import { resolveShortcut, isEditableTarget } from "./lib/keyboard-shortcuts";
  import { focusTrap } from "./lib/focus-trap";
  import { t } from "./lib/i18n.svelte";

  type View =
    | "library"
    | "news"
    | "stats"
    | "tools"
    | "logs"
    | "credits"
    | "settings";
  let view = $state<View>("library");
  let showHelp = $state(false);
  let showUpdateModal = $state(false);

  function handleKeydown(e: KeyboardEvent): void {
    // Ignore when a modal surface is open
    if (document.querySelector('[aria-modal="true"]') !== null) return;

    const editable = isEditableTarget(e.target);
    const action = resolveShortcut({ key: e.key, ctrlKey: e.ctrlKey, altKey: e.altKey, editable });

    if (action === "open-logs") {
      e.preventDefault();
      view = "logs";
    } else if (action === "open-help") {
      e.preventDefault();
      showHelp = true;
    }
  }

  function handleHelpEscape(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      showHelp = false;
    }
  }

  /** Whether Windows Defender is the active antivirus (and thus manageable). */
  let defenderAvailable = $state(false);
  /** Guards the one-time exclusion prompt against re-entry. */
  let defenderPrompted = false;

  onMount(() => {
    const unmountTooltips = mountTooltips();
    void appState.refreshReachability();
    const unlistenReachability = listen<Reachability>("reachability://changed", (event) => {
      appState.setReachability(event.payload);
    });
    void (async () => {
      await appState.refresh();
      // Check whether an update just completed.
      await appState.checkUpdateResult();
      // Update check FIRST, and deliberately before `adoptFromSteam`. Adoption
      // walks the whole Steam library and calls the network per game — measured
      // at nearly a minute on a real install. Behind it, the update button only
      // appeared after that minute, so the update looked undetected and the user
      // went to Settings to press "check now". Nothing here depends on adoption,
      // so it starts now and resolves whenever it resolves.
      void appState.checkForUpdate();
      // Games already sitting in {Steam}\config\lua join the library on their own.
      await appState.adoptFromSteam();
      try {
        defenderAvailable = (await defenderStatus()).available;
      } catch {
        defenderAvailable = false;
      }
    })();

    // Steam installs and downloads finish outside the app; keep states honest.
    const timer = setInterval(() => void appState.refreshStatuses(), 20_000);
    return () => {
      clearInterval(timer);
      unmountTooltips();
      void unlistenReachability.then((fn) => fn());
    };
  });

  const report = $derived(appState.report);
  const showOnboarding = $derived(
    report !== null && !report.first_run_done,
  );
  const attention = $derived(appState.needsAttention);

  // One-time Defender exclusion prompt, right after onboarding.
  // Excluding the Steam games folder once lets every online-fix install run
  // without the antivirus deleting the patch — and without re-asking.
  $effect(() => {
    if (defenderPrompted) return;
    const ready = report !== null && !showOnboarding;
    if (!ready || !defenderAvailable) return;
    if (report?.defender_exclusions != null) return; // already answered
    if (!report?.steam) return; // no Steam → nothing to exclude
    defenderPrompted = true;
    void promptDefenderSetup();
  });

  async function promptDefenderSetup() {
    const ok = await confirm(
      t("shell.defender.body"),
      {
        title: t("shell.defender.title"),
        kind: "warning",
        okLabel: t("shell.defender.ok"),
        cancelLabel: t("shell.defender.cancel"),
      },
    );
    try {
      if (ok) {
        await setupDefenderExclusions();
        appState.toast("success", t("shell.defender.added"));
      } else {
        await setDefenderChoice(false);
      }
    } catch (e) {
      appState.toast("error", String(e));
    }
    await appState.refresh();
  }

  const steamState = $derived(report === null ? "loading" : report.steam ? "ok" : "missing");
  const stState = $derived(
    report === null
      ? "loading"
      : report.steamtools?.installed
        ? "ok"
        : report.steam
          ? "missing"
          : "loading",
  );

  const DOT: Record<string, string> = {
    ok: "bg-mint",
    missing: "bg-peach",
    loading: "bg-azure-200",
  };

  const nav = $derived(
    [
      { id: "library", label: t("nav.library"), icon: "library" },
      { id: "news", label: t("nav.news"), icon: "news" },
      { id: "stats", label: t("nav.stats"), icon: "layers" },
      { id: "tools", label: t("nav.tools"), icon: "tools" },
      { id: "logs", label: t("nav.logs"), icon: "logs" },
      { id: "credits", label: t("nav.credits"), icon: "sparkle" },
      { id: "settings", label: t("nav.settings"), icon: "settings" },
    ] as { id: View; label: string; icon: string }[],
  );
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="flex h-full gap-4 p-4">
  <aside class="glass flex w-60 shrink-0 flex-col rounded-xl2 p-4 max-lg:w-16">
    <div class="mb-6 flex items-center gap-3 max-lg:justify-center">
      <div
        class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-azure-400 to-azure-600 text-sm font-bold text-white shadow-md"
      >LV</div>
      <div class="max-lg:hidden">
        <h1 class="text-sm leading-tight font-semibold">LuaVault</h1>
        <p class="text-xs text-azure-700/70">{t("shell.brand.tagline")}</p>
      </div>
    </div>

    <nav class="flex flex-1 flex-col gap-1.5">
      {#each nav as item (item.id)}
        <button
          onclick={() => (view = item.id)}
          data-tip={item.label}
          class="lift relative flex items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm font-medium max-lg:justify-center max-lg:px-0 {view ===
          item.id
            ? 'bg-surface/80 text-azure-800 shadow-sm'
            : 'text-azure-900/60 hover:bg-surface/50'}"
        >
          {#if view === item.id}
            <span
              class="absolute top-1/2 left-0 h-6 w-1 -translate-y-1/2 rounded-r-full bg-azure-500"
            ></span>
          {/if}
          <Icon name={item.icon} size={18} />
          <span class="max-lg:hidden">{item.label}</span>
          {#if item.id === "library" && appState.library.length > 0}
            <span class="ml-auto flex items-center gap-1 max-lg:hidden">
              {#if attention > 0}
                <span
                  data-tip={t("shell.attention.tip", { n: attention })}
                  class="rounded-full bg-peach-soft px-2 py-0.5 text-xs font-bold text-peach-deep"
                >
                  {attention}
                </span>
              {/if}
              <span
                class="rounded-full bg-azure-500/15 px-2 py-0.5 text-xs font-semibold text-azure-700"
              >
                {appState.library.length}
              </span>
            </span>
          {:else if item.id === "library" && attention > 0}
            <span class="pulse-dot absolute top-1.5 right-1.5 h-2 w-2 rounded-full bg-peach"></span>
          {/if}
        </button>
      {/each}
    </nav>

    <div class="mt-4 flex flex-col gap-2 border-t border-surface/60 pt-4">
      <button
        onclick={(event) => themeStore.toggleDark(originOf(event))}
        data-tip={themeStore.dark ? t("shell.theme.to-light") : t("shell.theme.to-dark")}
        class="lift flex items-center gap-2 rounded-lg bg-surface/45 px-2.5 py-1.5 text-xs hover:bg-surface/70 max-lg:justify-center max-lg:px-0"
      >
        <Icon name={themeStore.dark ? "moon" : "sun"} size={14} />
        <span class="max-lg:hidden">{themeStore.dark ? t("shell.theme.dark") : t("shell.theme.light")}</span>
      </button>
      <button
        onclick={() => (view = "tools")}
        data-tip={steamState === "ok" ? t("shell.steam.detected") : t("shell.steam.missing")}
        class="lift flex items-center gap-2 rounded-lg bg-surface/45 px-2.5 py-1.5 text-xs hover:bg-surface/70 max-lg:justify-center max-lg:px-0"
      >
        <span class="h-2 w-2 shrink-0 rounded-full {DOT[steamState]}"></span>
        <span class="max-lg:hidden">Steam</span>
      </button>
      <button
        onclick={() => (view = "tools")}
        data-tip={stState === "ok" ? t("shell.steamtools.installed") : t("shell.steamtools.missing")}
        class="lift flex items-center gap-2 rounded-lg bg-surface/45 px-2.5 py-1.5 text-xs hover:bg-surface/70 max-lg:justify-center max-lg:px-0"
      >
        <span class="h-2 w-2 shrink-0 rounded-full {DOT[stState]}"></span>
        <span class="max-lg:hidden">SteamTools</span>
      </button>
      {#if appState.updateAvailable}
        <button
          onclick={() => (showUpdateModal = true)}
          data-tip={t("shell.update.tip", { version: appState.updateAvailable.version })}
          class="lift flex items-center gap-2 rounded-lg bg-mint px-2.5 py-1.5 text-xs font-medium text-white hover:bg-mint/80 max-lg:justify-center max-lg:px-0"
        >
          <Icon name="update" size={14} />
          <span class="max-lg:hidden">{appState.updateAvailable.version}</span>
        </button>
      {/if}
      {#if !appState.online}
        <button
          type="button"
          onclick={() => void appState.refreshReachability()}
          data-tip={appState.offlineTip ?? t("shell.offline.tip")}
          class="lift flex items-center gap-2 rounded-lg bg-peach-soft px-2.5 py-1.5 text-xs text-peach-deep max-lg:justify-center max-lg:px-0"
        >
          <Icon name="wifi-off" size={14} />
          <span class="max-lg:hidden">{t("shell.offline.pill")}</span>
        </button>
      {/if}
      <button
        onclick={() => (showHelp = true)}
        data-tip={t("shell.help.title")}
        class="lift flex items-center gap-2 rounded-lg bg-surface/45 px-2.5 py-1.5 text-xs hover:bg-surface/70 max-lg:justify-center max-lg:px-0"
      >
        <Icon name="info" size={14} />
        <span class="max-lg:hidden">{t("shell.help.title")}</span>
      </button>
    </div>
  </aside>

  <main class="min-w-0 flex-1 overflow-hidden rounded-xl2">
    {#key view}
      <div class="enter-fade h-full">
        {#if view === "library"}
          <LibraryView />
        {:else if view === "news"}
          <NewsView />
        {:else if view === "stats"}
          <StatsView />
        {:else if view === "tools"}
          <ToolsView />
        {:else if view === "logs"}
          <LogView />
        {:else if view === "credits"}
          <CreditsView />
        {:else}
          <SettingsView />
        {/if}
      </div>
    {/key}
  </main>
</div>

{#if showOnboarding}
  <Onboarding />
{/if}

<Toasts />
<GameSpotlight />

{#if showHelp}
  <div
    class="enter-fade fixed inset-0 z-[300] flex items-center justify-center lv-veil"
    role="presentation"
    onclick={(e) => { if (e.target === e.currentTarget) showHelp = false; }}
  >
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="help-dialog-title"
      tabindex="-1"
      onkeydown={handleHelpEscape}
      use:focusTrap
      class="glass enter-fade max-w-lg rounded-xl2 p-6 shadow-xl"
    >
      <h2 id="help-dialog-title" class="text-lg font-semibold mb-4">{t("shell.help.title")}</h2>
      <ul class="space-y-2 text-sm">
        <li class="flex items-center gap-3">
          <kbd class="rounded bg-surface/60 px-2 py-0.5 font-mono text-xs">Ctrl</kbd>+<kbd class="rounded bg-surface/60 px-2 py-0.5 font-mono text-xs">L</kbd>
          <span class="text-azure-900/60">{t("shell.help.logs")}</span>
        </li>
        <li class="flex items-center gap-3">
          <kbd class="rounded bg-surface/60 px-2 py-0.5 font-mono text-xs">?</kbd>
          <span class="text-azure-900/60">{t("shell.help.help")}</span>
        </li>
        <li class="flex items-center gap-3">
          <kbd class="rounded bg-surface/60 px-2 py-0.5 font-mono text-xs">{t("shell.help.key.escape")}</kbd>
          <span class="text-azure-900/60">{t("shell.help.escape")}</span>
        </li>
      </ul>
      <div class="mt-6 flex justify-end">
        <button
          onclick={() => (showHelp = false)}
          class="rounded-lg bg-surface/60 px-3 py-1.5 text-xs font-medium hover:bg-surface/80"
        >
          {t("shell.help.close")}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if showUpdateModal && appState.updateAvailable}
  <UpdateModal
    update={appState.updateAvailable}
    onclose={() => (showUpdateModal = false)}
  />
{/if}



<script lang="ts">
  import { appState } from "../lib/app-state.svelte";
  import { getLogDir } from "../lib/api";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import Icon from "../components/Icons.svelte";
  import ActionButton from "../components/ActionButton.svelte";
  import ConfirmButton from "../components/ConfirmButton.svelte";
  import { logsState } from "../lib/logs-state.svelte";
  import { t } from "../lib/i18n.svelte";
  import type { LogLevelFilter } from "../lib/log-filter";
  import {
    LOG_DISPLAY_LIMIT,
    displayLogs,
    filterLogs,
    formatLogText,
    CLIPBOARD_TIMEOUT_MS,
    isFilterActive,
    levelColor,
    levelName,
    withTimeout,
  } from "../lib/log-filter";

  const allLogs = $derived(appState.logs);
  // LOT-18 : filtrer et chercher d'abord, tronquer pour l'affichage ensuite.
  // L'inverse rendrait introuvable une entrée que la limite d'affichage
  // exclut — la recherche jurerait qu'elle n'existe pas.
  const filtered = $derived(filterLogs(allLogs, logsState.level, logsState.search));
  const logs = $derived(displayLogs(allLogs, logsState.level, logsState.search, LOG_DISPLAY_LIMIT));
  /**
   * The header count, honest about all three numbers: how many entries
   * match the filter, how many exist in the buffer, and — when the display
   * limit kicks in — that only the last `shown` of the matches are on
   * screen. Announcing the buffer size while a filter hides most of it is
   * the lying-counter trap this project keeps paying for.
   */
  const countLabel = $derived.by(() => {
    const n = filtered.length;
    const total = allLogs.length;
    const shown = logs.length;
    if (shown < n) {
      return n === total
        ? t("logs.count.all-truncated", { n, shown })
        : t("logs.count.filtered-truncated", { n, total, shown });
    }
    return n === total ? t("logs.count.all", { n }) : t("logs.count.filtered", { n, total });
  });
  let logDir = $state<string | null>(null);
  let container: HTMLDivElement | undefined = $state();
  // Vrai tant que le conteneur est en bas : une nouvelle entrée n'a le droit
  // de tirer la vue vers le bas que dans ce cas. En train de lire plus haut,
  // la vue ne bouge plus sous les yeux.
  let nearBottom = $state(true);

  $effect(() => {
    void getLogDir()
      .then((d) => (logDir = d))
      .catch(() => {});
  });

  // Auto-scroll : rejoué à chaque changement de la liste RENDUE (filtrée),
  // et seulement si l'utilisateur est déjà en bas. La dépendance est la
  // liste elle-même, pas un compte : à tampon plein, chaque nouvelle entrée
  // en chasse une et la longueur ne change plus — un tick égal à la longueur
  // ne notifiait plus rien et la vue dérivait hors du bas sans rattrapage.
  // Quand la liste ne déborde plus (recherche qui la fait tenir dans
  // l'écran), aucun événement scroll n'est émis : nearBottom est réarmé
  // ici, sinon il resterait false sans ascenseur pour le recalculer.
  $effect(() => {
    logs.length; // la vraie dépendance : la liste rendue
    if (!container) return;
    if (container.scrollHeight - container.clientHeight <= 0) nearBottom = true;
    if (logsState.autoScroll && nearBottom) container.scrollTop = container.scrollHeight;
  });

  function onScroll() {
    if (!container) return;
    nearBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 48;
  }

  /** Chip count: what that level would show under the current search. */
  function countFor(id: LogLevelFilter): number {
    return filterLogs(allLogs, id, logsState.search).length;
  }

  async function writeClipboard(text: string, count: number, scope: "displayed" | "all") {
    try {
      await withTimeout(
        navigator.clipboard.writeText(text),
        CLIPBOARD_TIMEOUT_MS,
        t("logs.clipboard.timeout"),
      );
      appState.toast("success", scope === "displayed"
        ? t("logs.copied.displayed", { n: count })
        : t("logs.copied.all", { n: count }));
    } catch (e) {
      appState.toast("error", t("logs.copy.error", { error: String(e) }));
    }
  }

  function copyDisplayed() {
    void writeClipboard(formatLogText(logs), logs.length, "displayed");
  }

  function copyAll() {
    void writeClipboard(formatLogText(allLogs), allLogs.length, "all");
  }

  const LEVEL_FILTERS: { id: LogLevelFilter; icon: string }[] = [
    { id: "all", icon: "logs" },
    { id: "error", icon: "error" },
    { id: "warn", icon: "alert" },
    { id: "info", icon: "info" },
    { id: "other", icon: "more" },
  ];
</script>

<div class="flex h-full flex-col gap-4 p-1">
  <header class="glass enter-up flex flex-col gap-3 rounded-xl2 p-5">
    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <h2 class="flex items-center gap-2 text-lg font-semibold">
          <Icon name="logs" size={20} />
          {t("logs.title")}
        </h2>
        <p class="mt-0.5 truncate text-sm text-azure-900/60">
          {countLabel}
          {#if logDir}— {logDir}{/if}
        </p>
      </div>
      <label
        class="flex cursor-pointer items-center gap-2 text-sm text-azure-800"
        data-tip={t("logs.autoscroll.tip")}
      >
        <input type="checkbox" bind:checked={logsState.autoScroll} class="accent-azure-500" />
        {t("logs.autoscroll")}
      </label>
      {#if logDir}
        <ActionButton
          label={t("logs.dir")}
          icon="folder"
          onclick={() => void revealItemInDir(logDir!)}
        />
      {/if}
      <ActionButton
        label={t("logs.copy")}
        icon="copy"
        disabled={logs.length === 0}
        onclick={copyDisplayed}
        tip={t("logs.copy.tip", { limit: LOG_DISPLAY_LIMIT })}
      />
      <ActionButton
        label={t("logs.copy-all")}
        icon="copy"
        disabled={allLogs.length === 0}
        onclick={copyAll}
        tip={t("logs.copy-all.tip", { n: allLogs.length })}
      />
      <ConfirmButton
        label={t("logs.clear")}
        icon="broom"
        confirmLabel={t("logs.clear.confirm")}
        title={t("logs.clear.tip")}
        onconfirm={() => {
          appState.logs = [];
        }}
      />
    </div>
    <div class="relative">
      <Icon
        name="search"
        size={15}
        class="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 opacity-50"
      />
      <input
        type="text"
        bind:value={logsState.search}
        placeholder={t("logs.search.placeholder")}
        class="w-full rounded-xl border border-surface/70 bg-surface/60 py-2 pr-9 pl-9 text-sm outline-none backdrop-blur-md transition select-text placeholder:text-azure-900/40 focus:border-azure-300 focus:bg-surface/85 focus:ring-2 focus:ring-azure-300/40"
      />
      {#if logsState.search}
        <button
          onclick={() => (logsState.search = "")}
          aria-label={t("logs.search.clear-aria")}
          class="lift absolute top-1/2 right-2 -translate-y-1/2 rounded-md p-1 text-azure-900/45 hover:bg-surface/70 hover:text-azure-900/70"
        >
          <Icon name="x" size={13} />
        </button>
      {/if}
    </div>
    <div class="enter-fade flex flex-wrap gap-1.5">
      {#each LEVEL_FILTERS as item (item.id)}
        <button
          onclick={() => (logsState.level = item.id)}
          data-tip={t(`logs.level.${item.id}.tip`)}
          class="lift inline-flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs font-semibold {logsState.level ===
          item.id
            ? 'border-azure-300/60 bg-surface/85 text-azure-800 shadow-sm'
            : 'border-surface/60 bg-surface/45 text-azure-900/55 hover:bg-surface/70'}"
        >
          <Icon name={item.icon} size={13} tone="current" class={levelColor(item.id)} />
          {t(`logs.level.${item.id}`)}
          <span
            class="rounded-full bg-azure-500/12 px-1.5 py-px text-[0.65rem] font-bold text-azure-700"
          >
            {countFor(item.id)}
          </span>
        </button>
      {/each}
    </div>
  </header>

  <div
    class="glass min-h-0 flex-1 overflow-y-auto rounded-xl2 p-4 font-mono text-xs leading-relaxed"
    bind:this={container}
    onscroll={onScroll}
  >
    {#if logs.length === 0}
      <p class="text-azure-900/50">
        {#if isFilterActive(logsState.level, logsState.search)}
          {t("logs.empty.filtered")}
        {:else}
          {t("logs.empty")}
        {/if}
      </p>
    {:else}
      {#each logs as entry (entry.id)}
        <div class="flex gap-2 py-0.5">
          <span class="shrink-0 text-azure-900/40">
            {entry.timestamp?.slice(11, 19) ?? ""}
          </span>
          <span class="w-12 shrink-0 font-semibold {levelColor(entry.level)}">
            {levelName(entry.level).slice(0, 5)}
          </span>
          <span class="min-w-0 break-all whitespace-pre-wrap">{entry.message}</span>
        </div>
      {/each}
    {/if}
  </div>
</div>

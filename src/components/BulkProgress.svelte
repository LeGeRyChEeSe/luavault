<script lang="ts">
  import Icon from "./Icons.svelte";
  import { estimateEta } from "../lib/eta";
  import type { EtaSample } from "../lib/eta";
  import { focusTrap } from "../lib/focus-trap";
  import type { BulkPlan, BulkProgressEvent, BulkReport } from "../lib/api";
  import { t } from "../lib/i18n.svelte";

  let {
    plan,
    phase,
    progress,
    report,
    kind = "install",
    wording = undefined,
    selectionLabel = "",
    onconfirm,
    oncancel,
    onclose,
    onnext,
    nextLabel = t("bulk.next.default"),
  }: {
    plan: BulkPlan;
    phase: "confirm" | "running" | "done";
    progress: BulkProgressEvent[];
    report: BulkReport | null;
    /** Which pass is running — only the wording changes; progress,
        cancellation and the report are the same machinery. */
    kind?: "install" | "repair";
    /** Fifth mode: the selection action supplies its own sentences. When
        set, it wins over `kind` — the machinery underneath is unchanged. */
    wording?: {
      confirm: string;
      running: string;
      done: string;
      interrupted: string;
      ok: string;
    };
    /** Title of the confirmation list for a local selection action. */
    selectionLabel?: string;
    onconfirm: () => void;
    oncancel: () => void;
    onclose: () => void;
    /** When provided, a "next" button appears during the running phase (manual game installs). */
    onnext?: () => void;
    nextLabel?: string;
  } = $props();

  const totalSteps = $derived(
    plan.games.length + plan.fixes.length + plan.selection.length,
  );

  /** De-duplicate progress events: keep the latest status per (phase, app_id). */
  const latest = $derived.by(() => {
    const map = new Map<string, BulkProgressEvent>();
    for (const ev of progress) {
      map.set(`${ev.phase}:${ev.app_id}`, ev);
    }
    return [...map.values()];
  });

  const doneCount = $derived(latest.filter((e) => e.status !== "working").length);
  const currentEvent = $derived(latest.find((e) => e.status === "working"));
  const pct = $derived(totalSteps > 0 ? Math.round((doneCount / totalSteps) * 100) : 0);
  const wasCancelled = $derived(progress.some((e) => e.cancelled));

  const STATUS_ICON: Record<string, string> = {
    ok: "check",
    error: "alert",
    skipped: "clock",
    working: "refresh",
  };

  const STATUS_CLASS: Record<string, string> = {
    ok: "text-mint-deep",
    error: "text-rose-deep",
    skipped: "text-azure-900/40",
    working: "text-lilac-deep",
  };

  /** Arrival time of each (phase, app_id) event — the first sighting wins.
      The component instance lives exactly one run, so the map starts empty. */
  const arrival = new Map<string, number>();

  /** Remaining-time label during the automatic passes; null everywhere else —
      the pure estimator decides what the data can support (src/lib/eta.ts). */
  let etaLabel = $state<string | null>(null);

  $effect(() => {
    const events = progress;
    if (phase !== "running" || events.length === 0) {
      etaLabel = null;
      return;
    }
    const now = performance.now();
    for (const ev of events) {
      const key = `${ev.phase}:${ev.app_id}`;
      if (!arrival.has(key)) arrival.set(key, now);
    }
    const samples: EtaSample[] = events.map((ev) => ({
      phase: ev.phase,
      app_id: ev.app_id,
      status: ev.status,
      total: ev.total,
      cancelled: ev.cancelled,
      at: arrival.get(`${ev.phase}:${ev.app_id}`) ?? now,
    }));
    etaLabel = estimateEta(samples, now).label;
  });

  /** The wording follows the pass — the machinery it describes does not change. */
  const WORD = $derived(
    wording ??
      (kind === "repair"
        ? {
            confirm: t("bulk.repair.confirm"),
            running: t("bulk.repair.running"),
            done: t("bulk.repair.done"),
            interrupted: t("bulk.repair.interrupted"),
            ok: t("bulk.repair.ok"),
          }
        : {
            confirm: t("bulk.install.confirm"),
            running: t("bulk.install.running"),
            done: t("bulk.install.done"),
            interrupted: t("bulk.install.interrupted"),
            ok: t("bulk.install.ok"),
          }),
  );
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="fixed inset-0 z-[300] flex items-center justify-center bg-azure-950/30 backdrop-blur-sm"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget && phase === "done") onclose();
  }}
>
  <div
    class="glass-strong enter-up mx-4 flex max-h-[85vh] w-full max-w-lg flex-col overflow-hidden rounded-2xl border border-surface/60 shadow-2xl"
    use:focusTrap
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-labelledby="bulk-progress-title"
  >
    <!-- Header -->
    <div class="flex items-center gap-3 border-b border-surface/50 px-5 py-4">
      <Icon name={phase === "done" ? "check" : "layers"} size={20} />
      <div class="min-w-0 flex-1">
        <h3 id="bulk-progress-title" class="text-sm font-bold">
          {#if phase === "confirm"}
            {WORD.confirm}
          {:else if phase === "running"}
            {WORD.running}
          {:else}
            {wasCancelled ? WORD.interrupted : WORD.done}
          {/if}
        </h3>
        {#if phase === "running" && currentEvent}
          <p class="truncate text-xs text-azure-900/55">
            {t(`bulk.phase.${currentEvent.phase}`)} · {currentEvent.name} — {currentEvent.detail}
          </p>
        {/if}
      </div>
      {#if phase === "done"}
        <button
          onclick={onclose}
          aria-label={t("bulk.close.aria-label")}
          class="lift rounded-lg p-1.5 text-azure-900/45 hover:bg-surface/70 hover:text-azure-900/70"
        >
          <Icon name="x" size={16} />
        </button>
      {/if}
    </div>

    <!-- Body -->
    <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
      {#if phase === "confirm"}
        <!-- Warnings -->
        {#if plan.warnings.length > 0}
          <div class="mb-4 flex flex-col gap-2">
            {#each plan.warnings as warning}
              <div class="flex items-start gap-2 rounded-xl border border-peach/30 bg-peach-soft/70 px-3 py-2.5 text-xs text-peach-deep">
                <Icon name="alert" size={14} class="mt-px shrink-0" />
                <span>{warning}</span>
              </div>
            {/each}
          </div>
        {/if}

        <!-- Games to install -->
        {#if plan.games.length > 0}
          <p class="mb-2 flex items-center gap-1.5 text-xs font-bold tracking-wide text-azure-900/45 uppercase">
            <Icon name="steam" size={13} />
            {t("bulk.games.title", { count: plan.games.length })}
          </p>
          <div class="mb-4 flex flex-col gap-1">
            {#each plan.games as game (game.app_id)}
              <div class="flex items-start gap-2 rounded-lg px-3 py-2 text-sm {game.warning ? 'bg-peach-soft/50' : 'bg-surface/40'}">
                <Icon name="steam" size={14} class="mt-0.5 shrink-0 text-azure-900/40" />
                <div class="min-w-0">
                  <span class="font-medium">{game.name}</span>
                  <span class="ml-2 text-xs text-azure-900/45">{game.label}</span>
                  {#if game.warning}
                    <p class="mt-0.5 flex items-center gap-1 text-xs text-peach-deep">
                      <Icon name="alert" size={11} />
                      {game.warning}
                    </p>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        {/if}

        <!-- Fixes to apply -->
        {#if plan.fixes.length > 0}
          <p class="mb-2 flex items-center gap-1.5 text-xs font-bold tracking-wide text-azure-900/45 uppercase">
            <Icon name="patch" size={13} />
            {t("bulk.fixes.title", { count: plan.fixes.length })}
          </p>
          <div class="flex flex-col gap-1">
            {#each plan.fixes as fix (fix.app_id)}
              <div class="flex items-start gap-2 rounded-lg bg-surface/40 px-3 py-2 text-sm">
                <Icon name="patch" size={14} class="mt-0.5 shrink-0 text-azure-900/40" />
                <div class="min-w-0">
                  <span class="font-medium">{fix.name}</span>
                  <span class="ml-2 text-xs text-azure-900/45">{fix.label}</span>
                </div>
              </div>
            {/each}
          </div>
        {/if}

        <!-- Fifth mode: a local selection action's list — built by the view
             from the same eligibility the bar's buttons count. -->
        {#if plan.selection.length > 0}
          <p class="mb-2 flex items-center gap-1.5 text-xs font-bold tracking-wide text-azure-900/45 uppercase">
            <Icon name="select" size={13} />
            {selectionLabel || t("bulk.selection.fallback")} ({plan.selection.length})
          </p>
          <div class="flex flex-col gap-1">
            {#each plan.selection as item (item.app_id)}
              <div class="flex items-start gap-2 rounded-lg bg-surface/40 px-3 py-2 text-sm">
                <Icon name="select" size={14} class="mt-0.5 shrink-0 text-azure-900/40" />
                <div class="min-w-0">
                  <span class="font-medium">{item.name}</span>
                  <span class="ml-2 text-xs text-azure-900/45">{item.label}</span>
                </div>
              </div>
            {/each}
          </div>
        {/if}

        {#if plan.games.length === 0 && plan.fixes.length === 0 && plan.selection.length === 0}
          <div class="flex flex-col items-center gap-2 py-6 text-center text-azure-900/50">
            <Icon name="check" size={32} />
            <p class="text-sm">{t("bulk.nothing")}</p>
          </div>
        {/if}
      {:else}
        <!-- Progress bar -->
        <div class="mb-4">
          <div class="mb-1.5 flex items-center justify-between text-xs text-azure-900/55">
            <span>
              {#if phase === "running"}
                {t("bulk.progress.count", { done: doneCount, total: totalSteps })}
              {:else}
                {wasCancelled
                  ? t("bulk.progress.interrupted", { done: doneCount, total: totalSteps })
                  : t("bulk.progress.count", { done: totalSteps, total: totalSteps })}
              {/if}
            </span>
            <span class="font-semibold">{pct}%</span>
          </div>
          {#if phase === "running" && etaLabel}
            <p class="mb-1.5 text-xs text-azure-900/55">{t("bulk.eta", { eta: etaLabel })}</p>
          {/if}
          <div class="h-2 overflow-hidden rounded-full bg-surface/60">
            <div
              class="h-full rounded-full transition-all duration-500 {wasCancelled
                ? 'bg-peach/70'
                : phase === 'done'
                  ? 'bg-mint/70'
                  : 'bg-azure-400/80'}"
              style="width: {pct}%"
            ></div>
          </div>
        </div>

        <!-- Per-game status list -->
        {#if onnext && phase === "running"}
          <p class="mb-2 flex items-center gap-1.5 rounded-lg border border-sky/25 bg-sky-soft/70 px-3 py-2 text-xs text-sky-deep">
            <Icon name="info" size={13} class="shrink-0" />
            {t("bulk.manual.hint", { label: nextLabel })}
          </p>
        {/if}
        <div class="flex flex-col gap-1">
          {#each latest as ev (`${ev.phase}:${ev.app_id}`)}
            <div class="flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm {ev.status === 'working' ? 'bg-lilac-soft/50' : 'bg-surface/30'}">
              <Icon
                name={STATUS_ICON[ev.status] ?? "info"}
                size={14}
                class="shrink-0 {STATUS_CLASS[ev.status] ?? ''} {ev.status === 'working' ? 'animate-[lv-spin_0.9s_linear_infinite]' : ''}"
              />
              <span class="min-w-0 flex-1 truncate font-medium">{ev.name}</span>
              <span class="shrink-0 text-xs {ev.status === 'error' ? 'text-rose-deep' : 'text-azure-900/45'}">
                {ev.detail}
              </span>
            </div>
          {/each}
        </div>

        <!-- Final summary -->
        {#if phase === "done" && report}
          <div class="mt-4 flex flex-wrap gap-3 rounded-xl border border-surface/50 bg-surface/40 px-4 py-3 text-xs">
            {#if report.succeeded > 0}
              <span class="flex items-center gap-1 font-semibold text-mint-deep">
                <Icon name="check" size={12} />
                {report.succeeded} {WORD.ok}
              </span>
            {/if}
            {#if report.failed > 0}
              <span class="flex items-center gap-1 font-semibold text-rose-deep">
                <Icon name="alert" size={12} />
                {t("bulk.report.failed", { n: report.failed })}
              </span>
            {/if}
            {#if report.skipped > 0}
              <span class="flex items-center gap-1 font-semibold text-azure-900/45">
                <Icon name="clock" size={12} />
                {t("bulk.report.skipped", { n: report.skipped })}
              </span>
            {/if}
          </div>
        {/if}
      {/if}
    </div>

    <!-- Footer -->
    <div class="flex items-center justify-end gap-2 border-t border-surface/50 px-5 py-3.5">
      {#if phase === "confirm"}
        <button
          onclick={onclose}
          class="lift rounded-xl border border-surface/70 bg-surface/60 px-4 py-2 text-sm font-semibold text-azure-800 hover:border-azure-200 hover:bg-surface/90"
        >
          {t("bulk.cancel")}
        </button>
        {#if plan.games.length > 0 || plan.fixes.length > 0 || plan.selection.length > 0}
          <button
            onclick={onconfirm}
            class="lift sheen rounded-xl bg-gradient-to-br from-azure-500 to-azure-600 px-4 py-2 text-sm font-semibold text-white shadow-md hover:from-azure-400 hover:to-azure-500 hover:shadow-lg"
          >
            {totalSteps > 1
              ? t("bulk.start_many", { n: totalSteps })
              : t("bulk.start_one", { n: totalSteps })}
          </button>
        {/if}
      {:else if phase === "running"}
        <button
          onclick={oncancel}
          class="lift rounded-xl border border-rose/30 bg-rose-soft/60 px-4 py-2 text-sm font-semibold text-rose-deep hover:bg-rose-soft"
        >
          <span class="flex items-center gap-1.5">
            <Icon name="x" size={13} />
            {t("bulk.stop")}
          </span>
        </button>
        {#if onnext}
          <button
            onclick={onnext}
            class="lift sheen rounded-xl bg-gradient-to-br from-azure-500 to-azure-600 px-4 py-2 text-sm font-semibold text-white shadow-md hover:from-azure-400 hover:to-azure-500 hover:shadow-lg"
          >
            <span class="flex items-center gap-1.5">
              {nextLabel}
              <Icon name="play" size={13} />
            </span>
          </button>
        {/if}
      {:else}
        <button
          onclick={onclose}
          class="lift sheen rounded-xl bg-gradient-to-br from-azure-500 to-azure-600 px-4 py-2 text-sm font-semibold text-white shadow-md hover:from-azure-400 hover:to-azure-500 hover:shadow-lg"
        >
          {t("bulk.close")}
        </button>
      {/if}
    </div>
  </div>
</div>

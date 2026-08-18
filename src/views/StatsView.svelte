<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../components/Icons.svelte";
  import { libraryStats, type LibraryStats, type GameStage } from "../lib/api";
  import { STAGE_ORDER, stageInfo, TONE_EDGE, type StageInfo } from "../lib/stages";
  import { formatBytes, formatPlaytime, formatTotalPlaytime } from "../lib/format";
  import { t } from "../lib/i18n.svelte";

  let stats = $state<LibraryStats | null>(null);
  let loading = $state(true);
  let computing = $state(false);
  let error = $state<string | null>(null);

  /** Patches downloaded but not applied. `derive_stage` only reports
      `fix_downloaded` when the fix is NOT installed, so this count already is
      the pending one — subtracting the installed ones would double-count. */
  const pending = $derived(
    stats?.by_stage.find((b) => b.stage === "fix_downloaded")?.count ?? 0,
  );

  onMount(() => void fetchStats());

  async function fetchStats() {
    loading = true;
    error = null;
    try {
      stats = await libraryStats();
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function refresh() {
    computing = true;
    error = null;
    try {
      stats = await libraryStats();
    } catch (e) {
      error = String(e);
    } finally {
      computing = false;
    }
  }

  function stageFor(stage: string): StageInfo {
    return stageInfo(stage as GameStage);
  }

  function sortedStages(): { stage: string; info: StageInfo; count: number }[] {
    if (!stats) return [];
    // Build a lookup from the backend data.
    const lookup = new Map<string, number>();
    for (const s of stats.by_stage) {
      lookup.set(s.stage, s.count);
    }
    // Emit in STAGE_ORDER, skipping unknowns.
    const result: { stage: string; info: StageInfo; count: number }[] = [];
    for (const s of STAGE_ORDER) {
      const count = lookup.get(s);
      if (count && count > 0) {
        result.push({ stage: s, info: stageFor(s), count });
      }
    }
    // Any remaining stages not in STAGE_ORDER (shouldn't happen, but be safe).
    for (const [stage, count] of lookup) {
      if (!result.some((r) => r.stage === stage)) {
        result.push({ stage, info: stageFor(stage), count });
      }
    }
    return result;
  }
</script>

<div class="flex h-full flex-col overflow-y-auto p-6">
  {#if loading}
    <div class="flex flex-1 items-center justify-center">
      <div class="flex flex-col items-center gap-3">
        <Icon name="refresh" size={28} />
        <span class="text-sm text-azure-700/70">{t("stats.loading")}</span>
      </div>
    </div>
  {:else if !stats}
    <div class="flex flex-1 items-center justify-center">
      <div class="flex flex-col items-center gap-3 rounded-xl border border-rose-soft/40 bg-surface/60 px-6 py-4 text-center">
        <Icon name="error" size={24} class="text-rose" />
        <span class="text-sm font-medium text-rose-deep">{t("stats.load_error")}</span>
        <span class="text-xs text-azure-700/60">{error}</span>
        <button
          onclick={fetchStats}
          class="lift mt-1 rounded-lg bg-surface/70 px-3 py-1.5 text-xs font-medium hover:bg-surface/90"
        >
          {t("stats.retry")}
        </button>
      </div>
    </div>
  {:else if stats.total === 0}
    <div class="flex flex-1 items-center justify-center">
      <div class="flex flex-col items-center gap-3 rounded-xl border border-surface/40 bg-surface/40 px-6 py-4 text-center">
        <Icon name="info" size={28} />
        <span class="text-sm font-medium">{t("stats.empty.title")}</span>
        <span class="text-xs text-azure-700/60">
          {t("stats.empty.hint")}
        </span>
      </div>
    </div>
  {:else}
    {#if error}
      <div class="rounded-lg border border-rose-soft/40 bg-rose/10 px-4 py-2 text-sm text-rose-deep">
        <span class="font-medium">{t("stats.refresh_failed")}</span> {error}
        <button
          onclick={fetchStats}
          class="ml-2 underline hover:no-underline"
        >
          {t("stats.reload")}
        </button>
      </div>
    {/if}
    <div class="flex flex-col gap-6">
      <!-- Header: key figures -->
      <div class="grid grid-cols-1 sm:grid-cols-3 gap-4">
        <div class="glass rounded-xl p-4 text-center" data-tip={t("stats.total.tip")}>
          <div class="text-3xl font-bold text-azure-800">{stats.total}</div>
          <div class="mt-1 text-xs font-medium text-azure-700/70">{stats.hidden > 0
            ? stats.hidden > 1
              ? t("stats.total.hidden_many", { n: stats.hidden })
              : t("stats.total.hidden_one", { n: stats.hidden })
            : t("stats.total.label")}</div>
        </div>
        <div class="glass rounded-xl p-4 text-center" data-tip={t("stats.fixes.tip")}>
          <div class="text-3xl font-bold text-mint-deep">{stats.fixes_installed}</div>
          <div class="mt-1 text-xs font-medium text-mint-deep/70">
            {pending > 0
              ? pending > 1
                ? t("stats.fixes.pending_many", { n: pending })
                : t("stats.fixes.pending_one", { n: pending })
              : t("stats.fixes.label")}
          </div>
        </div>
        <div class="glass rounded-xl p-4 text-center" data-tip={t("stats.local_space.tip")}>
          <div class="text-3xl font-bold text-azure-800">{formatBytes(stats.lua_bytes + stats.fix_archive_bytes + stats.backup_bytes)}</div>
          <div class="mt-1 text-xs font-medium text-azure-700/70">{t("stats.local_space.label")}</div>
        </div>
      </div>

      <!-- Refresh button -->
      <div class="flex justify-end">
        <button
          onclick={refresh}
          disabled={computing}
          class="lift flex items-center gap-2 rounded-lg bg-surface/50 px-3 py-1.5 text-xs font-medium hover:bg-surface/70 disabled:opacity-50"
          data-tip={t("stats.refresh.tip")}
        >
          <Icon name="refresh" size={14} class="{computing ? 'animate-spin' : ''}" />
          {computing ? t("stats.computing") : t("stats.refresh")}
        </button>
      </div>

      <!-- Distribution par état -->
      <div class="glass rounded-xl p-4">
        <h3 class="mb-3 text-sm font-semibold text-azure-800">{t("stats.distribution.title")}</h3>
        <div class="flex flex-col gap-2">
          {#each sortedStages() as { stage, info, count }}
            <div class="flex items-center gap-3">
              <span class="w-44 shrink-0 text-right text-xs font-medium text-azure-700/80">
                {info.label}
              </span>
              <div class="relative flex-1 overflow-hidden rounded-full bg-surface/60">
                <div
                  class="h-5 min-w-[2%] rounded-full transition-[width] duration-500 ease-out {TONE_EDGE[info.tone]}"
                  style="width: {stats.total > 0 ? (count / stats.total * 100) : 0}%"
                  aria-hidden="true"
                ></div>
              </div>
              <span class="w-8 shrink-0 text-xs font-bold text-azure-800">{count}</span>
            </div>
          {/each}
        </div>
      </div>

      <!-- Temps de jeu (LOT-13) : données locales de Steam, aucune requête
           réseau. Le total ne compte que ce qui est connu ; les jeux sans
           donnée sont affichés comme tels, jamais comme « 0 min ». Et quand
           AUCUN jeu n'a de donnée (Steam introuvable, compte ambigu), le
           total affiche « temps inconnu », pas « jamais joué » : le backend
           refuse de compter un inconnu comme zéro, l'affichage aussi. -->
      <div
        class="glass rounded-xl p-4"
        data-tip={t("stats.playtime.tip")}
      >
        <h3 class="mb-3 text-sm font-semibold text-azure-800">{t("stats.playtime.title")}</h3>
        <div class="flex flex-col gap-2 text-sm">
          <div class="flex justify-between">
            <span class="text-azure-700/70">{t("stats.playtime.total")}</span>
            <span class="font-medium text-azure-800">
              {formatTotalPlaytime(stats.playtime_total_minutes, stats.playtime_unknown, stats.total)}
            </span>
          </div>
          {#if stats.most_played}
            <div class="flex justify-between">
              <span class="text-azure-700/70">{t("stats.playtime.most_played")}</span>
              <span class="font-medium text-azure-800">
                {stats.most_played.name} · {formatPlaytime(stats.most_played.minutes)}
              </span>
            </div>
          {/if}
          {#if stats.playtime_unknown > 0}
            <p class="text-xs text-azure-700/50">
              {stats.playtime_unknown > 1
                ? t("stats.playtime.unknown_many", { n: stats.playtime_unknown })
                : t("stats.playtime.unknown_one", { n: stats.playtime_unknown })}
            </p>
          {/if}
        </div>
      </div>

      <!-- Espace disque -->
      <div class="glass rounded-xl p-4">
        <h3 class="mb-3 text-sm font-semibold text-azure-800">{t("stats.disk.title")}</h3>
        <div class="flex flex-col gap-2 text-sm">
          <div class="flex justify-between">
            <span class="text-azure-700/70">{t("stats.disk.lua_files")}</span>
            <span class="font-medium text-azure-800">{formatBytes(stats.lua_bytes)}</span>
          </div>
          <div class="flex justify-between">
            <span class="text-azure-700/70">{t("stats.disk.fix_archives")}</span>
            <span class="font-medium text-azure-800">{formatBytes(stats.fix_archive_bytes)}</span>
          </div>
          <div class="flex justify-between">
            <span class="text-azure-700/70">{t("stats.disk.backups")}</span>
            <span class="font-medium text-azure-800">{formatBytes(stats.backup_bytes)}</span>
          </div>
          <div class="border-t border-surface/40 pt-2">
            <div class="flex justify-between">
              <span class="text-azure-700/70">{t("stats.disk.local_subtotal")}</span>
              <span class="font-semibold text-azure-800">{formatBytes(stats.lua_bytes + stats.fix_archive_bytes + stats.backup_bytes)}</span>
            </div>
          </div>
          <div class="flex justify-between">
            <span class="text-azure-700/70">{t("stats.disk.installed_games")}</span>
            <span class="font-medium text-azure-800">{formatBytes(stats.games_on_disk_bytes)}</span>
          </div>
          <p class="text-xs text-azure-700/50" data-tip={t("stats.disk.installed_games.tip")}>
            {t("stats.disk.installed_games.hint")}
          </p>
        </div>
      </div>
    </div>
  {/if}
</div>

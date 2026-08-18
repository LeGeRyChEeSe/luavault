<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import Icon from "../components/Icons.svelte";
  import { appState } from "../lib/app-state.svelte";
  import { changelogFeed, type FeedReport } from "../lib/api";
  import { formatUnixDate } from "../lib/format";
  import { t } from "../lib/i18n.svelte";

  let report = $state<FeedReport | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let patchesOnly = $state(false);

  const items = $derived(report?.items ?? []);
  const visible = $derived(patchesOnly ? items.filter((i) => i.is_patch_notes) : items);
  const failed = $derived(report?.failed ?? []);
  const libraryEmpty = $derived(appState.library.length === 0);

  /** `cacheOnly` never touches the network: opening the tab while offline
      shows what the 30-minute cache still holds instead of firing forty
      requests doomed to time out. The offline signal measures Steam
      reachability; individual Steam requests still remain independently
      retryable. */
  async function refresh(force: boolean, cacheOnly = false) {
    if (loading) return;
    loading = true;
    error = null;
    try {
      report = await changelogFeed(force, cacheOnly);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    void refresh(false, !appState.online);
  });
</script>

<div class="flex h-full flex-col overflow-y-auto p-6">
  <div class="mb-5 flex flex-wrap items-center gap-3">
    <Icon name="news" size={22} />
    <div class="min-w-0">
      <h2 class="text-lg leading-tight font-semibold">{t("news.title")}</h2>
      <p class="text-xs text-azure-900/50">
        {t("news.subtitle")}
      </p>
    </div>
    <div class="ml-auto flex items-center gap-2">
      <button
        onclick={() => (patchesOnly = !patchesOnly)}
        aria-pressed={patchesOnly}
        data-tip={t("news.patches_only.tip")}
        class="lift flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-semibold {patchesOnly
          ? 'border-mint/40 bg-mint-soft/70 text-mint-deep'
          : 'border-surface/60 bg-surface/55 hover:bg-surface/80'}"
      >
        <Icon name="patch" size={13} />
        {t("news.patches_only.label")}
      </button>
      <button
        onclick={() => void refresh(true)}
        disabled={loading}
        data-tip={appState.online
          ? t("news.refresh.tip")
          : t("news.retry.tip")}
        class="lift flex items-center gap-1.5 rounded-lg border border-surface/60 bg-surface/55 px-2.5 py-1.5 text-xs font-semibold hover:bg-surface/80 disabled:cursor-not-allowed disabled:opacity-55"
      >
        <Icon
          name="refresh"
          size={13}
          class={loading ? "animate-[lv-spin_0.9s_linear_infinite]" : ""}
        />
        {appState.online ? t("news.refresh") : t("news.retry")}
      </button>
    </div>
  </div>

  {#if loading}
    <!-- Skeleton while posts are being fetched — one shape per card. -->
    <div class="flex flex-col gap-3">
      {#each [0, 1, 2, 3] as i (i)}
        <div class="animate-pulse rounded-xl border border-surface/40 bg-surface/40 p-4">
          <div class="h-3 w-36 rounded bg-azure-200/70"></div>
          <div class="mt-2.5 h-4 w-2/3 rounded bg-azure-200/80"></div>
          <div class="mt-2.5 h-3 w-full rounded bg-azure-200/50"></div>
          <div class="mt-1.5 h-3 w-5/6 rounded bg-azure-200/50"></div>
        </div>
      {/each}
    </div>
  {:else if error && !report}
    <div class="flex flex-1 items-center justify-center">
      <div
        class="flex flex-col items-center gap-3 rounded-xl border border-rose-soft/40 bg-surface/60 px-6 py-4 text-center"
      >
        <Icon name="error" size={24} class="text-rose" />
        <span class="text-sm font-medium text-rose-deep">{t("news.load_error")}</span>
        <span class="text-xs text-azure-700/60">{error}</span>
        <button
          onclick={() => void refresh(true)}
          class="lift mt-1 rounded-lg bg-surface/70 px-3 py-1.5 text-xs font-medium hover:bg-surface/90"
        >
          {t("news.retry")}
        </button>
      </div>
    </div>
  {:else if libraryEmpty && items.length === 0}
    <!-- An empty library is a state, not an error. -->
    <div class="flex flex-1 items-center justify-center">
      <div
        class="flex flex-col items-center gap-3 rounded-xl border border-surface/40 bg-surface/40 px-6 py-4 text-center"
      >
        <Icon name="library" size={28} />
        <span class="text-sm font-medium">{t("news.library_empty.title")}</span>
        <span class="text-xs text-azure-700/60">
          {t("news.library_empty.hint")}
        </span>
      </div>
    </div>
  {:else}
    {#if !appState.online}
      <p
        class="mb-3 flex items-center gap-2 rounded-lg border border-sky/25 bg-sky-soft/50 px-3 py-2 text-xs text-sky-deep"
      >
        <Icon name="offline" size={13} class="shrink-0" />
        {t("news.offline")}
      </p>
    {/if}

    {#if error && report}
      <!-- A failed refresh is not swallowed once a feed is on screen: the
           stale report stays visible, and this banner says so. Non-blocking,
           peach like the partial-failure banner — rose stays the colour of
           the blocking card shown when nothing could be loaded at all. -->
      <p
        data-tip={error}
        class="mb-3 flex items-center gap-2 rounded-lg border border-peach/30 bg-peach-soft/55 px-3 py-2 text-xs text-peach-deep"
      >
        <Icon name="alert" size={13} class="shrink-0" />
        {t("news.stale")}
      </p>
    {/if}

    {#if report && (report.from_cache > 0 || report.fetched > 0)}
      <p class="mb-3 text-[0.66rem] text-azure-900/40">
        {items.length > 1
          ? t("news.count.articles", { n: items.length })
          : t("news.count.article", { n: items.length })}
        {#if report.from_cache > 0}· {t("news.count.cache_served", { n: report.from_cache })}{/if}
        {#if report.fetched > 0}· {t("news.count.fetched", { n: report.fetched })}{/if}
      </p>
    {/if}

    {#if failed.length > 0}
      <!-- A truncated feed says so: which games are missing, and why. -->
      <p
        data-tip={failed.map((f) => `${f.game_name} : ${f.error}`).join(" — ")}
        class="mb-3 flex items-center gap-2 rounded-lg border border-peach/30 bg-peach-soft/55 px-3 py-2 text-xs text-peach-deep"
      >
        <Icon name="alert" size={13} class="shrink-0" />
        {failed.length > 1
          ? t("news.failed.many", { n: failed.length })
          : t("news.failed.one", { n: failed.length })}
        {failed.map((f) => f.game_name).join(", ")}.
      </p>
    {/if}

    {#if visible.length === 0}
      <div class="flex flex-1 items-center justify-center">
        <div
          class="flex flex-col items-center gap-3 rounded-xl border border-surface/40 bg-surface/40 px-6 py-4 text-center"
        >
          <Icon name="news" size={28} />
          <span class="text-sm font-medium">
            {#if patchesOnly && items.length > 0}
              {t("news.empty.patches")}
            {:else if failed.length > 0 && items.length === 0}
              {t("news.empty.failed")}
            {:else}
              {t("news.empty.recent")}
            {/if}
          </span>
          {#if patchesOnly && items.length > 0}
            <span class="text-xs text-azure-700/60">
              {t("news.empty.patches_hint")}
            </span>
          {/if}
        </div>
      </div>
    {:else}
      <!-- Third-party text is rendered as text — never as markup. -->
      <div class="flex flex-col gap-3">
        {#each visible as item (`${item.app_id}:${item.date}:${item.title}`)}
          <article class="rounded-xl border border-surface/55 bg-surface/45 p-4">
            <div class="flex items-center gap-2">
              <span
                class="truncate rounded-full bg-azure-500/12 px-2 py-0.5 text-[0.66rem] font-bold text-azure-700"
              >
                {item.game_name}
              </span>
              {#if item.is_patch_notes}
                <span
                  data-tip={t("news.patch.tip")}
                  class="flex shrink-0 items-center gap-1 rounded-full border border-mint/30 bg-mint-soft/70 px-2 py-0.5 text-[0.66rem] font-bold text-mint-deep"
                >
                  <Icon name="patch" size={11} />
                  {t("news.patch.label")}
                </span>
              {/if}
              <span class="ml-auto shrink-0 text-[0.66rem] text-azure-900/45">
                {formatUnixDate(item.date)}
              </span>
            </div>
            <h3 class="mt-2 text-sm font-semibold">{item.title}</h3>
            <p class="mt-1 text-xs leading-relaxed whitespace-pre-line text-azure-900/70">
              {item.excerpt}
            </p>
            {#if item.url}
              <button
                onclick={() => void openUrl(item.url)}
                class="mt-2 text-xs font-semibold text-azure-600 underline underline-offset-2 hover:text-azure-500"
              >
                {t("news.read_more")}
              </button>
            {/if}
          </article>
        {/each}
      </div>
    {/if}
  {/if}
</div>

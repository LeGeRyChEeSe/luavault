<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import Artwork from "./Artwork.svelte";
  import Icon from "./Icons.svelte";
  import GameActions from "./GameActions.svelte";
  import { appState } from "../lib/app-state.svelte";
  import type { GameSpotlight } from "../lib/app-state.svelte";
  import { focusTrap } from "../lib/focus-trap";
  import { getSteamDetails, gameStatus, setLibraryDisplay } from "../lib/api";
  import type { GameStatus, SteamDetails } from "../lib/api";
  import { formatPlaytime, formatUnixDate } from "../lib/format";
  import { t, i18n } from "../lib/i18n.svelte";
  import type { LocaleId } from "../lib/i18n.svelte";
  import { displayCorrection, spotlightTitle } from "../lib/spotlight";
  import { creditLines } from "../lib/spotlight-credits";

  const spotlight = $derived(appState.spotlight);

  /** Steam uses its own language ids. Keep this tied to the shipped locales,
      while an unexpected persisted value still yields a readable store page. */
  const STEAM_LANGUAGES: Readonly<Record<LocaleId, string>> = {
    fr: "french",
    en: "english",
  };

  function steamLanguage(locale: string): string {
    return STEAM_LANGUAGES[locale as LocaleId] ?? "english";
  }

  /** The game on screen. Kept alive during the zoom-out so the card doesn't
      blank a beat before the animation finishes. */
  let current = $state<GameSpotlight | null>(null);
  let closing = $state(false);
  let closeTimer: ReturnType<typeof setTimeout> | undefined;

  let details = $state<SteamDetails | null>(null);
  let status = $state<GameStatus | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  // Follow the global spotlight; a new pick cancels a pending zoom-out.
  $effect(() => {
    if (spotlight) {
      clearTimeout(closeTimer);
      closing = false;
      current = spotlight;
    }
  });

  // A delayed close must never outlive this component. The effect has no
  // reactive dependencies, so its cleanup runs only when the spotlight is
  // dismantled rather than cancelling the card's own zoom-out on close.
  $effect(() => {
    return () => clearTimeout(closeTimer);
  });

  // Steam's translated content follows both the current game and the selected locale.
  $effect(() => {
    const cur = current;
    if (!cur) return;
    let cancelled = false;
    loading = true;
    error = null;
    details = null;
    const lang = steamLanguage(i18n.locale);
    getSteamDetails(cur.appId, lang)
      .then((result) => {
        if (cancelled) return;
        details = result;
        const correction = displayCorrection(cur, result);
        if (correction) {
          void setLibraryDisplay(cur.appId, correction.name, correction.icon).then(() => {
            if (!cancelled) void appState.refreshLibrary();
          });
        }
      })
      .catch((e) => {
        if (!cancelled) error = String(e);
      })
      .finally(() => {
        if (!cancelled) loading = false;
      });
    return () => {
      cancelled = true;
    };
  });

  // A different game refreshes its disk-backed status.
  // This effect intentionally does not read i18n.locale.
  $effect(() => {
    const cur = current;
    shotIndex = null;
    if (!cur) return;
    status = null;
    void refreshStatus(cur.appId);
  });

  /** Read the install/patch state so the card carries the same actions as the
      small library card. Guards against a stale answer overwriting a newer pick. */
  async function refreshStatus(appId?: string) {
    const id = appId ?? current?.appId;
    if (!id) return;
    try {
      const result = await gameStatus(id);
      if (id !== current?.appId) return;
      status = result;
    } catch {
      if (id === current?.appId) status = null;
    }
  }

  /** Any GameActions run can change the local status. Refresh it afterwards. */
  function afterAction() {
    void refreshStatus();
  }

  function close() {
    if (closing) return;
    closing = true;
    appState.closeSpotlight();
    // Drop the card once the zoom-out has played.
    closeTimer = setTimeout(() => {
      current = null;
      closing = false;
    }, 210);
  }

  /** Index of the screenshot open in the viewer, or null when it is shut.
      The viewer sits above the card, so it takes Escape and the arrows for
      itself — otherwise Escape would close the whole card underneath and the
      user would lose their place. */
  let shotIndex = $state<number | null>(null);
  const shots = $derived(details?.screenshots ? details.screenshots : []);
  const openShot = $derived(shotIndex === null ? null : (shots[shotIndex] ?? null));

  function stepShot(delta: number) {
    if (shotIndex === null || shots.length === 0) return;
    // Wrap both ways: the strip is a ring, so the last shot's "next" is the
    // first one rather than a dead end.
    shotIndex = (shotIndex + delta + shots.length) % shots.length;
  }

  function onKeyDown(event: KeyboardEvent) {
    if (shotIndex !== null) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        shotIndex = null;
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        event.stopPropagation();
        stepShot(1);
      } else if (event.key === "ArrowLeft") {
        event.preventDefault();
        event.stopPropagation();
        stepShot(-1);
      }
      return;
    }
    if (event.key === "Escape") close();
  }

  const storeUrl = $derived(
    current ? `https://store.steampowered.com/app/${current.appId}/` : "",
  );

</script>

<svelte:window onkeydown={onKeyDown} />

{#if current}
  {@const game = current}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-xs transition-opacity duration-200 {closing
      ? 'opacity-0'
      : 'opacity-100'}"
    onclick={close}
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="glass-strong lv-spotlight-card relative flex max-h-full w-full max-w-[27rem] flex-col overflow-hidden rounded-xl2 {closing
        ? 'lv-spotlight-card-out'
        : ''}"
      onclick={(event) => event.stopPropagation()}
      use:focusTrap={{ initial: "container" }}
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-labelledby="spotlight-title"
      aria-hidden={openShot !== null ? "true" : undefined}
    >
      <button
        onclick={close}
        aria-label={t("spotlight.close.aria-label")}
        data-tip={t("spotlight.close.tip")}
        class="lift absolute top-3 right-3 z-10 rounded-full bg-surface/60 p-1.5 text-azure-900/70 hover:bg-surface/90 hover:text-azure-900"
      >
        <Icon name="x" size={16} />
      </button>

      <!-- Big banner image: Steam's header artwork. Fallback to the small icon
           when unavailable (offline, network failure, missing asset). -->
      <div class="relative h-44 w-full bg-surface/50">
        <Artwork
          url={details?.header_image ?? game.icon}
          alt={game.name}
          class="h-full w-full object-cover"
          iconSize={48}
        />
        <div
          class="absolute inset-0 bg-gradient-to-t from-surface-elevated via-surface-elevated/30 to-transparent"
        ></div>
        <div class="absolute bottom-3 left-4 right-4 flex items-end justify-between gap-3">
          <div class="min-w-0">
            <h2 id="spotlight-title" class="truncate text-xl font-bold tracking-tight text-azure-900">
              {spotlightTitle(game.name, details?.name)}
            </h2>
            {#each creditLines(details?.developers, details?.publishers) as line}
              <p class="truncate text-xs font-medium text-azure-900/60">
                {#if line.key === "combined"}
                  {t("spotlight.developer")} / {t("spotlight.publisher")}
                {:else if line.key === "spotlight.developer"}
                  {t("spotlight.developer")}
                {:else}
                  {t("spotlight.publisher")}
                {/if}: {line.names}
              </p>
            {:else}
              <p class="truncate text-xs font-medium text-azure-900/60">
                {t("spotlight.app-id", { id: game.appId })}
              </p>
            {/each}
          </div>
          {#if storeUrl}
            <button
              onclick={() => void openUrl(storeUrl)}
              aria-label={t("spotlight.store.label")}
              data-tip={t("spotlight.store.tip")}
              class="lift shrink-0 rounded-lg border border-surface/60 bg-surface/80 p-2 text-azure-600 transition hover:bg-surface hover:text-azure-500"
            >
              <Icon name="external" size={16} />
            </button>
          {/if}
        </div>
      </div>

      <!-- Scrollable content area: description, genres, screenshots, news -->
      <div class="flex-1 overflow-y-auto p-4 space-y-4">
        {#if loading}
          <div class="flex items-center justify-center py-8 text-azure-900/50">
            <Icon name="refresh" size={20} class="animate-[lv-spin_0.9s_linear_infinite]" />
            <span class="ml-2 text-sm">{t("spotlight.loading")}</span>
          </div>
        {:else if error}
          <div class="rounded-xl border border-peach/30 bg-peach-soft/50 p-3 text-xs text-peach-deep">
            {error}
          </div>
        {:else}
          {#if details?.short_description}
            <p class="text-xs leading-relaxed text-azure-900/80">
              {details.short_description}
            </p>
          {/if}

          {#if details?.genres && details.genres.length > 0}
            <div class="flex flex-wrap gap-1.5">
              {#each details.genres as genre (genre)}
                <span
                  class="rounded-md bg-azure-500/10 px-2 py-0.5 text-[0.66rem] font-semibold text-azure-700"
                >
                  {genre}
                </span>
              {/each}
            </div>
          {/if}

          {#if details?.metacritic != null}
            <div class="flex flex-wrap gap-1.5">
              <span
                class="rounded-md bg-azure-500/10 px-2 py-0.5 text-[0.66rem] font-semibold text-azure-700"
              >
                {t("spotlight.metacritic", { score: details.metacritic })}
              </span>
            </div>
          {/if}

          <!-- LOT-13: local playtime — Steam's own records, read on this
               machine. The three states stay visible and distinct: a
               measured time, "jamais joué" and "temps inconnu". -->
          {#if status}
            <div
              class="flex items-center gap-3 rounded-xl border border-surface/55 bg-surface/45 px-3 py-2.5"
              data-tip={t("spotlight.playtime.tip")}
            >
              <Icon name="clock" size={18} class="shrink-0" />
              <div class="min-w-0">
                <div class="text-sm font-semibold">
                  {formatPlaytime(status.playtime_minutes, status.last_played)}
                </div>
                <div class="text-xs text-azure-900/50">
                  {#if status.last_played}
                    {t("spotlight.playtime.last", { date: formatUnixDate(status.last_played) })}
                  {:else if status.playtime_minutes === 0}
                    {t("spotlight.playtime.none")}
                  {:else}
                    {t("spotlight.playtime.unknown")}
                  {/if}
                </div>
              </div>
            </div>
          {/if}

          <!-- Same actions as the small card, so the big one is never a dead end. -->
          {#if status}
            <div class="border-t border-surface/55 pt-4">
              <GameActions {status} onAfterAction={afterAction} />
            </div>
          {/if}

          <!-- Screenshot gallery: thumbnail strip, click to enlarge -->
          {#if shots.length > 0}
            <div class="space-y-2">
              <span class="text-xs font-bold tracking-wide text-azure-900/50 uppercase">
                {t("spotlight.screenshots.heading")}
              </span>
              <div class="flex gap-2 overflow-x-auto pb-1">
                {#each shots as shot, i (shot.thumbnail)}
                  <button
                    onclick={() => (shotIndex = i)}
                    class="lift relative shrink-0 overflow-hidden rounded-lg border border-surface/60 transition hover:border-azure-500"
                  >
                    <Artwork
                      url={shot.thumbnail}
                      class="h-20 rounded-lg object-cover shadow-sm"
                      iconSize={15}
                    />
                  </button>
                {/each}
              </div>
            </div>
          {/if}

          {#if details?.changelog}
            {@const changelog = details.changelog}
            <div class="rounded-xl border border-surface/55 bg-surface/45 p-3">
              <div class="flex items-center gap-2">
                <Icon name={details.changelog.is_patch_notes ? "patch" : "info"} size={15} />
                <span class="text-xs font-bold tracking-wide text-azure-900/50 uppercase">
                  {details.changelog.is_patch_notes ? t("spotlight.changelog.patch") : t("spotlight.changelog.news")}
                </span>
                <span class="ml-auto text-[0.66rem] text-azure-900/45">
                  {formatUnixDate(details.changelog.date)}
                </span>
              </div>
              <h3 class="mt-1 text-xs font-bold text-azure-900">
                {changelog.title}
              </h3>
              {#if changelog.body}
                <p class="mt-1 line-clamp-4 text-xs leading-relaxed text-azure-900/75">
                  {changelog.body}
                </p>
              {/if}
              {#if changelog.url}
                <button
                  onclick={() => void openUrl(changelog.url)}
                  class="lift mt-2 inline-flex items-center gap-1 text-xs font-semibold text-azure-600 hover:underline"
                >
                  {t("spotlight.changelog.read")}
                  <Icon name="external" size={12} />
                </button>
              {/if}
            </div>
          {/if}
        {/if}
      </div>
    </div>
  </div>
{/if}

<!-- Fullscreen screenshot viewer -->
{#if openShot}
  {@const shot = openShot}
  <div
    class="fixed inset-0 z-60 flex items-center justify-center bg-black/80 backdrop-blur-sm"
    onclick={() => (shotIndex = null)}
    use:focusTrap
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-label={t("spotlight.viewer.aria-label")}
  >
    <!-- Always present so the viewer keeps at least one focusable control,
         even for a game with a single screenshot. -->
    <button
      onclick={(e) => { e.stopPropagation(); shotIndex = null; }}
      aria-label={t("spotlight.viewer.close.aria-label")}
      data-tip={t("spotlight.close.tip")}
      class="lift absolute top-3 right-3 rounded-full bg-surface/70 p-2 hover:bg-surface/90"
    >
      <Icon name="x" size={18} />
    </button>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="flex min-h-0 items-center gap-3" onclick={(e) => e.stopPropagation()}>
      {#if shots.length > 1}
        <button
          onclick={() => stepShot(-1)}
          aria-label={t("spotlight.viewer.prev.aria-label")}
          data-tip={t("spotlight.viewer.prev.tip")}
          class="lift shrink-0 rounded-full bg-surface/70 p-2 hover:bg-surface/90"
        >
          <Icon name="chevron-left" size={24} />
        </button>
      {/if}
      <div class="relative max-h-[85vh] max-w-[85vw] overflow-hidden rounded-xl">
        <Artwork
          url={shot.full}
          alt={t("spotlight.viewer.aria-label")}
          class="max-h-[85vh] max-w-[85vw] object-contain shadow-2xl"
          iconSize={48}
        />
      </div>
      {#if shots.length > 1}
        <button
          onclick={() => stepShot(1)}
          aria-label={t("spotlight.viewer.next.aria-label")}
          data-tip={t("spotlight.viewer.next.tip")}
          class="lift shrink-0 rounded-full bg-surface/70 p-2 hover:bg-surface/90"
        >
          <Icon name="chevron-right" size={24} />
        </button>
      {/if}
    </div>
  </div>
{/if}

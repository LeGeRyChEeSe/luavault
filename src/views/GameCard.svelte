<script lang="ts">
  import Artwork from "../components/Artwork.svelte";
  import Icon from "../components/Icons.svelte";
  import StatusBadge from "../components/StatusBadge.svelte";
  import GameActions from "../components/GameActions.svelte";
  import { appState } from "../lib/app-state.svelte";
  import type { GameStatus } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  import { stageInfo, TONE_EDGE } from "../lib/stages";
  import { formatDate, formatPlaytime, formatUnixDate } from "../lib/format";

  let {
    status,
    /** LOT-16 — multi-select mode: the card becomes a selection tile. */
    selecting = false,
    selected = false,
    onToggleSelect = () => {},
    /** Open the context menu at the given viewport coords. */
    onContextMenu,
    onKebabClick,
  }: {
    status: GameStatus;
    selecting?: boolean;
    selected?: boolean;
    onToggleSelect?: () => void;
    onContextMenu: (status: GameStatus, event: MouseEvent) => void;
    onKebabClick: (status: GameStatus, event: MouseEvent) => void;
  } = $props();

  const stage = $derived(stageInfo(status.stage));
  const fix = $derived(status.fix);
  const problems = $derived([...fix.missing, ...fix.modified]);
  /** True while the stage itself already speaks about the patch. */
  const stageIsAboutFix = $derived(status.stage.startsWith("fix_"));

  let expanded = $state(false);

  function onCardContextMenu(event: MouseEvent) {
    event.preventDefault();
    onContextMenu(status, event);
  }

  function onKebab(event: MouseEvent) {
    event.stopPropagation();
    onKebabClick(status, event);
  }

  function openSpotlight() {
    appState.openSpotlight({
      appId: status.app_id,
      name: status.name,
      icon: status.icon,
    });
  }

  /** Selection mode: a click anywhere on the card toggles — the header
      stops opening the spotlight, the checkbox says it out loud. */
  function onCardClick() {
    if (!selecting) return;
    onToggleSelect();
  }

  function onInnerClick(event: MouseEvent) {
    if (!selecting) return;
    // The article click would toggle a second time — one gesture, one flip.
    event.stopPropagation();
    onToggleSelect();
  }
</script>

<!-- `tabindex="-1"` makes the card focusable by script but keeps it out of the tab
     order: closing a menu opened by right-click can then hand the focus back here
     instead of dropping it on <body>. -->
<article
  class="glass lift-card relative flex flex-col gap-3 overflow-hidden rounded-xl2 p-4 pl-5 {selecting
    ? 'cursor-pointer'
    : ''} {selected ? 'ring-2 ring-azure-400/70' : ''}"
  tabindex="-1"
  oncontextmenu={onCardContextMenu}
  onclick={selecting ? onCardClick : undefined}
>
  <!-- Colour edge mirrors the badge: the card's state is readable at a glance. -->
  <span class="absolute inset-y-0 left-0 w-1.5 {TONE_EDGE[stage.tone]}"></span>

  <!-- The header opens the full Steam-style card; the buttons below stay
       untouched, so a click never triggers an action by accident. -->
  <div class="flex items-start gap-1.5">
    <button
      onclick={selecting ? onInnerClick : () => openSpotlight()}
      data-tip={selecting ? null : t("card.spotlight.tip")}
      class="flex min-w-0 flex-1 items-center gap-3 text-left"
    >
      <!-- LOT-14: served from the local artwork cache when available — the
           onerror fallback is the tile, shared with every other image. -->
      <Artwork
        url={status.icon}
        class="h-11 w-24 shrink-0 rounded-md object-cover shadow-sm"
      />
      <div class="min-w-0 flex-1">
        <h3 class="truncate text-sm font-semibold">{status.name}</h3>
        <p class="truncate text-xs text-azure-900/50">
          {status.app_id} · {formatDate(status.updated_at)}
        </p>
      </div>
    </button>
    {#if selecting}
      <button
        onclick={onInnerClick}
        aria-pressed={selected}
        aria-label={selected
          ? t("card.select.remove.aria-label", { name: status.name })
          : t("card.select.add.aria-label", { name: status.name })}
        data-tip={selected ? t("card.select.remove.tip") : t("card.select.add.tip")}
        class="lift shrink-0 rounded-lg p-1.5"
      >
        <span
          class="flex h-5 w-5 items-center justify-center rounded-md border transition {selected
            ? 'border-azure-500 bg-azure-500 text-white'
            : 'border-azure-900/30 bg-surface/70 text-transparent'}"
        >
          <Icon name="checkbox" size={13} tone="current" />
        </span>
      </button>
    {:else}
      <button
        onclick={onKebab}
        aria-label={t("card.menu.aria-label")}
        data-tip={t("card.menu.tip")}
        class="lift shrink-0 rounded-lg p-1.5 text-azure-900/45 hover:bg-surface/70 hover:text-azure-900/70"
      >
        <Icon name="more" size={16} />
      </button>
    {/if}
  </div>

  <!-- LOT-13: local playtime — Steam's own records, read on this machine.
       "temps inconnu" (no data) and "jamais joué" (zero with no session)
       stay two different lines on purpose. Steam also writes LastPlayed
       without Playtime (8 of the 96 sessions on the author's machine):
       that one renders "temps inconnu · der. session <date>" — a known
       date with an unknown duration, and the line says exactly that. -->
  <div
    class="flex items-center gap-1.5 text-xs text-azure-900/55"
    data-tip={t("card.playtime.tip")}
  >
    <Icon name="clock" size={13} class="shrink-0" />
    <span class="truncate">
      {formatPlaytime(status.playtime_minutes, status.last_played)}
      {#if status.last_played}
        · {t("card.playtime.last", { date: formatUnixDate(status.last_played) })}
      {/if}
    </span>
  </div>

  <div class="flex flex-wrap gap-1.5">
    <StatusBadge
      label={stage.label}
      icon={stage.icon}
      tone={stage.tone}
      tip={stage.tip}
      live={stage.live}
    />
    {#if status.game.installed && status.game.size_on_disk > 0}
      <StatusBadge
        label={t("card.installed.label")}
        icon="steam"
        tone="neutral"
        compact
        tip={status.game.install_dir ?? ""}
      />
    {/if}
  </div>

  {#if problems.length > 0}
    <div
      class="enter-fade rounded-xl border border-rose/25 bg-rose-soft/50 px-3 py-2 text-xs text-rose-deep"
    >
      <p class="font-semibold">
        {t("card.problems.count", {
          missing: fix.missing.length,
          modified: fix.modified.length,
        })}
      </p>
      <button
        onclick={() => (expanded = !expanded)}
        class="mt-1 underline decoration-rose/40 underline-offset-2 transition hover:decoration-rose"
      >
        {expanded ? t("card.problems.hide") : t("card.problems.show")}
      </button>
      {#if expanded}
        <ul class="mt-1.5 max-h-28 list-inside list-disc overflow-y-auto">
          {#each problems.slice(0, 40) as file (file)}
            <li class="break-all">{file}</li>
          {/each}
        </ul>
      {/if}
      <p class="mt-1.5">{t("card.problems.hint")}</p>
    </div>
  {/if}

  <div class="mt-auto">
    {#if selecting}
      <p class="text-xs text-azure-900/45">
        {selected ? t("card.select.selected") : t("card.select.hint")}
      </p>
    {:else}
      <GameActions {status} full />
    {/if}
  </div>
</article>

<script lang="ts">
  import Icon from "./Icons.svelte";
  import { appState } from "../lib/app-state.svelte";
  import { resolveArtwork } from "../lib/artwork";

  let {
    url,
    class: cls = "",
    style = "",
    alt = "",
    /** The neutral tile's background — the same one GameCard always had. */
    tileClass = "bg-azure-100/80",
    iconSize = 18,
  }: {
    url?: string | null;
    class?: string;
    style?: string;
    alt?: string;
    tileClass?: string;
    iconSize?: number;
  } = $props();

  /** Resolved asset-protocol src, or null while absent / broken. */
  let src = $state<string | null>(null);
  /** True once the first resolution attempt finished: before that, the
      fallback tile stays hidden so an image already on disk never flashes
      an empty state before appearing. Never READ by the effect below — a
      write must not re-run it (that re-run was the second resolution of
      every dead URL: two IPC round-trips, two network attempts). */
  let settled = $state(false);
  let lastTarget: string | null = null;
  /** URLs already attempted WHILE ONLINE — the outcome is final. Deliberately
      non-reactive: writing a Set must never re-run the effect. */
  const triedOnline = new Set<string>();
  /** URLs that gave up OFFLINE (nothing on disk): they get their second
      chance the moment the connection comes back — that retry is the
      offline contract, and it is why the effect still reads appState.online. */
  const triedOffline = new Set<string>();

  $effect(() => {
    const target = url ?? null;
    const online = appState.online;
    if (target !== lastTarget) {
      lastTarget = target;
      src = null;
      settled = false;
    }
    if (!target) {
      settled = true; // nothing to resolve — the tile shows at once
      return;
    }
    if (src) return; // already on screen — a cached image never re-fetches
    if (triedOnline.has(target)) return; // failed online: final, never re-resolved
    if (!online && triedOffline.has(target)) return; // still offline: already gave up
    const wasOnline = online;
    if (!online) triedOffline.add(target);
    let cancelled = false;
    void resolveArtwork(target).then((resolved) => {
      if (cancelled) return;
      if (wasOnline) triedOnline.add(target);
      src = resolved;
      settled = true;
    });
    return () => {
      cancelled = true;
    };
  });

  function onError() {
    // The asset file vanished or turned unreadable under our feet: back to
    // the tile, never a broken frame.
    src = null;
    settled = true;
  }
</script>

{#if src}
  <img {src} {alt} class={cls} {style} loading="lazy" draggable="false" onerror={onError} />
{:else if settled}
  <!-- The existing neutral tile — one fallback look for the whole app. -->
  <div class="flex items-center justify-center {tileClass} {cls}" {style} aria-hidden="true">
    <Icon name="gamepad" size={iconSize} />
  </div>
{:else}
  <!-- Unsettled: invisible box that only holds the layout. A cached image
       resolves in one IPC round-trip and appears without ever showing the
       tile. -->
  <div class={cls} {style} aria-hidden="true"></div>
{/if}

<script lang="ts">
  import Icon from "./Icons.svelte";
  import type { GameMenuItem } from "../lib/game-menu";

  let { items, x, y, onclose, onaction }: {
    items: GameMenuItem[];
    x: number; y: number;
    onclose: () => void;
    onaction?: (item: GameMenuItem) => Promise<void>;
  } = $props();

  let menuRef: HTMLDivElement | null = null;

  /** Position the menu so it never overflows the viewport.
   *  The final position depends on the measured size after rendering,
   *  so the calculation lives in an effect rather than a derived. */
  let posX = $state(x);
  let posY = $state(y);

  $effect(() => {
    const _x = x;
    const _y = y;
    if (!menuRef) return;
    const rect = menuRef.getBoundingClientRect();

    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const pad = 8;

    posX = _x;
    posY = _y;

    if (_x + rect.width > vw - pad) {
      posX = Math.max(pad, vw - rect.width - pad);
    }
    if (_y + rect.height > vh - pad) {
      posY = Math.max(pad, vh - rect.height - pad);
    }
  });

  /** Read the DOM for button elements — no intermediate state to desync. */
  function menuButtons(): HTMLButtonElement[] {
    return Array.from(menuRef?.querySelectorAll<HTMLButtonElement>('button[role="menuitem"]') ?? []);
  }

  /** Focus the given index (among visible items). Wraps around at edges. */
  function setFocus(idx: number) {
    const btns = menuButtons();
    const n = btns.length;
    if (n === 0) return;
    if (idx < 0) idx = n - 1;
    if (idx >= n) idx = 0;
    btns[idx]?.focus();
  }

  function handleKeydown(e: KeyboardEvent) {
    const btns = menuButtons();
    if (btns.length === 0) return;

    switch (e.key) {
      case "Escape":
        e.preventDefault();
        onclose();
        break;
      case "ArrowDown":
        e.preventDefault();
        const curDown = btns.indexOf(document.activeElement as HTMLButtonElement);
        setFocus(curDown + 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        const curUp = btns.indexOf(document.activeElement as HTMLButtonElement);
        setFocus(curUp - 1);
        break;
      case "Home":
        e.preventDefault();
        setFocus(0);
        break;
      case "End":
        e.preventDefault();
        setFocus(menuButtons().length - 1);
        break;
    }
  }

  function activateItem(item: GameMenuItem) {
    if (onaction) {
      onaction(item);
    } else {
      item.run();
    }
    onclose();
  }

  // Global listeners — cleaned up on destroy
  $effect(() => {
    const kd = (e: KeyboardEvent) => { if (e.key === "Escape") onclose(); };
    window.addEventListener("keydown", kd);

    const outsideClick = (e: MouseEvent) => {
      if (menuRef && !menuRef.contains(e.target as Node)) {
        onclose();
      }
    };
    document.addEventListener("mousedown", outsideClick);

    const scrollClose = (e: Event) => {
      if (!menuRef?.contains(e.target as Node)) onclose();
    };
    window.addEventListener("scroll", scrollClose, true);

    // Focus first visible item
    $effect(() => {
      const btns = menuButtons();
      btns[0]?.focus();
    });

    return () => {
      window.removeEventListener("keydown", kd);
      document.removeEventListener("mousedown", outsideClick);
      window.removeEventListener("scroll", scrollClose, true);
      // Restore focus to the element that opened the menu
      if (focusedElement) {
        (focusedElement as HTMLElement).focus?.();
        focusedElement = null;
      }
    };
  });

  /** Remember which element had focus when the menu opened. */
  let focusedElement: HTMLButtonElement | HTMLElement | null = document.activeElement as HTMLButtonElement | HTMLElement | null;
</script>

<div
  class="fixed inset-0 z-[200]"
  role="presentation"
  onclick={onclose}
  oncontextmenu={(e) => { e.preventDefault(); }}
></div>

<div
  bind:this={menuRef}
  class="glass-strong enter-fade fixed z-[201] min-w-48 max-w-72 max-h-[calc(100vh-16px)] overflow-y-auto overflow-x-hidden rounded-xl border border-surface/60 p-1 shadow-xl"
  style="left: {posX}px; top: {posY}px;"
  role="menu"
  aria-label="Actions du jeu"
  tabindex={-1}
  onkeydown={handleKeydown}
>
  {#each items as item, i (item.id)}
    {#if item.separated && i > 0}<hr class="my-1 border-surface/40" />{/if}
    <button
      role="menuitem"
      disabled={item.disabled}
      class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm font-medium transition {item.disabled
        ? 'cursor-not-allowed opacity-45'
        : item.danger
          ? 'text-rose-deep hover:bg-rose-soft/50'
          : 'text-azure-900 hover:bg-surface/70'}"
      data-tip={item.disabled && item.disabledTip ? item.disabledTip : item.tip}
      onclick={() => activateItem(item)}
    >
      <Icon name={item.icon} size={15} />
      {item.label}
    </button>
  {/each}
</div>

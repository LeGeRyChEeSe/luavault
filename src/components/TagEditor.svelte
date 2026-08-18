<script lang="ts">
  import Icon from "./Icons.svelte";
  import { setLibraryTags } from "../lib/api";
  import { appState } from "../lib/app-state.svelte";
  import { focusTrap } from "../lib/focus-trap";
  import { t } from "../lib/i18n.svelte";
  import type { GameStatus } from "../lib/api";

  let {
    status,
    allLibraryTags,
    onclose,
    onsave,
    trigger,
  }: {
    status: GameStatus;
    allLibraryTags: string[];
    onclose: () => void;
    onsave: () => void;
    /** Stable element to restore focus to on destroy (card or kebab button), not the context-menu item. */
    trigger?: HTMLElement | null;
  } = $props();

  /** Working copy of the tags — shown to the user with the same normalisation
   *  as the backend so they see exactly what will be saved. */
  let localTags = $state<string[]>([...status.tags]);

  /** Input field value — the tag being typed. */
  let input = $state("");

  /** Bound panel — the outside-click test uses it directly instead of a
      global querySelector. */
  let panelRef: HTMLDivElement | null = $state(null);

  /** All tags currently used in the library (deduplicated, sorted). */
  const libraryTags = $derived(allLibraryTags);

  /** Tags already assigned to this game (from local working copy), normalised. */
  const usedTags = $derived(new Set(localTags.map((t) => normaliseTag(t).toLowerCase())));

  /** Suggestions: tags from the library that are not yet used on this game. */
  const suggestions = $derived(
    libraryTags.filter((t) => !usedTags.has(t.toLowerCase())),
  );

  /** Normalise a tag the same way the backend does: trim, collapse internal spaces, cap at 24 code points. */
  function normaliseTag(raw: string): string {
    const t = raw.trim().split(/\s+/).join(" ");
    if (t.length === 0) return "";
    return [...t].slice(0, 24).join("").replace(/\s+$/, "");
  }

  /** Error message shown under the input field. */
  let inputError = $state("");

  function addTag() {
    const tag = normaliseTag(input);
    if (!tag) {
      inputError = t("tags.error.empty");
      return;
    }
    // Don't add duplicates (on normalised value).
    if (localTags.some((t) => normaliseTag(t).toLowerCase() === tag.toLowerCase())) {
      inputError = t("tags.error.duplicate");
      return;
    }
    // Cap at 8.
    if (localTags.length >= 8) {
      inputError = t("tags.error.max");
      return;
    }
    inputError = "";
    localTags = [...localTags, tag];
    input = "";
  }

  function removeTag(tag: string) {
    localTags = localTags.filter((t) => t !== tag);
  }

  function useSuggestion(tag: string) {
    if (localTags.some((t) => normaliseTag(t).toLowerCase() === tag.toLowerCase())) return;
    if (localTags.length >= 8) return;
    inputError = "";
    localTags = [...localTags, tag];
  }

  /** Guard against double-click / concurrent saves. */
  let saving = $state(false);

  async function save() {
    if (saving) return;
    saving = true;
    try {
      await setLibraryTags(status.app_id, localTags);
      await appState.refreshLibrary();
      appState.toast("success", t("tags.toast.saved"));
      onsave();
      onclose();
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      saving = false;
    }
  }

  function handleClose() {
    onclose();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    } else if (e.key === "Enter") {
      e.preventDefault();
      addTag();
    }
  }

  /** Global listeners — cleaned up on destroy. The focusTrap action owns the
      focus itself (initial focus via data-autofocus, restore on destroy). */
  $effect(() => {
    const kd = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onclose();
      }
    };
    window.addEventListener("keydown", kd);

    const outsideClick = (e: MouseEvent) => {
      // Close if clicking outside the panel.
      if (panelRef && !panelRef.contains(e.target as Node)) {
        onclose();
      }
    };
    document.addEventListener("mousedown", outsideClick);

    return () => {
      window.removeEventListener("keydown", kd);
      document.removeEventListener("mousedown", outsideClick);
    };
  });
</script>

<!-- Full-screen dim overlay -->
<div class="fixed inset-0 z-[300] lv-veil" role="presentation" onclick={handleClose}></div>

<!-- Panel — rendered at view root for backdrop-filter to work -->
<div
  bind:this={panelRef}
  class="glass-strong enter-fade fixed z-[301] w-full max-w-md rounded-xl border border-surface/60 p-5 shadow-xl"
  style="top: 50%; left: 50%; transform: translate(-50%, -50%);"
  use:focusTrap={{ returnFocus: trigger }}
  role="dialog"
  tabindex="-1"
  aria-modal="true"
  aria-label={t("tags.dialog.aria-label")}
>
  <div class="flex items-center justify-between">
    <h3 class="text-base font-semibold">{t("tags.title", { name: status.name })}</h3>
    <button
      onclick={handleClose}
      aria-label={t("tags.close.aria-label")}
      class="lift rounded-md p-1 text-azure-900/45 hover:bg-surface/70 hover:text-azure-900/70"
    >
      <Icon name="x" size={16} />
    </button>
  </div>

  <!-- Current tags -->
  <div class="mt-3 flex flex-wrap gap-1.5">
    {#each localTags as tag (tag)}
      <span
        class="inline-flex items-center gap-1 rounded-full border border-lilac/40 bg-lilac-soft/40 px-2.5 py-1 text-xs font-medium text-lilac-deep"
      >
        {tag}
        <button
          onclick={() => removeTag(tag)}
          aria-label={t("tags.remove.aria-label", { tag })}
          class="lift rounded-full p-0.5 text-lilac-deep/60 hover:bg-lilac-soft/60 hover:text-lilac-deep"
        >
          <Icon name="x" size={10} />
        </button>
      </span>
    {/each}
    {#if localTags.length === 0}
      <span class="text-xs text-azure-900/40 italic">{t("tags.none")}</span>
    {/if}
  </div>

  <!-- Input -->
  <div class="mt-3">
    <input
      data-autofocus
      bind:value={input}
      onkeydown={handleKeydown}
      placeholder={t("tags.input.placeholder")}
      aria-label={t("tags.input.aria-label")}
      class="w-full rounded-lg border border-surface/70 bg-surface/60 px-3 py-2 text-sm outline-none backdrop-blur-md transition placeholder:text-azure-900/40 focus:border-azure-300 focus:bg-surface/85 focus:ring-2 focus:ring-azure-300/40"
    />
    {#if inputError}
      <p class="mt-1 text-xs text-rose-deep">{inputError}</p>
    {/if}
  </div>

  <!-- Suggestions -->
  {#if suggestions.length > 0}
    <div class="mt-2 flex flex-wrap gap-1">
      <span class="text-xs text-azure-900/40">{t("tags.suggestions")}</span>
      {#each suggestions.slice(0, 10) as tag (tag)}
        <button
          onclick={() => useSuggestion(tag)}
          class="lift rounded-full border border-surface/50 bg-surface/50 px-2 py-0.5 text-[0.7rem] text-azure-900/55 hover:bg-surface/70"
        >
          {tag}
        </button>
      {/each}
    </div>
  {/if}

  <!-- Actions -->
  <div class="mt-4 flex justify-end gap-2">
    <button
      onclick={handleClose}
      class="lift rounded-lg border border-surface/60 px-3 py-1.5 text-xs font-medium text-azure-900/65 hover:bg-surface/70"
    >
      {t("tags.cancel")}
    </button>
    <button
      onclick={save}
      disabled={saving}
      class="lift rounded-lg bg-gradient-to-b from-azure-500 to-azure-600 px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-50 disabled:cursor-not-allowed"
    >
      {t("tags.save")}
    </button>
  </div>
</div>

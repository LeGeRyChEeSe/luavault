<script lang="ts">
  import { focusTrap } from "../lib/focus-trap";
  import { downloadUpdate, installUpdate } from "../lib/api";
  import type { UpdateAvailable } from "../lib/api";
  import { t, i18n } from "../lib/i18n.svelte";
  import Icon from "./Icons.svelte";

  interface Props {
    update: UpdateAvailable;
    onclose: () => void;
  }

  let { update, onclose }: Props = $props();

  let busy: string | null = $state(null);
  let error: string | null = $state(null);

  function closeModal() {
    if (busy) return;
    onclose();
  }

  function resolveNotes(notes_i18n: Record<string, string> | null | undefined, fallback: string | null): string | null {
    if (notes_i18n) {
      return notes_i18n[i18n.locale] ?? notes_i18n["fr"] ?? notes_i18n["en"] ?? fallback;
    }
    return fallback;
  }

  /** Download then install the single available artifact. */
  async function handleUpdate() {
    if (busy) return;
    busy = "installing";
    error = null;
    try {
      const artifact = update.artifacts[0];
      const path = await downloadUpdate(update.version, artifact.file, artifact.sha256, artifact.size);
      await installUpdate(path);
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div
  class="enter-fade fixed inset-0 z-[300] flex items-center justify-center lv-veil"
  role="presentation"
  onclick={(e: MouseEvent) => { if (e.target === e.currentTarget) closeModal(); }}
>
  <div
    role="dialog"
    aria-modal="true"
    aria-labelledby="update-dialog-title"
    tabindex="-1"
    onkeydown={(e) => { if (e.key === "Escape") closeModal(); }}
    use:focusTrap
    class="glass enter-fade max-w-lg rounded-xl2 p-6 shadow-xl"
  >
    <h2 id="update-dialog-title" class="text-lg font-semibold mb-2">
      {t("update.title", { version: update.version })}
    </h2>

    {#if update.changes.length > 0}
      <div class="space-y-3 mb-4 max-h-60 overflow-y-auto">
        {#each update.changes as change (change.version)}
          <div class="rounded-xl bg-surface/50 px-4 py-3 text-sm">
            <div class="flex items-center gap-2 font-semibold">
              <Icon name="update" size={14} />
              <span>{t("update.change.version", { version: change.version })}</span>
            </div>
            {#if change.notes || change.notes_i18n}
              <p class="mt-1 whitespace-pre-line text-xs opacity-85">{resolveNotes(change.notes_i18n, change.notes)}</p>
            {/if}
          </div>
        {/each}
      </div>
    {:else if update.notes || update.notes_i18n}
      <p class="mb-4 whitespace-pre-line text-sm opacity-85">{resolveNotes(update.notes_i18n, update.notes)}</p>
    {/if}

    <div class="mt-4 flex justify-end gap-2">
      <button
        onclick={closeModal}
        class="rounded-lg bg-surface/60 px-3 py-1.5 text-xs font-medium hover:bg-surface/80"
      >
        {t("update.cancel")}
      </button>
      <button
        onclick={handleUpdate}
        disabled={busy !== null}
        class="rounded-lg bg-mint px-3 py-1.5 text-xs font-medium text-white hover:bg-mint/80 disabled:opacity-50"
      >
        {#if busy === "installing"}
          {t("update.busy")}
        {:else if error}
          {t("update.retry")}
        {:else}
          {t("update.install")}
        {/if}
      </button>
    </div>

    {#if error}
      <div class="mt-3 rounded-lg bg-rose-soft/50 px-3 py-2 text-xs text-rose-deep">
        {error}
      </div>
    {/if}
  </div>
</div>

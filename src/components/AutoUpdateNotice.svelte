<script lang="ts">
  /**
   * What the window says while an automatic update runs, so it never closes in silence
   * (`appState.autoUpdate`). While this window downloads, a card at the bottom; once the app is
   * about to close (its own update, or the background check's in another process), a veil over
   * everything: the person must not start something the next seconds would cut short.
   * No button: there is nothing to decide, the app comes back by itself.
   */
  import { t } from "../lib/i18n.svelte";
  import Icon from "./Icons.svelte";

  interface Props {
    version: string;
    phase: "downloading" | "restarting" | "background";
  }

  let { version, phase }: Props = $props();
  const closing = $derived(phase !== "downloading");
  const text = $derived(
    phase === "downloading"
      ? t("update.auto.downloading", { version })
      : phase === "restarting"
        ? t("update.auto.restarting", { version })
        : t("update.auto.background", { version }),
  );
</script>

{#if closing}
  <div class="enter-fade fixed inset-0 z-[400] flex items-center justify-center lv-veil p-4" data-auto-update="closing">
    <div role="status" aria-live="polite" class="glass enter-fade w-full max-w-md rounded-xl2 p-6 shadow-xl">
      <div class="mb-2 flex items-center gap-3">
        <span class="shrink-0 animate-spin text-sky-deep [animation-duration:2.4s]"><Icon name="refresh" size={22} /></span>
        <h2 class="text-lg leading-tight font-semibold">{t("update.auto.title")}</h2>
      </div>
      <p class="text-sm text-azure-900/80">{text}</p>
    </div>
  </div>
{:else}
  <div
    role="status"
    aria-live="polite"
    data-auto-update="downloading"
    class="glass enter-up fixed bottom-4 left-1/2 z-[250] flex w-[min(28rem,calc(100%-2rem))] -translate-x-1/2 items-start gap-3 rounded-xl2 p-4 shadow-xl"
  >
    <span class="mt-0.5 shrink-0 text-sky-deep"><Icon name="download" size={18} /></span>
    <span class="flex min-w-0 flex-col gap-0.5">
      <span class="text-sm font-semibold">{t("update.auto.title")}</span>
      <span class="text-xs text-azure-900/70">{text}</span>
    </span>
  </div>
{/if}

<script lang="ts">
  import Icon from "./Icons.svelte";
  import { appState } from "../lib/app-state.svelte";

  const toasts = $derived(appState.toasts);

  const STYLE: Record<string, { border: string; icon: string }> = {
    success: { border: "border-mint/45", icon: "check" },
    error: { border: "border-rose/45", icon: "error" },
    info: { border: "border-sky/45", icon: "info" },
    warning: { border: "border-peach/45", icon: "alert" },
  };

  function dismiss(id: number) {
    appState.toasts = appState.toasts.filter((t) => t.id !== id);
  }
</script>

<!-- Two persistent non-nested live regions: one polite for success/info,
     one assertive for errors. The visual stack lives outside the regions.
     Each toast's text is announced exactly once (aria-relevant="additions"
     means removals are silent). The dismiss button includes the toast text
     in its aria-label; the message is visible on screen and was already announced. -->
<div class="pointer-events-none fixed right-4 bottom-4 z-100 flex w-80 flex-col gap-2">
  <!-- Polite region — success / info -->
  <div class="sr-only" aria-live="polite" aria-relevant="additions">
    {#each toasts as toast (toast.id)}
      {#if toast.kind !== "error"}
        <span>{toast.text}</span>
      {/if}
    {/each}
  </div>
  <!-- Assertive region — errors -->
  <div class="sr-only" aria-live="assertive" aria-relevant="additions">
    {#each toasts as toast (toast.id)}
      {#if toast.kind === "error"}
        <span>{toast.text}</span>
      {/if}
    {/each}
  </div>
  <!-- Visual stack — outside live regions -->
  {#each toasts as toast (toast.id)}
    <button
      onclick={() => dismiss(toast.id)}
      aria-label={`Fermer : ${toast.text}`}
      class="glass-strong enter-up lift pointer-events-auto w-full rounded-xl px-4 py-3 text-left text-sm shadow-lg {STYLE[
        toast.kind
      ].border}"
    >
      <span class="flex items-start gap-2">
        <Icon name={STYLE[toast.kind].icon} size={16} class="mt-0.5 shrink-0" />
        <span class="min-w-0 break-words whitespace-pre-line">{toast.text}</span>
      </span>
    </button>
  {/each}
</div>

<script lang="ts">
  import Icon from "./Icons.svelte";
  import { originOf, themes, themeStore } from "../lib/theme.svelte";
  import { t } from "../lib/i18n.svelte";

  let { compact = false }: { compact?: boolean } = $props();
</script>

<div class="flex flex-col gap-3">
  <div class="flex flex-wrap gap-2">
    {#each themes() as theme (theme.id)}
      <button
        onclick={(event) => themeStore.setTheme(theme.id, originOf(event))}
        data-tip={theme.hint}
        aria-pressed={themeStore.theme === theme.id}
        class="lift flex items-center gap-2 rounded-xl border px-3 py-2 text-xs font-semibold {themeStore.theme ===
        theme.id
          ? 'border-azure-300/60 bg-surface/85 shadow-sm'
          : 'border-surface/60 bg-surface/45 text-azure-900/60 hover:bg-surface/70'}"
      >
        <span
          class="h-4 w-4 shrink-0 rounded-full shadow-inner"
          style="background: {theme.swatch}; box-shadow: inset 0 0 0 1.5px oklch(1 0 0 / 0.35);"
        ></span>
        {theme.label}
        {#if themeStore.theme === theme.id}
          <Icon name="check" size={13} />
        {/if}
      </button>
    {/each}
  </div>

  {#if !compact}
    <div class="flex items-center gap-2">
      <button
        onclick={(event) => themeStore.dark && themeStore.toggleDark(originOf(event))}
        aria-pressed={!themeStore.dark}
        data-tip={t("theme.mode.light.tip")}
        class="lift flex flex-1 items-center justify-center gap-2 rounded-xl border px-3 py-2 text-xs font-semibold {themeStore.dark
          ? 'border-surface/60 bg-surface/45 text-azure-900/60 hover:bg-surface/70'
          : 'border-azure-300/60 bg-surface/85 shadow-sm'}"
      >
        <Icon name="sun" size={14} />
        {t("theme.mode.light")}
      </button>
      <button
        onclick={(event) => !themeStore.dark && themeStore.toggleDark(originOf(event))}
        aria-pressed={themeStore.dark}
        data-tip={t("theme.mode.dark.tip")}
        class="lift flex flex-1 items-center justify-center gap-2 rounded-xl border px-3 py-2 text-xs font-semibold {themeStore.dark
          ? 'border-azure-300/60 bg-surface/85 shadow-sm'
          : 'border-surface/60 bg-surface/45 text-azure-900/60 hover:bg-surface/70'}"
      >
        <Icon name="moon" size={14} />
        {t("theme.mode.dark")}
      </button>
    </div>
  {/if}
</div>

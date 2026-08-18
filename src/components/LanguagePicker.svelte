<script lang="ts">
  import Icon from "./Icons.svelte";
  import { i18n, LOCALES } from "../lib/i18n.svelte";
</script>

<div class="flex flex-wrap gap-2">
  {#each LOCALES as locale (locale.id)}
    <button
      onclick={() => i18n.setLocale(locale.id)}
      aria-pressed={i18n.locale === locale.id}
      class="lift flex items-center gap-2 rounded-xl border px-3 py-2 text-xs font-semibold {i18n.locale === locale.id
        ? 'border-azure-300/60 bg-surface/85 shadow-sm'
        : 'border-surface/60 bg-surface/45 text-azure-900/60 hover:bg-surface/70'}"
    >
      {locale.label}
      {#if i18n.locale === locale.id}
        <Icon name="check" size={13} />
      {/if}
    </button>
  {/each}
</div>

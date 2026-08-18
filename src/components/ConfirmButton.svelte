<script lang="ts">
  import Icon from "./Icons.svelte";

  let {
    label,
    icon = "",
    confirmLabel = "Confirmer ?",
    onconfirm,
    disabled = false,
    primary = false,
    title = "",
  }: {
    label: string;
    icon?: string;
    confirmLabel?: string;
    onconfirm: () => void | Promise<void>;
    disabled?: boolean;
    primary?: boolean;
    title?: string;
  } = $props();

  let armed = $state(false);
  let running = $state(false);

  async function handleClick() {
    if (running || disabled) return;
    if (!armed) {
      armed = true;
      setTimeout(() => (armed = false), 4000);
      return;
    }
    armed = false;
    running = true;
    try {
      await onconfirm();
    } finally {
      running = false;
    }
  }
</script>

<button
  {disabled}
  data-tip={title || null}
  onclick={handleClick}
  class="lift inline-flex items-center gap-2 rounded-xl px-4 py-2 text-sm font-semibold disabled:cursor-not-allowed disabled:opacity-45 {armed
    ? 'border border-peach/40 bg-peach-soft text-peach-deep shadow-sm'
    : primary
      ? 'sheen bg-gradient-to-br from-azure-500 to-azure-600 text-white shadow-md hover:from-azure-400 hover:to-azure-500 hover:shadow-lg'
      : 'border border-surface/70 bg-surface/60 text-azure-800 hover:border-azure-200 hover:bg-surface/90'}"
>
  {#if running}
    <Icon
      name="refresh"
      size={15}
      tone={primary ? "current" : "duo"}
      class="animate-[lv-spin_0.9s_linear_infinite]"
    />
    En cours…
  {:else if armed}
    <Icon name="alert" size={15} />
    {confirmLabel}
  {:else}
    {#if icon}
      <Icon name={icon} size={15} tone="duo" />
    {/if}
    {label}
  {/if}
</button>

<script lang="ts">
  import Icon from "./Icons.svelte";

  let {
    label,
    icon,
    onclick,
    variant = "ghost",
    disabled = false,
    busy = false,
    busyLabel = "",
    tip = "",
    size = "md",
    full = false,
  }: {
    label: string;
    icon?: string;
    /** Return value is ignored — handlers may be any expression. */
    onclick: () => unknown;
    /** `primary` = the one obvious next step. `danger` = the rose
        vocabulary: an irreversible action, or the repair of a broken state. */
    variant?: "primary" | "ghost" | "soft" | "danger";
    disabled?: boolean;
    busy?: boolean;
    busyLabel?: string;
    tip?: string;
    size?: "sm" | "md";
    full?: boolean;
  } = $props();

  const VARIANT: Record<string, string> = {
    primary:
      "sheen bg-gradient-to-br from-azure-500 to-azure-600 text-white shadow-md hover:from-azure-400 hover:to-azure-500 hover:shadow-lg",
    ghost:
      "border border-surface/70 bg-surface/60 text-azure-800 hover:border-azure-200 hover:bg-surface/90",
    soft: "border border-mint/30 bg-mint-soft/70 text-mint-deep hover:bg-mint-soft",
    danger: "border border-rose/30 bg-rose-soft/60 text-rose-deep hover:bg-rose-soft",
  };

  const SIZE: Record<string, string> = {
    sm: "px-2.5 py-1.5 text-xs gap-1.5",
    md: "px-4 py-2 text-sm gap-2",
  };
</script>

<button
  {disabled}
  data-tip={tip || null}
  onclick={() => void onclick()}
  class="lift inline-flex items-center justify-center rounded-xl font-semibold disabled:cursor-not-allowed disabled:opacity-45 {VARIANT[
    variant
  ]} {SIZE[size]} {full ? 'w-full' : ''}"
>
  {#if busy}
    <Icon
      name="refresh"
      size={size === "sm" ? 13 : 15}
      tone={variant === "primary" ? "current" : "duo"}
      class="animate-[lv-spin_0.9s_linear_infinite]"
    />
    {busyLabel || label}
  {:else}
    {#if icon}
      <Icon
        name={icon}
        size={size === "sm" ? 13 : 15}
        tone={variant === "primary" ? "current" : "duo"}
      />
    {/if}
    {label}
  {/if}
</button>

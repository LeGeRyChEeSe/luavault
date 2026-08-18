<script lang="ts">
  import Icon from "./Icons.svelte";
  import { TONE_CLASS, type Tone } from "../lib/stages";

  let {
    label,
    icon,
    tone = "neutral",
    tip = "",
    live = false,
    compact = false,
  }: {
    label: string;
    icon: string;
    tone?: Tone;
    tip?: string;
    live?: boolean;
    compact?: boolean;
  } = $props();
</script>

<span
  data-tip={tip || null}
  class="enter-pop inline-flex shrink-0 items-center gap-1.5 rounded-full border font-semibold {TONE_CLASS[
    tone
  ]} {compact ? 'px-2 py-0.5 text-[0.68rem]' : 'px-2.5 py-1 text-xs'}"
>
  <span class="relative flex items-center">
    <Icon name={icon} size={compact ? 12 : 14} />
    {#if live}
      <span class="pulse-dot absolute -top-0.5 -right-0.5 h-1.5 w-1.5 rounded-full bg-lilac"></span>
    {/if}
  </span>
  {label}
</span>

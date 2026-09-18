<script lang="ts">
  /**
   * Rend l'arbre produit par `parseMotd` avec de vrais éléments — jamais par
   * injection de balisage (garde N6 de test-news-wiring.ts, qui lit la source
   * brute : ne cite pas la directive interdite, même en commentaire). Le
   * texte du message vient du
   * réseau : signé, mais un mécanisme de rendu ne doit rien supposer de ce
   * qu'il affiche.
   *
   * Les tons nommés passent par les classes `text-<ton>-deep`, qui suivent le
   * mode sombre ; une couleur hexadécimale est appliquée telle quelle, à la
   * charge de l'auteur de la choisir lisible sur les deux fonds.
   */
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { parseMotd } from "../lib/motd-markdown";
  import type { Inline, MotdColor } from "../lib/motd-markdown";

  let { source }: { source: string } = $props();

  const blocks = $derived(parseMotd(source));

  const TONE_CLASS: Record<string, string> = {
    mint: "text-mint-deep",
    peach: "text-peach-deep",
    rose: "text-rose-deep",
    lilac: "text-lilac-deep",
    sky: "text-sky-deep",
  };

  function toneClass(color: MotdColor): string {
    return "tone" in color ? TONE_CLASS[color.tone] : "";
  }

  function hexStyle(color: MotdColor): string | undefined {
    return "hex" in color ? `color: ${color.hex}` : undefined;
  }

  function open(e: MouseEvent, href: string): void {
    e.preventDefault();
    void openUrl(href);
  }
</script>

{#snippet inlines(nodes: Inline[])}
  {#each nodes as node}
    {#if node.kind === "text"}{node.text}
    {:else if node.kind === "break"}<br />
    {:else if node.kind === "code"}<code class="rounded bg-surface/70 px-1 py-0.5 font-mono text-[0.85em]">{node.text}</code>
    {:else if node.kind === "strong"}<strong class="font-semibold">{@render inlines(node.children)}</strong>
    {:else if node.kind === "em"}<em>{@render inlines(node.children)}</em>
    {:else if node.kind === "link"}<a
        href={node.href}
        onclick={(e) => open(e, node.href)}
        class="text-sky-deep underline decoration-sky/50 underline-offset-2 hover:decoration-sky"
        >{@render inlines(node.children)}</a
      >
    {:else if node.kind === "color"}<span class="{toneClass(node.color)} font-medium" style={hexStyle(node.color)}>{@render inlines(node.children)}</span>
    {/if}
  {/each}
{/snippet}

<div class="space-y-3 text-sm leading-relaxed">
  {#each blocks as block}
    {#if block.kind === "heading"}
      {#if block.level === 1}
        <h3 class="text-base font-semibold">{@render inlines(block.children)}</h3>
      {:else if block.level === 2}
        <h4 class="text-sm font-semibold">{@render inlines(block.children)}</h4>
      {:else}
        <h5 class="text-xs font-semibold uppercase tracking-wide opacity-80">{@render inlines(block.children)}</h5>
      {/if}
    {:else if block.kind === "paragraph"}
      <p>{@render inlines(block.children)}</p>
    {:else if block.kind === "list"}
      {#if block.ordered}
        <ol class="list-decimal space-y-1 pl-5">
          {#each block.items as item}
            <li>{@render inlines(item)}</li>
          {/each}
        </ol>
      {:else}
        <ul class="list-disc space-y-1 pl-5">
          {#each block.items as item}
            <li>{@render inlines(item)}</li>
          {/each}
        </ul>
      {/if}
    {:else if block.kind === "quote"}
      <blockquote class="border-l-2 border-azure-300 pl-3 opacity-85">{@render inlines(block.children)}</blockquote>
    {:else if block.kind === "rule"}
      <hr class="border-surface/80" />
    {/if}
  {/each}
</div>

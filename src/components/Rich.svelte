<script lang="ts">
  /**
   * Rend une valeur de catalogue dont un segment est encadré de `**`.
   *
   * Aucun élément enveloppant : le composant rend du contenu INLINE, et
   * c'est l'appelant qui garde son <p> et ses classes.
   */
  import { richSegments } from "../lib/rich-text";

  let { text }: { text: string } = $props();
</script>

<!-- Une seule ligne par convention, pas par nécessité : mesuré au compilateur
     du dépôt, la forme repliée sur plusieurs lignes rend un HTML identique —
     Svelte élague l'espace autour des balises de bloc. Ne réécris donc pas ce
     commentaire en affirmant qu'un saut de ligne ajouterait une espace : c'est
     ce que disait la version précédente, et c'était faux.

     Ce qui est vrai et vérifié (rendu SSR sur les quatre formes, 2026-08-10) :
     le texte rendu est la valeur d'entrée privée de ses marqueurs, <strong>
     entoure le bon segment, et une valeur à marqueur impair sort telle quelle,
     astérisques visibles. -->
{#each richSegments(text) as seg}{#if seg.strong}<strong>{seg.text}</strong>{:else}{seg.text}{/if}{/each}

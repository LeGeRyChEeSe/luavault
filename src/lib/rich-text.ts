/**
 * Découpage d'une valeur de catalogue en segments normaux et en gras.
 *
 * Fonction pure, sans import — donc testée pour de vrai par le runner tsx,
 * et non par une garde textuelle. C'est la leçon du piège n°54 : une garde
 * qui lit la source épingle l'ordre des lignes, jamais la valeur qu'elles
 * portent.
 *
 * Le marqueur est `**`, comme en Markdown. La valeur reste UNE phrase
 * entière — c'est ce qui permet à l'anglais de placer son emphase ailleurs
 * que le français, ce qu'un découpage en fragments interdirait.
 *
 * Ce module ne rend JAMAIS de HTML, et `Rich.svelte` n'emploie jamais
 * `{@html}` : la garde N6 de test-news-wiring.ts l'interdit dans tout
 * `.svelte` du dépôt, et cette interdiction protège le texte tiers affiché
 * par le fil d'actualités. Un mécanisme de typographie ne l'affaiblit pas.
 *
 * Marqueur impair : la valeur est rendue TELLE QUELLE, marqueurs compris.
 * Le défaut devient visible à l'écran plutôt que silencieux — et le cas 45
 * de test-i18n-wiring.ts l'attrape avant qu'un utilisateur le voie.
 */

export type RichSegment = { text: string; strong: boolean };

export function richSegments(value: string): RichSegment[] {
  const parts = value.split("**");

  // Un nombre PAIR de marqueurs découpe en un nombre IMPAIR de morceaux.
  // Sinon la valeur est mal formée : on la rend entière, marqueurs compris.
  if (parts.length % 2 === 0) {
    return [{ text: value, strong: false }];
  }

  const out: RichSegment[] = [];
  for (let i = 0; i < parts.length; i++) {
    if (parts[i] === "") continue;
    out.push({ text: parts[i], strong: i % 2 === 1 });
  }
  return out;
}

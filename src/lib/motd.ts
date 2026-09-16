/**
 * Le message du jour côté interface : choix de la langue, du ton, et la
 * décision d'ouvrir la fenêtre au démarrage. Fonctions pures, testées par
 * valeur dans `scripts/test-motd-markdown.ts`.
 */

import type { MotdMessage } from "./api";

export type MotdSeverity = "info" | "warning" | "critical";

/** Un ton inconnu retombe sur `info` : l'auteur peut se tromper, pas l'écran. */
export function motdSeverity(raw: string | null | undefined): MotdSeverity {
  return raw === "warning" || raw === "critical" ? raw : "info";
}

/**
 * Le texte dans la langue de l'interface, sinon dans la langue neutre (`en`),
 * sinon en français, sinon la première valeur non vide — un message publié
 * dans une seule langue vaut mieux qu'aucun message.
 */
export function resolveMotdText(map: Record<string, string> | null | undefined, locale: string): string {
  if (!map) return "";
  const order = [locale, "en", "fr"];
  for (const key of order) {
    const v = map[key];
    if (typeof v === "string" && v.trim() !== "") return v;
  }
  for (const v of Object.values(map)) {
    if (typeof v === "string" && v.trim() !== "") return v;
  }
  return "";
}

/**
 * Faut-il ouvrir la fenêtre sans que l'utilisateur l'ait demandée ?
 * Jamais pour un identifiant qu'il a demandé d'oublier, jamais deux fois
 * dans une session — la relance passe par le bouton de la barre latérale.
 */
export function shouldOpenMotdAtStartup(
  message: MotdMessage | null,
  dismissedId: string | null,
  seenThisSession: string | null,
): boolean {
  if (!message) return false;
  if (dismissedId === message.id) return false;
  if (seenThisSession === message.id) return false;
  return true;
}

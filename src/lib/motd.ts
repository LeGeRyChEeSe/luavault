/**
 * Le message du jour côté interface : choix de la langue, du ton, et la
 * décision d'ouvrir la fenêtre au démarrage. Fonctions pures, testées par
 * valeur dans `scripts/test-motd-markdown.ts`.
 */

import type { MotdAction, MotdActionKind, MotdMessage, Workaround } from "./api";

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
  forced = false,
): boolean {
  if (!message) return false;
  // A persistent message about a fix this machine still needs cannot be silenced
  // for good: "do not show again" is withdrawn while `forced`.
  if (dismissedId === message.id && !forced) return false;
  if (seenThisSession === message.id) return false;
  return true;
}

/**
 * Does this button close or relaunch Steam, or run a script? Those ask for a
 * second click; opening a web page does not. A kind written down as an exhaustive
 * record, so adding one to `MotdActionKind` fails the type-check until it decides.
 */
const CONFIRMS: Record<MotdActionKind, boolean> = {
  steam_downgrade: true,
  steam_undo: true,
  restart_steam: true,
  repair_tools: true,
  open_url: false,
};

export function actionNeedsConfirm(kind: MotdActionKind): boolean {
  return CONFIRMS[kind];
}

/** The button's own text in the interface language, or `""` for the default of its kind. */
export function actionLabel(action: MotdAction, locale: string): string {
  return resolveMotdText(action.label, locale);
}

/**
 * The live message's Steam downgrade button, if it carries one. The tools tab
 * offers its card only while this is non-null: once the author withdraws the
 * message — the add-on works with current Steam again — the card goes with it,
 * and nothing about the workaround stays compiled into the app.
 */
export function steamDowngradeOffer(message: MotdMessage | null): MotdAction | null {
  return message?.actions?.find((a) => a.kind === "steam_downgrade") ?? null;
}

/**
 * Is this notice insisting? Only a persistent message whose fix this machine still
 * needs: for everyone else it is an ordinary, dismissible notice.
 */
export function isForced(message: MotdMessage | null, workaround: Workaround | null): boolean {
  return !!message?.persistent && !!workaround?.needed;
}

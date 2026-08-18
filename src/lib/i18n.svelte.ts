import { setLocale as persistLocale } from "./api";
import { fr, type Key } from "./i18n/fr";
import { en } from "./i18n/en";

/**
 * The languages offered, each labelled **in its own language** — "English",
 * never "Anglais". No flag emoji: the charter forbids emojis, and a flag names
 * a country, not a language.
 *
 * These labels are the one set of strings that must NOT be translated, which is
 * why this stays a module constant while `STAGES`, `THEMES` and `CREDITS` have
 * to become functions.
 */
const LOCALES = [
  { id: "fr", label: "Français" },
  { id: "en", label: "English" },
] as const;

export type LocaleId = (typeof LOCALES)[number]["id"];

/**
 * Catalogues, all imported statically. Lazy loading would buy a few kilobytes
 * against an `await` at switch time — that is, against the flicker-free change
 * the whole feature exists for.
 *
 * Typing this `Record<LocaleId, …>` is what makes the runtime unreachable:
 * adding an entry to `LOCALES` without its catalogue is a `svelte-check`
 * error, i.e. a failing `validate` step. That is the guard — not a `??` whose
 * fallback branch no test could ever reach.
 */
const CATALOGS: Record<LocaleId, Record<Key, string>> = { fr, en };

/** La langue de référence du catalogue : toute autre s'écrit d'après elle. */
const REFERENCE: LocaleId = "fr";
/** Où atterrit un système dont la langue n'est pas livrée (décidé le 2026-08-09). */
export const NEUTRAL: LocaleId = "en";

/**
 * Langue du système, lue **à chaque appel** — jamais gelée dans une const de
 * module, sinon un changement de langue Windows ne serait jamais repris et
 * aucun test ne pourrait faire varier la valeur simulée d'un cas à l'autre.
 *
 * Lit `navigator.languages` (ou `navigator.language` en repli) et rend la
 * première langue **livrée** qu'on y rencontre, sinon `NEUTRAL`.
 *
 * La correspondance se fait contre `LOCALES`, jamais contre une liste écrite à
 * la main : le plan i18n promet qu'ajouter une langue est « un fichier et zéro
 * ligne de logique », et deux branches codées en dur ici suffiraient à rendre
 * cette promesse fausse — en silence, puisque le typage ne surveille que les
 * catalogues. Le dernier cas de `test-i18n-state.ts` est la garde qui le tient.
 */
export function systemLocale(): LocaleId {
  const list = navigator.languages?.length
    ? navigator.languages
    : navigator.language
      ? [navigator.language]
      : [];

  for (const tag of list) {
    // "fr-BE", "fr" et "FR" désignent tous le français : seul le sous-tag
    // primaire porte la langue.
    const primary = tag.toLowerCase().split("-")[0];
    const hit = LOCALES.find((l) => l.id === primary);
    if (hit) return hit.id;
  }

  return NEUTRAL;
}

function isLocaleId(id: unknown): id is LocaleId {
  return LOCALES.some((l) => l.id === id);
}

/**
 * Reactive i18n store. Mutates the locale and persists immediately — no view
 * transition, no reload. `{t("…")}` in any template reacts because it reads
 * `i18n.locale` on every evaluation.
 *
 * Deliberately NOT routed through `startViewTransition`: that API does not run
 * its callback synchronously, so anything persisting after the call saves the
 * state from *before* the mutation (the bug that made the theme revert on the
 * next launch). Here the new id is passed to `persistLocale` by value, never
 * read back from the store, so the ordering trap cannot reappear.
 */
class I18nStore {
  locale = $state<LocaleId>(REFERENCE);
  /**
   * True until the saved locale has been read. **Nothing gates on it yet** —
   * said plainly because `ThemeStore.loading` carries the comment "avoids a
   * flash of default" while no view reads it either, and a comment asserting a
   * property the code does not have is what stops anyone writing the real
   * guard. Whoever adds the anti-flicker gate adds it for both at once.
   */
  loading = $state(true);

  /** Adopt the persisted locale. `null` or unknown values fall back to the system. */
  hydrate(locale: string | null | undefined) {
    this.locale = isLocaleId(locale) ? locale : systemLocale();
    this.loading = false;
  }

  /**
   * Swap locale and persist. An id we do not ship is refused outright: letting
   * it through would set a locale whose catalogue does not exist, show French
   * anyway, write the unknown id to `config.json`, and have the next launch
   * silently revert the user's choice.
   */
  setLocale(id: string) {
    if (!isLocaleId(id) || id === this.locale) return;
    this.locale = id;
    void persistLocale(id).catch(() => {
      // Persistence is best-effort — a failed save must never surface.
    });
  }

  /** Look up `key` in the current catalogue and interpolate every `{name}`. */
  t(key: Key, params?: Record<string, unknown>): string {
    let text: string = CATALOGS[this.locale][key];
    if (params) {
      for (const [name, value] of Object.entries(params)) {
        text = text.split(`{${name}}`).join(String(value));
      }
    }
    return text;
  }
}

export const i18n = new I18nStore();

/** Shorthand — always reads `i18n.locale` so templates stay reactive. */
export function t(key: Key, params?: Record<string, unknown>): string {
  return i18n.t(key, params);
}

export { LOCALES };
export type { Key };

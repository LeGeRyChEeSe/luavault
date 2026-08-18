import { setAppearance } from "./api";

/**
 * A theme is a hue, not a stylesheet: `app.css` derives every ramp from
 * `--lv-h` / `--lv-c`, so switching one is writing one attribute. The swatch
 * colours here are only for the picker itself.
 */
export interface Theme {
  id: string;
  label: string;
  /** Short reason to pick this one, shown under the swatch. */
  hint: string;
  /** Preview dot, in the same OKLCH space the ramps use. */
  swatch: string;
}

import { t } from "./i18n.svelte";

/** Theme metadata only — no text. Labels and hints come from the i18n catalogue. */
export const THEME_DEFS = [
  { id: "azur", swatch: "oklch(0.64 0.15 240)" },
  { id: "emeraude", swatch: "oklch(0.64 0.14 168)" },
  { id: "orchidee", swatch: "oklch(0.64 0.15 315)" },
  { id: "ambre", swatch: "oklch(0.68 0.15 62)" },
  { id: "ardoise", swatch: "oklch(0.6 0.03 255)" },
] as const;

/**
 * Metadata + text for a theme, composed at call time so a language switch
 * re-renders every swatch button — the whole point of this file's migration.
 */
export function themes(): Theme[] {
  return [...THEME_DEFS].map((def) => ({
    ...def,
    label: t(`theme.${def.id}.label`),
    hint: t(`theme.${def.id}.hint`),
  }));
}

const DEFAULT_THEME = "azur";

function systemPrefersDark(): boolean {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

/**
 * Live appearance state. Applied to `<html>` as `data-theme` / `data-mode`,
 * which is all `app.css` needs, and mirrored into the Rust config so the next
 * launch paints the right colours on the first frame.
 */
class ThemeStore {
  theme = $state(DEFAULT_THEME);
  dark = $state(false);
  /** True until the saved values have been read — avoids a flash of default. */
  loading = $state(true);

  /** Adopt the persisted appearance. `null` values fall back to the system. */
  hydrate(theme: string | null | undefined, dark: boolean | null | undefined) {
    // `def`, not `t`: the translation function is imported into this module, and
    // a callback parameter named `t` shadows it. Harmless today because nothing
    // here translates — a landmine the day someone adds a `t(...)` inside.
    this.theme = THEME_DEFS.some((def) => def.id === theme) ? (theme as string) : DEFAULT_THEME;
    this.dark = dark ?? systemPrefersDark();
    this.loading = false;
    this.paint();
  }

  private paint() {
    const root = document.documentElement;
    root.dataset.theme = this.theme;
    root.dataset.mode = this.dark ? "dark" : "light";
  }

  private persist() {
    void setAppearance(this.theme, this.dark).catch(() => {
      // Appearance is cosmetic — a failed save must never surface as an error.
    });
  }

  /**
   * Swap the palette with a circular wipe starting from the control that was
   * clicked. Falls back to a plain repaint where View Transitions are missing
   * or when the user asked for reduced motion.
   */
  private transition(mutate: () => void, origin?: { x: number; y: number }) {
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    const start = (document as Document & {
      startViewTransition?: (cb: () => void) => unknown;
    }).startViewTransition;

    if (reduced || typeof start !== "function") {
      mutate();
      this.paint();
      this.persist();
      return;
    }

    const root = document.documentElement;
    root.style.setProperty("--lv-wipe-x", `${origin?.x ?? innerWidth / 2}px`);
    root.style.setProperty("--lv-wipe-y", `${origin?.y ?? innerHeight / 2}px`);
    // `persist()` belongs INSIDE the callback. `startViewTransition` does not
    // run its callback synchronously — it captures the old frame first — so a
    // `persist()` placed after this call reads the state *before* `mutate()`
    // and saves the previous appearance. The screen showed the new theme and
    // the next launch restored the old one; the graphical bench measured
    // `dark_mode: true` in config.json while the window read "light".
    start.call(document, () => {
      mutate();
      this.paint();
      this.persist();
    });
  }

  setTheme(id: string, origin?: { x: number; y: number }) {
    if (id === this.theme) return;
    this.transition(() => (this.theme = id), origin);
  }

  toggleDark(origin?: { x: number; y: number }) {
    this.transition(() => (this.dark = !this.dark), origin);
  }
}

export const themeStore = new ThemeStore();

/**
 * Centre of the element that triggered a switch, for the wipe origin.
 *
 * The element's own rect is correct for mouse clicks and keyboard activation
 * alike, so it always wins when present. Only when there is no element at all do
 * we fall back: to the viewport centre for keyboard activation (`detail === 0`,
 * where `clientX`/`clientY` are both meaningless zeros), to the pointer otherwise.
 */
export function originOf(event: MouseEvent): { x: number; y: number } {
  const target = event.currentTarget as HTMLElement | null;
  if (target) {
    const box = target.getBoundingClientRect();
    return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
  }
  if (event.detail === 0) {
    return { x: window.innerWidth / 2, y: window.innerHeight / 2 };
  }
  return { x: event.clientX, y: event.clientY };
}

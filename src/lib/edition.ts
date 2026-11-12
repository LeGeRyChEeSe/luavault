/**
 * Which edition this frontend was built for.
 *
 * The value is frozen at build time by `vite.config.js` (`define` on
 * `import.meta.env.VITE_LV_EDITION`, set by `scripts/build-exp.ps1`), so the UI
 * knows its edition before any command answers. The Rust binary carries the same
 * fact as `config::EXPERIMENTAL`; `appEdition()` is how the two are compared at
 * startup (`app-state.svelte.ts`), and a disagreement is logged, never repaired:
 * it means a bundle was built for one edition and embedded in the other.
 *
 * Pure on purpose: `editionFrom`, `editionsAgree` and `showChannelChoice` are what
 * the unit runner calls (`scripts/test-edition-wiring.ts`).
 */

export type Edition = "stable" | "experimental";

/** Normalise whatever the build put there: only the exact name is experimental. */
export function editionFrom(raw: unknown): Edition {
  return raw === "experimental" ? "experimental" : "stable";
}

/** True when the frontend's edition and the binary's channel are the same. */
export function editionsAgree(front: Edition, backendChannel: string): boolean {
  return front === editionFrom(backendChannel);
}

/** The edition of this bundle. Outside Vite (the unit runner) `import.meta.env`
 *  is undefined, which reads as stable. */
export const EDITION: Edition = editionFrom(
  (import.meta as unknown as { env?: { VITE_LV_EDITION?: unknown } }).env?.VITE_LV_EDITION,
);

export const isExperimental: boolean = EDITION === "experimental";

/**
 * Whether Settings shows the update-channel choice (and, on the stable, the
 * « install beside » button). An experimental edition that does not exist must
 * not be offered: picking it would leave the user with no update at all, in
 * silence. So the choice appears only while an experimental version is actually
 * published (`offered`), disappears when it is withdrawn, and comes back with the
 * next one. It stays visible regardless for the experimental binary, and for a
 * stable already on that channel: both need the way back.
 */
export function showChannelChoice(e: {
  experimental: boolean;
  channel: "stable" | "experimental" | null;
  offered: boolean;
}): boolean {
  return e.experimental || e.channel === "experimental" || e.offered;
}

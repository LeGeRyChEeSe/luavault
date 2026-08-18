/**
 * Toast visibility durations (LOT-19, task 1). Pure module — the runes
 * store (app-state.svelte.ts) applies the resolved value; the tests in
 * test-virtual-scroll.ts pin default, override and invalid fallbacks.
 */

/** Every toast stays visible at least this long unless the caller says otherwise. */
export const DEFAULT_TOAST_MS = 5200;

/** For messages that take real reading time (announcements, long errors). */
export const LONG_TOAST_MS = 9000;

/**
 * The caller's duration, or the default when it is absent or invalid.
 * Only a finite, strictly positive number of milliseconds is accepted —
 * NaN, Infinity, zero and negatives all fall back, so a bad override can
 * never hide a toast instantly or pin it forever.
 */
export function resolveToastDuration(ms: number | undefined | null): number {
  if (typeof ms === "number" && Number.isFinite(ms) && ms > 0) return ms;
  return DEFAULT_TOAST_MS;
}

/**
 * Pure sorting logic for the library view.
 *
 * Lives in a plain `.ts` file — `library-state.svelte.ts` carries runes and
 * cannot be imported by the tsx test runner; re-exported from there so the
 * components keep their imports.
 */
import type { GameStatus } from "./api";
import { STAGE_ORDER } from "./stages";

/** Sort mode for the library view. */
export type LibrarySort =
  | "name_asc"
  | "name_desc"
  | "added_desc"
  | "added_asc"
  | "stage"
  | "fix_first"
  | "playtime";

/** Pure function — does not mutate the input array.
 *
 * Always falls back to `localeCompare` on name (fr, base sensitivity) and
 * finally on `app_id` so that two games with the same stage never swap
 * positions on a refresh.
 */
export function sortStatuses(list: GameStatus[], sort: LibrarySort): GameStatus[] {
  return [...list].sort((a, b) => {
    let cmp = 0;

    switch (sort) {
      case "name_asc":
        cmp = a.name.localeCompare(b.name, "fr", { sensitivity: "base" });
        break;
      case "name_desc":
        cmp = b.name.localeCompare(a.name, "fr", { sensitivity: "base" });
        break;
      case "added_desc":
        cmp = compareDate(a.added_at, b.added_at);
        break;
      case "added_asc":
        cmp = compareDate(b.added_at, a.added_at);
        break;
      case "stage": {
        const idxA = STAGE_ORDER.indexOf(a.stage);
        const idxB = STAGE_ORDER.indexOf(b.stage);
        cmp = (idxA === -1 ? STAGE_ORDER.length : idxA)
          - (idxB === -1 ? STAGE_ORDER.length : idxB);
        break;
      }
      case "fix_first":
        cmp = (b.fix_downloaded ? 1 : 0) - (a.fix_downloaded ? 1 : 0);
        break;
      case "playtime": {
        // Descending on KNOWN minutes. Games without data (`null` — "on ne
        // sait pas", not zero) go LAST, never first: a descending sort that
        // leads with dataless games reads as a broken sort.
        const minutesA = a.playtime_minutes;
        const minutesB = b.playtime_minutes;
        if (minutesA == null && minutesB == null) {
          cmp = 0;
        } else if (minutesA == null) {
          cmp = 1;
        } else if (minutesB == null) {
          cmp = -1;
        } else {
          cmp = minutesB - minutesA;
        }
        break;
      }
    }

    // Stable tie-breaker: alphabetical name (fr), then app_id.
    if (cmp === 0) {
      cmp = a.name.localeCompare(b.name, "fr", { sensitivity: "base" });
    }
    if (cmp === 0) {
      return a.app_id < b.app_id ? -1 : a.app_id > b.app_id ? 1 : 0;
    }
    return cmp;
  });
}

/** Milliseconds since epoch, or 0 when the date is missing or unreadable.
 * An unreadable date must never produce NaN: a comparator returning NaN is
 * not transitive and scrambles the whole list, not just the faulty entry.
 */
function dateKey(v: string | null | undefined): number {
  if (!v) return 0;
  const t = new Date(v).getTime();
  return Number.isNaN(t) ? 0 : t;
}

/** Compare two optional RFC-3339 dates. */
function compareDate(a: string | null | undefined, b: string | null | undefined): number {
  return dateKey(b) - dateKey(a);
}

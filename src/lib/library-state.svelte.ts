import type { GameStatus } from "./api";
import type { LibrarySort } from "./library-sort";

// The pure sorting logic lives in library-sort.ts (a rune-free module the
// tsx test runner can import); this file re-exports it so every existing
// import keeps working unchanged.
export type { LibrarySort } from "./library-sort";
export { sortStatuses } from "./library-sort";

// Same arrangement for the fifth bulk mode (LOT-16): the pure selection
// logic lives in selection.ts, this file holds the state and re-exports.
export type { SelectionAction } from "./selection";
export {
  eligibleForSelectionAction,
  purgeSelection,
  selectAllVisible,
  deselectVisible,
  withAddedTag,
  withRemovedTag,
} from "./selection";

/** Filter mode for the library view. */
export type LibraryFilter = "all" | "attention" | "fix" | "ready";

/**
 * The library tab's state lives here, outside the component.
 *
 * Svelte destroys a view when you navigate away, which used to throw away the
 * search query, the filter and the sort. Keeping the state in a module means
 * leaving the tab and coming back restores the exact same screen.
 */
class LibraryStore {
  search = $state("");
  filter = $state<LibraryFilter>("all");
  sort = $state<LibrarySort>("added_desc");
  selectedTags = $state<string[]>([]);
  /** LOT-16 — multi-select. AppIDs, purged of entries that leave the
      library (see the `purgeSelection` effect in LibraryView). */
  selection = $state<string[]>([]);
  /** LOT-16 — checkboxes only clutter the view while this is on. Leaving
      the mode clears the selection: an invisible selection a pass could
      still treat is the hidden-games trap all over again. */
  selectionMode = $state(false);
}

/** Every tag used in the library, deduplicated on the lowercase key but
 * returning the first spelling encountered, sorted alphabetically (fr). */
export function allTags(list: GameStatus[]): string[] {
  const map = new Map<string, string>(); // lowercase key → first spelling
  for (const s of list) {
    for (const t of s.tags) {
      const key = t.toLowerCase();
      if (!map.has(key)) {
        map.set(key, t);
      }
    }
  }
  return [...map.values()].sort((a, b) => a.localeCompare(b, "fr"));
}

/** Games carrying ALL the selected tags. An empty selection matches everything.
 * Comparison is case-insensitive. */
export function filterByTags(
  list: GameStatus[],
  selected: string[],
): GameStatus[] {
  if (selected.length === 0) return list;
  const selectedLower = selected.map((t) => t.toLowerCase());
  return list.filter((s) =>
    selectedLower.every((sel) =>
      s.tags.some((t) => t.toLowerCase() === sel),
    ),
  );
}

export const libraryState = new LibraryStore();

import type { GameStatus } from "./api";

/**
 * Pure logic of the fifth bulk mode — multi-select and the management
 * actions applied to it. Rune-free on purpose: the tsx test runner imports
 * this file directly (same arrangement as library-sort.ts).
 *
 * The selection itself lives in library-state.svelte.ts, with the search,
 * the filter and the sort — leaving the tab and coming back restores it.
 */

/** Management actions that make sense on ten games at once. Launching is
    deliberately absent: Steam starts one game per `steam://rungameid`, and
    ten of them would be ten windows fighting for focus. */
export type SelectionAction =
  | "fixes"
  | "verify"
  | "copy"
  | "add-tag"
  | "remove-tag"
  | "hide";

/** Stages the selection patch pass treats — the mirror of the backend's
    `INSTALLABLE_FIX_STAGES` (commands.rs). `fix_external` is absent on
    purpose: the app holds no backup of those files and must never
    overwrite them. Being in one of these stages is not sufficient on its
    own — `fix_downloaded` and `fully_installed` are checked beside it,
    exactly like the backend's `is_fix_candidate`. */
export const SELECTION_FIX_STAGES = [
  "fix_downloaded",
  "fix_damaged",
  "fix_game_moved",
];

/** Case-insensitive tag test — the convention of `filterByTags` and of
    `library::normalize_tags` on the Rust side. */
export function hasTag(status: GameStatus, tag: string): boolean {
  const wanted = tag.trim().toLowerCase();
  if (!wanted) return false;
  return status.tags.some((t) => t.toLowerCase() === wanted);
}

/** The games of `list` a selection action will treat. These are the same
    conditions the context menu applies per game, and for `"fixes"` the
    same selection the backend pass runs on — the bar's counts come from
    here, so a button never announces a game the pass won't treat.
    `tag` matters only for the two tag actions. */
export function eligibleForSelectionAction(
  list: GameStatus[],
  action: SelectionAction,
  tag = "",
): GameStatus[] {
  const cleaned = tag.trim();
  switch (action) {
    case "fixes":
      return list.filter(
        (s) =>
          s.fix_downloaded &&
          s.game.fully_installed &&
          SELECTION_FIX_STAGES.includes(s.stage),
      );
    case "verify":
      // The context menu's "Vérifier le patch": downloaded + installed.
      return list.filter((s) => s.fix_downloaded && s.game.fully_installed);
    case "copy":
      return list.filter((s) => s.in_library && !s.lua_in_steam);
    case "add-tag":
      if (!cleaned) return [];
      return list.filter((s) => s.in_library && !hasTag(s, cleaned));
    case "remove-tag":
      if (!cleaned) return [];
      return list.filter((s) => s.in_library && hasTag(s, cleaned));
    case "hide":
      return list.filter((s) => s.in_library && !s.hidden);
  }
}

/** The selection minus the AppIDs that left the library — the mirror of
    the `selectedTags` purge in LibraryView. Hidden games stay: they are
    still in the library, only out of sight, and every count already
    excludes them because it runs on what the view shows. */
export function purgeSelection(
  selection: string[],
  library: GameStatus[],
): string[] {
  const present = new Set(library.map((s) => s.app_id));
  return selection.filter((id) => present.has(id));
}

/** "Select all" stops at the filter: it adds exactly what is on screen
    after search, filter and tags. Selecting games a filter hides would
    recreate the hidden-games trap — an invisible selection a pass still
    treats. */
export function selectAllVisible(
  shown: GameStatus[],
  selection: string[],
): string[] {
  const already = new Set(selection);
  return [
    ...selection,
    ...shown.filter((s) => !already.has(s.app_id)).map((s) => s.app_id),
  ];
}

/** "Deselect all" is the mirror image: it clears exactly what is on
    screen. Selected games a filter hides keep their place — inert until
    visible again, never processed while invisible. */
export function deselectVisible(
  shown: GameStatus[],
  selection: string[],
): string[] {
  const visible = new Set(shown.map((s) => s.app_id));
  return selection.filter((id) => !visible.has(id));
}

/** Add a tag the way `library::normalize_tags` will see it: trimmed, and
    deduplicated case-insensitively against what the game already carries. */
export function withAddedTag(tags: string[], tag: string): string[] {
  const cleaned = tag.trim();
  if (!cleaned) return tags;
  if (tags.some((t) => t.toLowerCase() === cleaned.toLowerCase())) return tags;
  return [...tags, cleaned];
}

export function withRemovedTag(tags: string[], tag: string): string[] {
  const lower = tag.trim().toLowerCase();
  return tags.filter((t) => t.toLowerCase() !== lower);
}

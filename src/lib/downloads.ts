import type { DownloadFound } from "./api";

/** Why a recognised file cannot be added yet. Each one is an i18n key suffix. */
export type DownloadBlock = "archive.no-app-id" | "archive.unknown-game";

/** The catalogue key that explains each block. Literal keys, so the i18n guards see them. */
export const BLOCK_KEYS: Record<DownloadBlock, "downloads.reason.archive.no-app-id" | "downloads.reason.archive.unknown-game"> = {
  "archive.no-app-id": "downloads.reason.archive.no-app-id",
  "archive.unknown-game": "downloads.reason.archive.unknown-game",
};

export type DownloadPlan =
  | { importable: true; gameName: string | null }
  | { importable: false; reason: DownloadBlock };

/**
 * What adding this file would do. A `.lua` always can: its AppID comes from its
 * content. A patch archive is stored beside the game it belongs to, so the game
 * must already be in the library — the same rule `import_patch_archive` enforces
 * (the backend stays the judge; this only keeps the popup from offering a click
 * that is certain to fail).
 *
 * `library` maps an AppID to the game's name. `ticked` holds the AppIDs of the `.lua`
 * files chosen in the same list: they are added first, so a patch for a game whose
 * `.lua` is ticked beside it can be added in the same gesture.
 */
export function planDownload(
  item: DownloadFound,
  library: ReadonlyMap<string, string>,
  ticked: ReadonlySet<string> = new Set(),
): DownloadPlan {
  if (item.kind === "lua") {
    return { importable: true, gameName: item.app_id ? (library.get(item.app_id) ?? null) : null };
  }
  if (!item.app_id) return { importable: false, reason: "archive.no-app-id" };
  const name = library.get(item.app_id);
  if (name !== undefined) return { importable: true, gameName: name };
  return ticked.has(item.app_id)
    ? { importable: true, gameName: null }
    : { importable: false, reason: "archive.unknown-game" };
}

/** The order the files are added in: every `.lua` first, so the games exist for their patches. */
export function importOrder(items: readonly DownloadFound[]): DownloadFound[] {
  return [...items.filter((item) => item.kind === "lua"), ...items.filter((item) => item.kind !== "lua")];
}

/** The catalogue key naming what a file is. Literal keys, so the i18n guards see them. */
export function flavorKey(item: DownloadFound): "downloads.kind.lua" | "downloads.kind.patch" | "downloads.kind.bypass" {
  if (item.kind === "lua") return "downloads.kind.lua";
  return item.flavor === "bypass" ? "downloads.kind.bypass" : "downloads.kind.patch";
}

/**
 * Fold a batch of newly recognised files into the pending list. A path is listed
 * once: if it comes again (the file was rewritten) the newer description wins and
 * keeps its place.
 */
export function mergeFound(queue: readonly DownloadFound[], incoming: readonly DownloadFound[]): DownloadFound[] {
  const merged = [...queue];
  for (const item of incoming) {
    const at = merged.findIndex((known) => known.path === item.path);
    if (at === -1) merged.push(item);
    else merged[at] = item;
  }
  return merged;
}

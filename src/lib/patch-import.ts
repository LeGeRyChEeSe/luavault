export type PatchFailure =
  | { key: "library.patch.failed.backend"; params: { error: string } }
  | { key: "library.patch.failed"; params: { names: string } };

/** Select the translated failure message without changing the backend error semantics. */
export function patchFailureFor(error: unknown, summarizedName: string): PatchFailure {
  const message = error instanceof Error ? error.message : typeof error === "string" ? error : "";
  return message
    ? { key: "library.patch.failed.backend", params: { error: message } }
    : { key: "library.patch.failed", params: { names: summarizedName } };
}

/**
 * Read an AppID only from a deliberate filename form: `123.zip`,
 * `123_online_fix.zip`, `123_bypass.rar`, or `Game name (123).zip`.
 * Bare digits elsewhere must never select a game by accident.
 */
export function patchAppIdFromFilename(path: string): string | null {
  const filename = path.split(/[\\/]/).pop() ?? path;
  const stem = filename.replace(/\.[^.]+$/, "");
  const plain = plainStemAppId(stem);
  if (plain) return plain;
  // A browser that finds the name taken saves `123_bypass (1).rar`: one or two digits after a
  // space are a counter, not a game. Longer ones stay the `Name (AppID)` form.
  const counter = /^(.*) \((\d{1,2})\)$/.exec(stem);
  if (counter) {
    const base = plainStemAppId(counter[1].trim());
    if (base) return base;
  }
  // Three digits at least: `video (1).zip` is a browser's duplicate, not the game 1.
  return /\((\d{3,})\)$/.exec(stem)?.[1] ?? null;
}

/** The words that may follow the AppID in `123_<word>.zip`. A closed list, as in Rust. */
const PATCH_NAME_SUFFIXES = ["online_fix", "onlinefix", "bypass", "drm_bypass", "drm", "fix", "patch"];

function plainStemAppId(stem: string): string | null {
  if (/^\d+$/.test(stem)) return stem;
  const named = /^(\d+)_(.+)$/.exec(stem);
  return named && PATCH_NAME_SUFFIXES.includes(named[2].toLowerCase()) ? named[1] : null;
}

/** Archives accepted by the native picker; Rust validates their magic bytes. */
export function isPatchArchivePath(path: string): boolean {
  return /\.(zip|rar|7z)$/i.test(path);
}

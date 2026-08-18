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
 * `123_online_fix.zip`, or `Game name (123).zip`.
 * Bare digits elsewhere must never select a game by accident.
 */
export function patchAppIdFromFilename(path: string): string | null {
  const filename = path.split(/[\\/]/).pop() ?? path;
  const stem = filename.replace(/\.[^.]+$/, "");
  if (/^\d+$/.test(stem)) return stem;
  const onlineFix = /^(\d+)_online_fix$/.exec(stem)?.[1];
  if (onlineFix) return onlineFix;
  return /\((\d+)\)$/.exec(stem)?.[1] ?? null;
}

/** Archives accepted by the native picker; Rust validates their magic bytes. */
export function isPatchArchivePath(path: string): boolean {
  return /\.(zip|rar|7z)$/i.test(path);
}

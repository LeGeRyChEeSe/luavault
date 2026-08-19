import { revealItemInDir } from "@tauri-apps/plugin-opener";

/** The single, Explorer-oriented folder opener used throughout the interface. */
export type FolderRevealer = (path: string) => Promise<void>;

/**
 * Reveal a folder in the platform file explorer.
 *
 * Keeping the Tauri call behind this small seam makes every folder action use
 * the same OS behaviour and lets its contract be exercised without a WebView.
 */
export function openFolder(path: string, reveal: FolderRevealer = revealItemInDir): Promise<void> {
  return reveal(path);
}

export type ErrorToast = (kind: "error", message: string) => void;

/**
 * SteamTools lives in the Steam folder. Unlike the incidental folder actions,
 * this entry point reports a failed Explorer launch because it is the only
 * way to reach that installation from Settings.
 */
export async function openSteamtoolsFolder(
  path: string,
  errorMessage: string,
  toast: ErrorToast,
  opener: (path: string) => Promise<void> = openFolder,
): Promise<void> {
  try {
    await opener(path);
  } catch {
    toast("error", errorMessage);
  }
}

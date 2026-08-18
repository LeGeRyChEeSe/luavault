/**
 * LOT-14 — artwork resolution: the disk cache first, the network only as a
 * fallback, and never any request when offline. The resolved value is an
 * `http://asset.localhost/…` URL (convertFileSrc) served straight from disk
 * by the asset protocol — no IPC payload, no CDN round-trip once cached.
 *
 * A failure resolves to `null` — the caller shows the neutral tile. Nothing
 * is cached on failure, neither here (the in-flight map is always cleaned)
 * nor on disk (the backend writes temp-then-rename, see artwork.rs).
 */
import { convertFileSrc } from "@tauri-apps/api/core";
import { artworkCached, artworkFetch } from "./api";
import { appState } from "./app-state.svelte";

/** One download per URL at a time: two cards showing the same image must
 *  not fetch it twice. The entry is removed as soon as it settles, so a
 *  failure is never remembered — the next attempt is free to retry. */
const inFlight = new Map<string, Promise<string | null>>();

export function resolveArtwork(url: string): Promise<string | null> {
  const pending = inFlight.get(url);
  if (pending) return pending;
  const task = (async () => {
    try {
      // The cache is consulted even offline — that IS the offline mode.
      const cached = await artworkCached(url);
      if (cached) return convertFileSrc(cached);
      // Offline with nothing on disk: no request, the tile takes over.
      if (!appState.online) return null;
      const fetched = await artworkFetch(url);
      return convertFileSrc(fetched);
    } catch {
      return null;
    } finally {
      inFlight.delete(url);
    }
  })();
  inFlight.set(url, task);
  return task;
}

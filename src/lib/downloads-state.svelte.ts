import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  DOWNLOADS_FOUND_EVENT,
  DOWNLOADS_STATE_EVENT,
  downloadsWatchStart,
  downloadsWatchStatus,
  downloadsWatchStop,
  importLuaFile,
  importPatchArchive,
} from "./api";
import type { DownloadFound, DownloadsWatchStatus } from "./api";
import { appState } from "./app-state.svelte";
import { BLOCK_KEYS, mergeFound, planDownload } from "./downloads";
import { t } from "./i18n.svelte";
import { patchFailureFor } from "./patch-import";

/** How long a listening started from the button lasts. */
export const WATCH_MINUTES = 10;

const IDLE: DownloadsWatchStatus = { active: false, ends_in_secs: null };

/** What the downloads watcher found, waiting for the user's yes or no. */
class DownloadsState {
  /** Recognised files not yet answered. The popup shows while this is non-empty. */
  pending = $state<DownloadFound[]>([]);
  watch = $state<DownloadsWatchStatus>(IDLE);
  /** When a timed listening ends (ms since epoch); `null` when none is running. */
  endsAt = $state<number | null>(null);
  busy = $state(false);

  private unlisten: UnlistenFn[] = [];

  private apply(status: DownloadsWatchStatus) {
    this.watch = status;
    this.endsAt = status.active && status.ends_in_secs !== null ? Date.now() + status.ends_in_secs * 1000 : null;
  }

  /** Subscribe once. The watcher keeps running in Rust; this only listens. */
  async init() {
    if (this.unlisten.length > 0) return;
    this.unlisten = await Promise.all([
      listen<DownloadFound[]>(DOWNLOADS_FOUND_EVENT, (event) => {
        this.pending = mergeFound(this.pending, event.payload);
      }),
      listen<DownloadsWatchStatus>(DOWNLOADS_STATE_EVENT, (event) => this.apply(event.payload)),
    ]);
    try {
      this.apply(await downloadsWatchStatus());
    } catch {
      this.apply(IDLE);
    }
  }

  dispose() {
    for (const off of this.unlisten) off();
    this.unlisten = [];
  }

  async start() {
    try {
      this.apply(await downloadsWatchStart(WATCH_MINUTES));
      appState.toast("info", t("downloads.watch.started", { minutes: WATCH_MINUTES }));
    } catch (e) {
      appState.toast("error", String(e));
    }
  }

  async stop() {
    try {
      this.apply(await downloadsWatchStop());
    } catch (e) {
      appState.toast("error", String(e));
    }
  }

  /** "Ignorer": forget what is pending. The files are left exactly where they are. */
  dismiss() {
    this.pending = [];
  }

  /**
   * Add the chosen files through the library's own import commands, one at a time
   * (each write signs the shared index). The popup is the confirmation, so the
   * native "are you sure" the manual patch import asks is not asked twice.
   */
  async add(paths: string[]) {
    const chosen = this.pending.filter((item) => paths.includes(item.path));
    this.pending = [];
    if (chosen.length === 0) return;
    const library = new Map(appState.statuses.map((game) => [game.app_id, game.name]));
    this.busy = true;
    let luaCount = 0;
    const patched: string[] = [];
    const failures: string[] = [];
    try {
      for (const item of chosen) {
        try {
          if (item.kind === "lua") {
            await importLuaFile(item.path);
            luaCount++;
          } else {
            const plan = planDownload(item, library);
            if (!plan.importable) throw new Error(t(BLOCK_KEYS[plan.reason], { appId: item.app_id ?? "" }));
            await importPatchArchive(item.path, item.app_id ?? undefined);
            patched.push(plan.gameName ?? item.name);
          }
        } catch (error) {
          const failure = patchFailureFor(error, item.name);
          failures.push(item.kind === "lua" ? t("library.import.failed", { names: item.name }) : t(failure.key, failure.params));
        }
      }
      if (luaCount > 0) appState.toast("success", t("library.import.done", { count: luaCount }));
      if (patched.length === 1) appState.toast("success", t("library.patch.done", { name: patched[0] }));
      else if (patched.length > 1) appState.toast("success", t("library.patch.done.many", { count: patched.length }));
      if (failures.length > 0) appState.toast("error", failures.join("\n"));
    } finally {
      try {
        await appState.refreshLibrary();
      } catch (e) {
        appState.toast("error", String(e));
      }
      this.busy = false;
    }
  }
}

export const downloadsState = new DownloadsState();

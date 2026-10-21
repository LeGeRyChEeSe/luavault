import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  DOWNLOADS_FOUND_EVENT,
  DOWNLOADS_STATE_EVENT,
  downloadsWatchStart,
  downloadsWatchStatus,
  openSourceSite,
  downloadsWatchStop,
  downloadsPendingClear,
  importLuaFile,
  importPatchArchive,
  downloadsInventory,
  downloadsSteamNames,
} from "./api";
import type { DownloadFound, DownloadsWatchStatus, SteamName } from "./api";
import { appState } from "./app-state.svelte";
import { BLOCK_KEYS, importOrder, mergeFound, planDownload } from "./downloads";
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
  /**
   * Everything in the watched folders the library does not have yet, old files included:
   * what a file ignored or forgotten falls back to. « Fichiers détectés » opens it.
   */
  inventory = $state<DownloadFound[]>([]);
  /** The list is open on the inventory (the user asked), not on a fresh arrival. */
  browsing = $state(false);
  /** What Steam calls the AppIDs of the listed files (a hint shown beside each). */
  steamNames = $state<Record<string, SteamName>>({});

  private unlisten: UnlistenFn[] = [];
  private refocus = () => void this.refreshInventory();

  /** What the window lists right now. */
  get shown(): DownloadFound[] {
    return this.browsing ? this.inventory : this.pending;
  }

  async refreshInventory() {
    try {
      this.inventory = await downloadsInventory();
    } catch {
      // The count is a convenience: a folder that cannot be read leaves the last list.
      return;
    }
    void this.resolveNames(this.inventory);
  }

  /**
   * Ask Steam what the listed AppIDs are called. The games already in the library are named
   * locally; a name Steam answered (or said it does not know) is not asked twice; an answer
   * that never came is asked again next time. Never blocks the list.
   */
  async resolveNames(items: readonly DownloadFound[]) {
    const inLibrary = new Set(appState.statuses.map((game) => game.app_id));
    const wanted = [
      ...new Set(
        items
          .map((item) => item.app_id)
          .filter((id): id is string => !!id && !inLibrary.has(id) && this.steamNames[id] === undefined),
      ),
    ];
    if (wanted.length === 0) return;
    try {
      const answers = await downloadsSteamNames(wanted);
      const next = { ...this.steamNames };
      for (const answer of answers) if (answer.status !== "unreachable") next[answer.app_id] = answer;
      this.steamNames = next;
    } catch {
      // Offline or throttled: the file is listed without a name, as before.
    }
  }

  openBrowse() {
    this.browsing = true;
    void this.refreshInventory();
  }

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
        void this.resolveNames(event.payload);
        void this.refreshInventory();
      }),
      listen<DownloadsWatchStatus>(DOWNLOADS_STATE_EVENT, (event) => this.apply(event.payload)),
    ]);
    try {
      this.apply(await downloadsWatchStatus());
    } catch {
      this.apply(IDLE);
    }
    void this.refreshInventory();
    // A file may have come or gone while the window was elsewhere.
    window.addEventListener("focus", this.refocus);
  }

  dispose() {
    for (const off of this.unlisten) off();
    this.unlisten = [];
    window.removeEventListener("focus", this.refocus);
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

  /**
   * « Trouver un fichier »: open the download site in the browser and listen to the
   * downloads folder, so the file is offered as soon as it lands. The listening is a
   * convenience: if it cannot start (no downloads folder), the site still opens.
   */
  async findFile() {
    try {
      await openSourceSite();
    } catch (e) {
      appState.toast("error", String(e));
      return;
    }
    if (!this.watch.active) await this.start();
  }

  /**
   * "Ignorer" / "Fermer": put the question away. The files are left exactly where they are,
   * and they stay in the inventory: adding them later is one click on « Fichiers détectés ».
   */
  dismiss() {
    if (this.browsing) {
      this.browsing = false;
      return;
    }
    this.pending = [];
    this.answered();
  }

  /** Nothing waits for an answer: the notification that listed the files has no point left. */
  private answered() {
    void downloadsPendingClear().catch(() => {});
  }

  /**
   * Add the chosen files through the library's own import commands, one at a time
   * (each write signs the shared index). The popup is the confirmation, so the
   * native "are you sure" the manual patch import asks is not asked twice.
   */
  async add(paths: string[]) {
    const chosen = importOrder(this.shown.filter((item) => paths.includes(item.path)));
    this.pending = [];
    this.browsing = false;
    this.answered();
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
            // The game now exists: a patch chosen beside its .lua can follow.
            if (item.app_id) library.set(item.app_id, item.name);
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
      await this.refreshInventory();
      this.busy = false;
    }
  }
}

export const downloadsState = new DownloadsState();

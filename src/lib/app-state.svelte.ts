import { checkUpdate, detectAll, getReachability, getUpdateNotified, libraryStatus, listLibrary, markUpdateNotified, syncFromSteam, takeUpdateResult } from "./api";
import type { DetectionReport, GameStatus, LibraryEntry, Reachability, UpdateAvailable, UpdateResult } from "./api";
import { themeStore } from "./theme.svelte";
import { i18n, t } from "./i18n.svelte";
import { LONG_TOAST_MS, resolveToastDuration } from "./toast-duration";

export interface Toast {
  id: number;
  kind: "success" | "error" | "info" | "warning";
  text: string;
  /** The visibility duration actually chosen for this toast, in ms. */
  duration: number;
}

export interface LogEntry {
  id: number;
  level: string;
  message: string;
  timestamp?: string;
}

/** What the big Steam-style card is showing, if anything. */
export interface GameSpotlight {
  appId: string;
  name: string;
  icon?: string | null;
}

class AppStore {
  report = $state<DetectionReport | null>(null);
  library = $state<LibraryEntry[]>([]);
  /** Non-null when the library index is unreadable — keeps the error visible.
   *  Remains null when the library is healthy. */
  libraryError = $state<string | null>(null);
  /** Per-game install/patch state — the source of truth for every colour code. */
  statuses = $state<GameStatus[]>([]);
  toasts = $state<Toast[]>([]);
  logs = $state<LogEntry[]>([]);
  online = $state(true);
  offlineTip: string | null = null;

  setReachability(r: Reachability) {
    this.online = r.online;
    this.offlineTip = r.tip ?? null;
  }

  async refreshReachability() {
    try {
      const r = await getReachability();
      this.setReachability(r);
    } catch {
      // A failed Tauri command is itself an offline result: keeping the prior
      // value would leave the UI falsely online after a command timeout.
      this.online = false;
      this.offlineTip = null;
    }
  }
  /** Non-null while the Steam-style card is open, over whatever view is behind. */
  spotlight = $state<GameSpotlight | null>(null);
  /** Non-null when a newer version is available (set once at startup). */
  updateAvailable = $state<UpdateAvailable | null>(null);
  private toastId = 0;
  private logId = 0;

  async refresh() {
    try {
      this.report = await detectAll();
      themeStore.hydrate(this.report.theme, this.report.dark_mode);
      i18n.hydrate(this.report.locale);
    } catch (e) {
      themeStore.hydrate(null, null);
      i18n.hydrate(null);
      this.toast("error", t("state.detect.error", { error: String(e) }));
    }
    await this.refreshLibrary();
  }

  /**
   * Adopt `.lua` files already in Steam and refresh online-fix availability.
   * Quiet by design: it runs at startup, and a user who never touched Steam by
   * hand should see nothing at all.
   */
  async adoptFromSteam(announce = true) {
    try {
      const report = await syncFromSteam();
      if (report.imported.length > 0) {
        await this.refreshLibrary();
        if (announce) {
          const names = report.imported.slice(0, 3).join(", ");
          const rest = report.imported.length - 3;
          this.toast(
            "info",
            rest > 0
              ? t("state.adopt.some-more", { n: report.imported.length, names, rest })
              : t("state.adopt.some", { n: report.imported.length, names }),
            LONG_TOAST_MS,
          );
        }
      }
      for (const error of report.errors) this.addLog("warn", t("state.log.import-error", { error }));
      return report;
    } catch (e) {
      this.addLog("warn", t("state.log.import-failed", { error: String(e) }));
      return null;
    }
  }

  /**
   * Consume the update result stored before the last install.
   * Shows a success toast when the update actually changed the version.
   * Called once at startup — never blocks the UI.
   */
  async checkUpdateResult() {
    try {
      const result = await takeUpdateResult();
      if (result) {
        this.toast(
          "success",
          t("state.update.done", { from: result.from, to: result.to }),
          LONG_TOAST_MS,
        );
      }
    } catch {
      // Backend command failed — nothing to show.
    }
  }

  /**
   * Background update check: silent on failure, toast once per version.
   * Called once at startup — never blocks the UI.
   */
  async checkForUpdate() {
    try {
      const update = await checkUpdate();
      if (!update) return;
      this.updateAvailable = update;
      const notified = await getUpdateNotified();
      if (notified !== update.version) {
        this.toast("info", t("state.update.available", { version: update.version }), LONG_TOAST_MS);
        await markUpdateNotified(update.version);
      }
    } catch {
      // Offline or server error — the nominal case, nothing to show.
    }
  }

  async refreshLibrary() {
    try {
      this.library = await listLibrary();
      this.libraryError = null;
    } catch (e) {
      this.libraryError = t("state.library.error", { error: String(e) });
      this.toast("error", this.libraryError);
    }
    await this.refreshStatuses();
  }

  /** Re-read install/patch state without re-reading the whole index. */
  async refreshStatuses() {
    try {
      this.statuses = await libraryStatus();
    } catch {
      // Steam may be missing; the views degrade gracefully.
    }
  }

  statusFor(appId: string): GameStatus | undefined {
    return this.statuses.find((s) => s.app_id === appId);
  }

  /** Games that need the user to do something, for the sidebar counter. */
  get needsAttention(): number {
    return this.statuses.filter((s) =>
      ["lua_not_in_steam", "needs_steam_install", "fix_damaged", "fix_game_moved"].includes(
        s.stage,
      ),
    ).length;
  }

  openSpotlight(spotlight: GameSpotlight) {
    this.spotlight = spotlight;
  }

  closeSpotlight() {
    this.spotlight = null;
  }

  /** Show a toast. `durationMs` is optional: absent or invalid values fall
      back to the default (see toast-duration.ts). The timer removes its own
      toast by id — never another one. */
  toast(kind: Toast["kind"], text: string, durationMs?: number) {
    const id = ++this.toastId;
    const duration = resolveToastDuration(durationMs);
    this.toasts.push({ id, kind, text, duration });
    setTimeout(() => {
      this.toasts = this.toasts.filter((t) => t.id !== id);
    }, duration);
  }

  addLog(level: string, message: string, timestamp?: string) {
    this.logs.push({ id: ++this.logId, level, message, timestamp });
    if (this.logs.length > 500) this.logs = this.logs.slice(-500);
  }
}

export const appState = new AppStore();

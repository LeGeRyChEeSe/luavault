/**
 * The guide in Steam, wired: Rust's events in, the engine's effects out.
 *
 * `engine.ts` decides; this file listens (`steam-guide://call|fetch|job|tick`), carries out the
 * effects (open a Steam page, look a game up, read its status), and turns the current step into
 * the bubble Steam draws (`buildView`: texts from the catalogues, `[[…]]` names cut into segments).
 * It runs in the application's window, shown or hidden: the heartbeat that wakes it comes from
 * Rust (`steam-guide://tick`), since a hidden window's timers sleep.
 */

import { listen } from "@tauri-apps/api/event";
import {
  gameStatus,
  getSteamDetails,
  steamGuideOpen,
  steamGuideReady,
  steamGuideShow,
  type GameStatus,
  type SteamGuideReadiness,
  type SteamGuideView,
} from "../api";
import { i18n, t, type Key } from "../i18n.svelte";
import { splitUiRefs } from "./ui-refs";
import {
  actionKey,
  CHAPTER_TITLE,
  choicesOf,
  gestureOf,
  canContinue,
  infoKey,
  INITIAL,
  progressOf,
  reduce,
  STEPS,
  viewsOf,
  type Effect,
  type Event,
  type Place,
  type State,
  type StatusFacts,
} from "./engine";

function segments(text: string) {
  return splitUiRefs(text).map((s) => (s.ref ? { t: s.text, ui: true } : { t: s.text }));
}

/** Pure but for `t`. The bubble of the current step, in the person's language. */
export function buildView(s: State, run: number): SteamGuideView {
  const step = STEPS[s.index];
  const { where, bubble, target } = viewsOf(step, s);
  const { n, total } = progressOf(s.index);
  const action = actionKey(step, s);
  const byGame = target === "store-job" || target === "store-install" || target === "play-block" || target === "play-manage" || target === "list-dot";
  return {
    run,
    where,
    bubble,
    target,
    appId: byGame ? s.appId : null,
    title: t(CHAPTER_TITLE[step.chapter]),
    progress: t("steamguide.progress", { n, total }),
    info: segments(t(infoKey(step, s))),
    action: action ? segments(t(action)) : [],
    choices: choicesOf(step, s).map((c) => ({ id: c.id, label: t(`steamguide.choice.${c.id}` as Key), primary: c.primary })),
    next: canContinue(step, s) ? t("steamguide.next") : "",
    back: step.back && target !== null ? t("steamguide.back") : "",
    quit: t("steamguide.quit"),
    gesture: target !== null && gestureOf(step),
    veil: target !== null ? step.veil === true : step.veil === true && !step.corner,
    corner: step.corner === true,
  };
}

/** What the guide reads of a game's status. */
export function factsOf(st: GameStatus): StatusFacts {
  const installing = st.stage === "installing" || (st.game.known_to_steam && st.game.installed && !st.game.fully_installed);
  const patchWaiting = st.fix_downloaded && st.fix.health === "not_installed";
  return { appId: st.app_id, stage: st.stage, installing, installed: st.game.fully_installed, patches: patchWaiting, luaInSteam: st.lua_in_steam };
}

type CallPayload = { from: "window" | "web"; call: { kind: string; choice?: string; target?: string } & Partial<Place> };

class SteamGuideRunner {
  running = $state(false);
  private state: State | null = null;
  private run = 0;
  private lastSent = "";
  private wired = false;

  /** Listen to Rust once, from the app shell. */
  init(): () => void {
    if (this.wired) return () => {};
    this.wired = true;
    const offs = [
      listen<CallPayload>("steam-guide://call", (e) => this.onCall(e.payload)),
      listen<{ appIds: string[] }>("steam-guide://fetch", (e) => this.dispatch({ kind: "fetch", appIds: e.payload.appIds })),
      listen<{ appId: string; tone: string; install: boolean; library: boolean; site: boolean }>("steam-guide://job", (e) =>
        this.dispatch({
          kind: "job",
          appId: e.payload.appId,
          tone: e.payload.tone,
          install: e.payload.install,
          library: e.payload.library,
          site: e.payload.site === true,
        }),
      ),
      listen("steam-guide://tick", () => this.dispatch({ kind: "tick" })),
    ];
    return () => {
      for (const off of offs) void off.then((f) => f());
      this.wired = false;
    };
  }

  /** Start in Steam, or say why not (a key of `steamguide.ready.*`). */
  async start(): Promise<Exclude<SteamGuideReadiness["reason"], "ready"> | "error" | null> {
    let ready: SteamGuideReadiness;
    try {
      ready = await steamGuideReady();
    } catch {
      return "error";
    }
    if (ready.reason !== "ready") return ready.reason;
    this.state = { ...INITIAL };
    this.run += 1;
    this.running = true;
    this.lastSent = "";
    await this.draw();
    return null;
  }

  private onCall(p: CallPayload) {
    const c = p.call;
    if (c.kind === "start") {
      if (this.state === null) void this.start();
      else {
        this.dispatch({ kind: "again" });
        void this.draw(true);
      }
      return;
    }
    if (this.state === null) {
      // A page still showing a guide this window no longer runs (the app restarted): take it away.
      if (c.kind === "quit" || c.kind === "next" || c.kind === "choose") void steamGuideShow(null).catch(() => {});
      return;
    }
    switch (c.kind) {
      case "next":
      case "back":
      case "quit":
        return this.dispatch({ kind: c.kind });
      case "choose":
        return this.dispatch({ kind: "choose", choice: String(c.choice) });
      case "hit":
        return this.dispatch({ kind: "hit", target: String(c.target) });
      case "lost":
        return this.dispatch({ kind: "lost", target: String(c.target) });
      case "place": {
        const { kind: _kind, ...place } = c;
        return this.dispatch({ kind: "place", from: p.from, place: place as Place });
      }
    }
  }

  dispatch(ev: Event) {
    const s = this.state;
    if (s === null) return;
    const { state, effects } = reduce(s, ev);
    if (state.index !== s.index || state.notice !== s.notice || state.paused !== s.paused) this.run += 1;
    this.state = state;
    for (const e of effects) this.carry(e);
    if (this.state !== null) void this.draw();
  }

  private carry(e: Effect) {
    switch (e.kind) {
      case "open":
        void steamGuideOpen(e.place).catch(() => {});
        return;
      case "lookup-free":
        getSteamDetails(e.appId, i18n.locale)
          .then((d) => this.dispatch({ kind: "free", appId: e.appId, free: d.is_free === true }))
          // No answer from Steam's store: better to go on than to block on a lookup.
          .catch(() => this.dispatch({ kind: "free", appId: e.appId, free: false }));
        return;
      case "status":
        gameStatus(e.appId)
          .then((st) => this.dispatch({ kind: "status", facts: factsOf(st) }))
          .catch(() => {});
        return;
      case "finish":
        this.stop();
        return;
      case "quit":
        this.stop();
        return;
    }
  }

  /** Draw the current step again (the language changed). */
  redraw() {
    void this.draw(true);
  }

  stop() {
    this.state = null;
    this.running = false;
    this.lastSent = "";
    void steamGuideShow(null).catch(() => {});
  }

  /** Send the step to Steam, unless it is the very one already there (`force`: draw it again). */
  private async draw(force = false) {
    const s = this.state;
    if (s === null) return;
    const view = s.paused ? null : buildView(s, this.run);
    const json = JSON.stringify(view);
    if (!force && json === this.lastSent) return;
    this.lastSent = json;
    try {
      await steamGuideShow(view);
    } catch {
      // The step was refused (it never should be): better to stop than to show a broken guide.
      this.stop();
    }
  }
}

export const steamGuide = new SteamGuideRunner();

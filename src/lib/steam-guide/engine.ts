/**
 * The guide drawn in Steam: the scenario and its engine, pure.
 *
 * The person picks a paid game in Steam's own store and the guide walks them through it: the
 * « Trouver sur le site » button opens the site in their browser, they download the game's files
 * there, LuaVault picks them up from their Downloads and hands them to Steam, and Steam installs
 * the game. Then a few bubbles say the rest. Its gestures are the real buttons' own calls: it
 * listens to what happens, it never does it for them.
 *
 * Everything that happens reaches `reduce` as an `Event`; what the engine wants done comes back
 * as `Effect`s (draw, open a Steam page, look a game up), which `runner.svelte.ts` carries out.
 * No rune, no IPC, no DOM here: `scripts/test-steam-guide.ts` plays whole runs by value.
 *
 * Steam is not ours to snapshot: a step states the place it needs (`need`), and when the person
 * is elsewhere the bubble says so and offers to bring them back (`away`) instead of pointing at
 * nothing.
 */

import type { Key } from "../i18n/fr";

/** The page script's targets (`TARGETS` in `scripts/steam-guide.js` and `steam_guide.rs`). */
export const TARGETS = [
  "pill",
  "tab-store",
  "tab-library",
  "settings",
  "store-search",
  "store-find",
  "store-owned",
  "store-panel",
  "store-job",
  "store-install",
  "play-button",
  "play-block",
  "play-manage",
  "play-menu",
  "list-dot",
] as const;
export type Target = (typeof TARGETS)[number];

/** The view each target lives in. The settings panel is drawn by whichever view is in front. */
export const TARGET_VIEW: Record<Target, "window" | "web" | "any"> = {
  pill: "window",
  "tab-store": "window",
  "tab-library": "window",
  settings: "any",
  "store-search": "web",
  "store-find": "web",
  "store-owned": "web",
  "store-panel": "web",
  "store-job": "web",
  "store-install": "web",
  "play-button": "window",
  "play-block": "window",
  "play-manage": "window",
  "play-menu": "window",
  "list-dot": "window",
};

/** Where a page says the client is (`steam_guide::Place`, as serde writes it). */
export type Place =
  | { place: "store-app"; appId: string; owned: boolean; known: boolean }
  | { place: "store-bundle" }
  | { place: "store" }
  | { place: "web" }
  | { place: "web-shown" }
  | { place: "library-app"; appId: string }
  | { place: "window" };

/** What a step needs of Steam to be shown. */
export type Need = "any" | "store" | "store-page" | "library-game";

export type Gate =
  | "next"
  | "choice"
  /** Any page of the store. */
  | "store"
  /** A game's page that is a good first game (paid, not owned, not already LuaVault's): `candidate`. */
  | "candidate"
  /** A fetch asked from the store (`steam-guide://fetch`). */
  | "fetch"
  /** The store panel's line of the game settled (`steam-guide://job`). */
  | "job"
  /** Steam started installing (or has installed) the game. */
  | "installing"
  /** Steam has the whole game. */
  | "installed"
  /** The library shows the game's page. */
  | "library-game";

export const CHOICES = [
  "start",
  "later",
  "open-store",
  "open-library",
  "skip",
  "pause",
  "resume",
  "return",
  "other",
  "library",
  "retry",
  "finish",
] as const;
export type ChoiceId = (typeof CHOICES)[number];

export type StepId =
  | "first-welcome"
  | "first-pill"
  | "first-store"
  | "first-search"
  | "first-button"
  | "first-check"
  | "first-panel"
  | "first-install"
  | "first-wait"
  | "first-open-library"
  | "first-install-library"
  | "first-ready"
  | "more-dot"
  | "more-manage"
  | "more-settings"
  | "more-end";

export type ChapterId = "first" | "more";

export interface Step {
  id: StepId;
  chapter: ChapterId;
  need: Need;
  target?: Target;
  gate: Gate;
  /** The bubble's choices, the first one highlighted unless `primary` says otherwise. */
  choices?: readonly ChoiceId[];
  primary?: ChoiceId;
  /** « Retour » goes to the step before (informational steps only). */
  back?: boolean;
  /** Dim all but the target. Off where the person must move freely (the store's search). */
  veil?: boolean;
  /** A small bubble in a corner, no veil: the guide waits on something outside it. */
  corner?: boolean;
  /** The target may simply not exist (a switched-off block): lost = skipped. */
  optional?: boolean;
  /** A text variant when the game needs a patch (`patches` in the state). */
  patches?: boolean;
}

export const STEPS: readonly Step[] = [
  { id: "first-welcome", chapter: "first", need: "any", gate: "choice", choices: ["start", "later"], veil: true },
  { id: "first-pill", chapter: "first", need: "any", target: "pill", gate: "next", back: true, veil: true },
  { id: "first-store", chapter: "first", need: "any", target: "tab-store", gate: "store", choices: ["open-store"], veil: true },
  { id: "first-search", chapter: "first", need: "store", target: "store-search", gate: "candidate" },
  { id: "first-button", chapter: "first", need: "store-page", target: "store-find", gate: "fetch", veil: true },
  { id: "first-check", chapter: "first", need: "any", gate: "job", corner: true },
  { id: "first-panel", chapter: "first", need: "store-page", target: "store-job", gate: "next", veil: true, optional: true, patches: true },
  { id: "first-install", chapter: "first", need: "store-page", target: "store-install", gate: "installing", veil: true, optional: true },
  { id: "first-wait", chapter: "first", need: "any", gate: "installed", choices: ["skip", "pause"], primary: "pause", corner: true },
  { id: "first-open-library", chapter: "first", need: "any", target: "tab-library", gate: "library-game", choices: ["open-library"], veil: true },
  // A game LuaVault already had, opened from the store, that Steam has not installed: its own
  // « Installer » is the play bar's button (found from the real client's props, `lvPlayButtons`).
  { id: "first-install-library", chapter: "first", need: "library-game", target: "play-button", gate: "installing", veil: true },
  { id: "first-ready", chapter: "first", need: "library-game", target: "play-block", gate: "next", veil: true, optional: true, patches: true },
  { id: "more-dot", chapter: "more", need: "library-game", target: "list-dot", gate: "next", back: true, veil: true, optional: true },
  { id: "more-manage", chapter: "more", need: "library-game", target: "play-manage", gate: "next", back: true, veil: true, optional: true },
  { id: "more-settings", chapter: "more", need: "any", target: "pill", gate: "next", veil: true },
  { id: "more-end", chapter: "more", need: "any", gate: "choice", choices: ["finish"], veil: true },
];

export const CHAPTER_TITLE: Record<ChapterId, Key> = {
  first: "steamguide.title.first",
  more: "steamguide.title.more",
};

export function indexOf(id: StepId): number {
  return STEPS.findIndex((s) => s.id === id);
}

/** A message that replaces the step's info while it holds (a free game, a failed fetch). */
/** `detect-off`: the downloads detection is switched off, the files are added by hand. */
export type Notice = "free" | "owned" | "known" | "bundle" | "detect-off" | "failed" | "paused";

/** Each notice's own choices; the first is highlighted. None: the step's. */
export const NOTICE_CHOICES: Partial<Record<Notice, readonly ChoiceId[]>> = {
  known: ["library", "other"],
  failed: ["retry"],
  paused: ["resume"],
};

/** The notices after which the next gesture is a new search: the bubble keeps pointing at it. */
export const SEARCH_AGAIN: readonly Notice[] = ["free", "owned", "bundle"];

export interface State {
  index: number;
  /** The game fetched (a DLC's page fetches its base game: the fetch says which). */
  appId: string | null;
  /** The store page the person opened. */
  pageId: string | null;
  notice: Notice | null;
  /** The game needs a patch that will be applied once Steam has it. */
  patches: boolean;
  /** The fetch left the game for Steam to install (the panel offered « Installer »). */
  installable: boolean;
  /** « Reprendre plus tard »: nothing drawn until the game is installed. */
  paused: boolean;
  /** Steam has the whole game, as its status last said (`null`: not read yet). */
  gameInstalled: boolean | null;
  /** The install step the wait came from, and how many readings in a row saw Steam do nothing:
   *  Steam's own install window was cancelled (it asks a confirmation), the guide goes back. */
  installFrom: StepId | null;
  idle: number;
  windowPlace: Place | null;
  webPlace: Place | null;
}

/** Readings of the status (one per heartbeat, 3 s) with Steam neither installing nor done before a
 *  wait is taken for a cancelled install. Steam's confirmation window takes a few seconds to answer. */
export const IDLE_READINGS = 5;

export const INITIAL: State = {
  index: 0,
  appId: null,
  pageId: null,
  notice: null,
  patches: false,
  installable: true,
  paused: false,
  gameInstalled: null,
  installFrom: null,
  idle: 0,
  windowPlace: null,
  webPlace: null,
};

/** What a game's status says, reduced to what the guide reads (`GameStatus`). */
export interface StatusFacts {
  appId: string;
  stage: string;
  /** Steam knows it and started it (or is downloading). */
  installing: boolean;
  /** Steam has the whole game. */
  installed: boolean;
  /** A patch was downloaded and waits for the game (`fix_downloaded`, not applied). */
  patches: boolean;
  /** The game's `.lua` is where Steam reads it (imported from the Downloads, or added by hand). */
  luaInSteam: boolean;
}

export type Event =
  | { kind: "next" }
  | { kind: "back" }
  | { kind: "quit" }
  | { kind: "choose"; choice: string }
  | { kind: "hit"; target: string }
  | { kind: "lost"; target: string }
  | { kind: "place"; from: "window" | "web"; place: Place }
  | { kind: "fetch"; appIds: string[] }
  /** A line of the store panel. `site`: it still offers « Ouvrir le site » (the person is on the site). */
  | { kind: "job"; appId: string; tone: string; install: boolean; library: boolean; site: boolean }
  | { kind: "status"; facts: StatusFacts }
  | { kind: "free"; appId: string; free: boolean }
  | { kind: "tick" }
  /** « Visite guidée » again while a guide runs: shown again, out of a pause included. */
  | { kind: "again" };

export type Effect =
  | { kind: "open"; place: string }
  | { kind: "lookup-free"; appId: string }
  | { kind: "status"; appId: string }
  | { kind: "finish" }
  | { kind: "quit" };

export interface Outcome {
  state: State;
  effects: Effect[];
}

/** The place that counts: the web view's when the window shows it in front, else the window's. */
export function currentPlace(s: State): Place | null {
  if (s.windowPlace?.place === "web-shown") return s.webPlace ?? { place: "web" };
  return s.windowPlace ?? s.webPlace;
}

/** Whether the client is on one of Valve's web pages (the web view in front). */
export function onWeb(p: Place | null): boolean {
  return p !== null && ["store-app", "store-bundle", "store", "web", "web-shown"].includes(p.place);
}

function isStore(p: Place | null): boolean {
  return p !== null && ["store-app", "store-bundle", "store"].includes(p.place);
}

/** Pure. Whether the step's place holds. Nothing heard yet is no reason to nag. */
export function needMet(step: Step, s: State): boolean {
  const p = currentPlace(s);
  if (step.need === "any" || p === null) return true;
  if (step.need === "store") return isStore(p);
  if (step.need === "store-page") return p.place === "store-app" && p.appId === s.pageId;
  return p.place === "library-app" && p.appId === s.appId;
}

/** Where « Y retourner » leads for this step (`steam_guide::place_uri`'s set). */
export function returnPlace(step: Step, s: State): string {
  if (step.need === "store-page" && s.pageId) return `store-app:${s.pageId}`;
  if (step.need === "library-game" && s.appId) return `library-app:${s.appId}`;
  return step.need === "library-game" ? "library" : "store";
}

/** Pure. The choices the bubble shows now: away, a notice's, or the step's. */
export function choicesOf(step: Step, s: State): { id: ChoiceId; primary: boolean }[] {
  if (!needMet(step, s) && s.notice === null) return [{ id: "return", primary: true }];
  const list = (s.notice !== null ? NOTICE_CHOICES[s.notice] : undefined) ?? step.choices ?? [];
  const primary = s.notice === null && step.primary ? step.primary : list[0];
  return list.map((id) => ({ id, primary: id === primary }));
}

function goTo(s: State, id: StepId): State {
  return { ...s, index: indexOf(id), notice: null };
}

function advance(s: State): State {
  return { ...s, index: Math.min(s.index + 1, STEPS.length - 1), notice: null };
}

/** The step after the panel: Steam already has the game, or it is Steam's to install. */
function afterPanel(s: State): State {
  return goTo(s, s.installable ? "first-install" : "first-open-library");
}

/** A game's page was opened while the guide waits for one: is it a good first game? */
function candidate(s: State, p: Place): Outcome {
  if (p.place === "store-bundle") return { state: { ...goTo(s, "first-search"), notice: "bundle" }, effects: [] };
  if (p.place !== "store-app") return { state: goTo(s, "first-search"), effects: [] };
  const at = { ...goTo(s, "first-search"), pageId: p.appId };
  if (p.known) return { state: { ...at, notice: "known" }, effects: [] };
  if (p.owned) return { state: { ...at, notice: "owned" }, effects: [] };
  // Free or not is Steam's own data: asked once, and the answer moves the guide on (`free`).
  return { state: at, effects: [{ kind: "lookup-free", appId: p.appId }] };
}

const same = (s: State): Outcome => ({ state: s, effects: [] });

/** Pure. One event, the state after it and what to do. A step just entered is judged at once
 *  against where the person already is (the game's page already open, the library already on
 *  it): pages report a place only when it changes. */
export function reduce(s: State, ev: Event): Outcome {
  let out = reduceOnce(s, ev);
  let before = s.index;
  // A step entered may itself lead on (already there, already installed): a few hops at most.
  for (let hop = 0; hop < 4 && out.state.index !== before; hop++) {
    before = out.state.index;
    const at = entered(out.state);
    const again = placed(at.state, STEPS[at.state.index]);
    out = { state: again.state, effects: [...out.effects, ...at.effects, ...again.effects] };
  }
  return out;
}

/** What entering a step decides by itself, from what the game's status already said. */
function entered(s: State): Outcome {
  const id = STEPS[s.index].id;
  if (id === "first-install-library" && s.gameInstalled === true) return same(goTo(s, "first-ready"));
  if (id === "first-ready" && s.gameInstalled === false) return same(goTo(s, "first-install-library"));
  // Not read yet: asked now, and the answer moves the guide (`status`).
  if ((id === "first-install-library" || id === "first-ready") && s.gameInstalled === null && s.appId) {
    return { state: s, effects: [{ kind: "status", appId: s.appId }] };
  }
  return same(s);
}

function reduceOnce(s: State, ev: Event): Outcome {
  const step = STEPS[s.index];
  switch (ev.kind) {
    case "quit":
      return { state: s, effects: [{ kind: "quit" }] };
    case "next":
      if (!canContinue(step, s)) return same(s);
      return same(step.id === "first-panel" ? afterPanel(s) : advance(s));
    case "back":
      return step.back && s.index > 0 ? same({ ...s, index: s.index - 1, notice: null }) : same(s);
    case "again":
      return same(s.paused ? { ...s, paused: false } : s);
    case "tick": {
      // At the site too: the files may be added by hand (detection off), only the status says so.
      const waiting = step.gate === "installing" || step.gate === "installed" || s.paused || step.id === "first-check";
      return { state: s, effects: waiting && s.appId ? [{ kind: "status", appId: s.appId }] : [] };
    }
    case "choose":
      return choose(s, step, ev.choice);
    case "hit":
      if (step.id === "first-install" && ev.target === "store-install") return same({ ...goTo(s, "first-wait"), installFrom: step.id, idle: 0 });
      if (step.id === "first-install-library" && ev.target === "play-button") return same({ ...goTo(s, "first-wait"), installFrom: step.id, idle: 0 });
      return same(s);
    case "lost":
      return step.optional && ev.target === step.target ? same(step.id === "first-panel" ? afterPanel(s) : advance(s)) : same(s);
    case "place": {
      const next = ev.from === "window" ? { ...s, windowPlace: ev.place } : { ...s, webPlace: ev.place };
      return placed(next, STEPS[next.index]);
    }
    case "free":
      if (step.id !== "first-search" || ev.appId !== s.pageId || s.notice !== null) return same(s);
      return ev.free ? same({ ...s, notice: "free" }) : same(goTo(s, "first-button"));
    case "fetch": {
      if (step.id !== "first-search" && step.id !== "first-button") return same(s);
      const appId = ev.appIds[0];
      if (!appId) return same(s);
      return same({ ...goTo(s, "first-check"), appId, pageId: s.pageId ?? appId });
    }
    case "job": {
      if (step.id !== "first-check" || ev.appId !== s.appId) return same(s);
      // The site did not open.
      if (ev.tone === "error") return same({ ...goTo(s, "first-button"), notice: "failed" });
      // Still at the site (the panel offers « Ouvrir le site »); detection switched off says so.
      if (ev.site) return same(ev.tone === "warn" ? { ...s, notice: "detect-off" } : s);
      if (ev.tone === "busy") return same(s);
      const installable = ev.install || !ev.library;
      return { state: { ...goTo(s, "first-panel"), installable }, effects: [{ kind: "status", appId: ev.appId }] };
    }
    case "status": {
      const f = ev.facts;
      if (f.appId !== s.appId) return same(s);
      const known = { ...s, patches: f.patches, gameInstalled: f.installed };
      // The files reached Steam another way than the panel says (added by hand): on to the panel.
      if (step.id === "first-check" && f.luaInSteam) {
        return { state: { ...goTo(known, "first-panel"), installable: !f.installed }, effects: [] };
      }
      if (s.paused) {
        return f.installed ? same({ ...goTo(known, "first-open-library"), paused: false, notice: "paused" }) : same(known);
      }
      if (step.id === "first-install" && f.installed) return same(goTo(known, "first-open-library"));
      if (step.id === "first-install" && f.installing) return same({ ...goTo(known, "first-wait"), installFrom: null, idle: 0 });
      if (step.id === "first-wait" && f.installed) return same(goTo(known, "first-open-library"));
      if (step.id === "first-wait" && f.installing) return same({ ...known, idle: 0 });
      if (step.id === "first-wait" && s.installFrom !== null) {
        const idle = s.idle + 1;
        return same(idle >= IDLE_READINGS ? { ...goTo(known, s.installFrom), installFrom: null, idle: 0 } : { ...known, idle });
      }
      if (step.id === "first-install-library" && f.installed) return same(goTo(known, "first-ready"));
      if ((step.id === "first-install-library" || step.id === "first-ready") && f.installing) return same(goTo(known, "first-wait"));
      if (step.id === "first-ready" && !f.installed) return same(goTo(known, "first-install-library"));
      return same(known);
    }
  }
}

/** The automatic gates a new place may open. */
function placed(s: State, step: Step): Outcome {
  const p = currentPlace(s);
  if (p === null) return same(s);
  if (step.gate === "store" && isStore(p)) {
    // Already on a game's page: judged at once, rather than asking to search for it again.
    return p.place === "store-app" || p.place === "store-bundle" ? candidate(advance(s), p) : same(advance(s));
  }
  if (step.gate === "candidate" && (p.place === "store-app" || p.place === "store-bundle")) {
    if (p.place === "store-app" && p.appId === s.pageId && s.notice !== "bundle") return same(s);
    return candidate(s, p);
  }
  if (step.gate === "candidate" && p.place === "store" && s.notice !== null) {
    // Back on the store's home or search: the warning about the last page goes.
    return same({ ...s, notice: null, pageId: null });
  }
  if (step.id === "first-button" && p.place === "store-app" && p.appId !== s.pageId) return candidate(s, p);
  if (step.id === "first-button" && (p.place === "store" || p.place === "store-bundle")) return candidate(s, p);
  if (step.gate === "library-game" && p.place === "library-app" && p.appId === s.appId) {
    return same(goTo(s, s.gameInstalled === false ? "first-install-library" : "first-ready"));
  }
  return same(s);
}

function choose(s: State, step: Step, choice: string): Outcome {
  const offered = choicesOf(step, s).some((c) => c.id === choice);
  if (!offered) return same(s);
  switch (choice as ChoiceId) {
    case "start":
      return same(advance(s));
    case "later":
      return { state: s, effects: [{ kind: "quit" }] };
    case "open-store":
      return { state: s, effects: [{ kind: "open", place: "store" }] };
    case "open-library":
      return { state: s, effects: [{ kind: "open", place: s.appId ? `library-app:${s.appId}` : "library" }] };
    case "return":
      return { state: s, effects: [{ kind: "open", place: returnPlace(step, s) }] };
    case "skip":
      return same(goTo(s, "more-settings"));
    case "pause":
      return same({ ...s, paused: true, notice: null });
    case "resume":
      return same({ ...s, notice: null });
    case "other":
      // Something visible happens: the store's front page, where another game is picked.
      return { state: { ...s, notice: null, pageId: null }, effects: [{ kind: "open", place: "store" }] };
    case "library": {
      const appId = s.pageId;
      // Whether Steam has it decides what the library step asks (play, or install first).
      return {
        state: { ...goTo(s, "first-open-library"), appId, gameInstalled: null },
        effects: appId ? [{ kind: "open", place: `library-app:${appId}` }, { kind: "status", appId }] : [],
      };
    }
    case "retry":
      return same({ ...s, notice: null });
    case "finish":
      return { state: s, effects: [{ kind: "finish" }] };
  }
  return same(s);
}

// ── the bubble's keys, as data ─────────────────────────────────────────

/** Pure. Whether « Continuer » is offered (and heard): the step waits on it, the place holds, and
 *  the block's colour is only described once the game's status said where it stands. */
export function canContinue(step: Step, s: State): boolean {
  if (step.gate !== "next" || s.notice !== null || !needMet(step, s)) return false;
  return step.id !== "first-ready" || s.gameInstalled === true;
}

/** Pure. The info's key now: away, a notice, the patch variant, or the step's own. */
export function infoKey(step: Step, s: State): Key {
  if (s.notice !== null) return noticeKey(s.notice);
  if (!needMet(step, s)) return "steamguide.away.info";
  if (step.id === "first-ready" && s.gameInstalled !== true) return "steamguide.step.first-ready.checking";
  if (step.patches && s.patches) return `steamguide.step.${step.id}.patches` as Key;
  return `steamguide.step.${step.id}.info` as Key;
}

function noticeKey(n: Notice): Key {
  return n === "paused" ? "steamguide.paused.info" : (`steamguide.notice.${n}` as Key);
}

/** Pure. The action's key, or none (away, a notice, or a step without a gesture). */
export function actionKey(step: Step, s: State): Key | null {
  if (s.notice !== null && SEARCH_AGAIN.includes(s.notice) && step.id === "first-search") return "steamguide.step.first-search.action";
  if (s.notice !== null || !needMet(step, s)) return null;
  return ACTIONS.includes(step.id) ? (`steamguide.step.${step.id}.action` as Key) : null;
}

/** The steps whose bubble carries an action line (a gesture is expected). */
export const ACTIONS: readonly StepId[] = [
  "first-store",
  "first-search",
  "first-button",
  "first-check",
  "first-install",
  "first-open-library",
  "first-install-library",
];

/** The steps with a patch variant of their info. */
export const PATCH_VARIANTS: readonly StepId[] = STEPS.filter((s) => s.patches).map((s) => s.id);

/** Pure. Where the target and the bubble are drawn. */
export function viewsOf(step: Step, s: State): { where: "window" | "web" | "any"; bubble: "window" | "web"; target: Target | null } {
  const away = !needMet(step, s) && s.notice === null;
  const kept = s.notice !== null && SEARCH_AGAIN.includes(s.notice) && step.id === "first-search";
  const target = away || (s.notice !== null && !kept) || s.paused ? null : (step.target ?? null);
  const front = onWeb(currentPlace(s)) ? "web" : "window";
  if (target === null) return { where: front, bubble: front, target: null };
  const where = TARGET_VIEW[target];
  const bubble = where === "web" ? "web" : front;
  return { where, bubble, target };
}

/** Pure. Position in the chapter, 1-based, for « Étape n sur total ». */
export function progressOf(index: number): { n: number; total: number } {
  const chapter = STEPS[index].chapter;
  const steps = STEPS.filter((st) => st.chapter === chapter);
  return { n: steps.findIndex((st) => st.id === STEPS[index].id) + 1, total: steps.length };
}

/** Pure. Whether the click on the target is what the step waits for (reported by the page). */
export function gestureOf(step: Step): boolean {
  return step.gate === "fetch" || step.gate === "installing";
}

/**
 * Behavioural tests for the focusTrap helper (LOT-19, corrections 1 & 2).
 *
 * Runs the REAL focusTrap action against a minimal fake DOM — no jsdom,
 * no new dependency. The DOM is a plain object graph that mimics the
 * properties and methods the helper actually calls (addEventListener,
 * querySelector, contains, focus, tabIndex, document.activeElement).
 *
 * Proves the helper contract, not WebView2 behaviour — a browser bench
 * belongs to LOT-23.
 */

// @ts-expect-error — `node:fs` has no types here.
import { readFileSync as readFileSyncRaw } from "node:fs";
import { stripComments } from "./test-dlc-wiring";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

// ── M12 — meta-guard: proves the runner imports test-keyboard-shortcuts.ts ──
// This guard lives here (imported before test-keyboard-shortcuts in the runner)
// so it still runs when that import is commented out.  It strips comments
// from the runner and fails if the import line disappears.
{
  const runnerSource = readFileSync("scripts/test-virtual-scroll.ts", { encoding: "utf8" });
  const runnerNoComments = stripComments(runnerSource);
  const importPresent = /await\s+import\s*\(\s*["']\.\/test-keyboard-shortcuts["']\s*\)/.test(runnerNoComments);
  if (!importPresent) throw new Error("M12: runner does not import test-keyboard-shortcuts.ts");
}

// ── Fake element ──────────────────────────────────────────────
/** Minimal element that implements the subset of HTMLElement the helper uses. */
class FakeEl {
  tagName = "DIV";
  children: FakeEl[] = [];
  _inDoc = true;
  _querySelectorResult: FakeEl | null = null;
  _tabIndex = 0;
  _overrideFocusable: boolean | null = null;

  // Shared document reference — set by FakeDoc constructor
  static _doc: FakeDoc | null = null;

  // Minimal Element properties required by svelte-check.
  attributes: Map<string, string> = new Map();
  classList: DOMTokenList = {} as DOMTokenList;
  className = "";
  clientHeight = 0;
  clientWidth = 0;
  offsetHeight = 0;
  offsetWidth = 0;
  clientLeft = 0;
  clientTop = 0;
  id = "";
  innerHTML = "";

  getAttribute(attr: string): string | null {
    return this.attributes.get(attr) ?? null;
  }
  setAttribute() { /* no-op */ }
  removeAttribute() { /* no-op */ }
  hasAttribute(attr: string): boolean {
    return this.attributes.has(attr);
  }
  getBoundingClientRect() { return { width: 0, height: 0, top: 0, left: 0, bottom: 0, right: 0 } as DOMRect; }

  querySelectorAll<_K extends keyof HTMLElementTagNameMap>(sel: string): NodeListOf<HTMLElement> {
    // Respect the selector: for "button:not([disabled])" etc., check _overrideFocusable
    // When _overrideFocusable is false, also set "disabled" attribute so :not([disabled]) matches
    const result = Array.from(this.children).filter((c) => {
      if (c._overrideFocusable === false) {
        if (!c.hasAttribute("disabled")) c.attributes.set("disabled", "");
        return false;
      }
      if (c._overrideFocusable === true) {
        c.attributes.delete("disabled");
        return true;
      }
      return c._focusable;
    }) as unknown as NodeListOf<HTMLElement>;
    return result;
  }

  addEventListener(_name: string, _handler: unknown): void { /* no-op — helper installs on document */ }
  removeEventListener(_name: string, _handler: unknown): void { /* no-op */ }

  querySelector<_K extends keyof HTMLElementTagNameMap>(sel: string): FakeEl | null {
    if (this._querySelectorResult) return this._querySelectorResult;
    if (sel === "[data-autofocus]") {
      return this.children.find((c) => c.hasAttribute("data-autofocus")) ?? null;
    }
    return this.children.find((c) => c._focusable) ?? null;
  }

  contains(child: unknown): boolean {
    if (child === this) return true;
    if (child instanceof FakeEl) return this.children.includes(child as FakeEl);
    return false;
  }

  focus(): void {
    // Update the shared document's activeElement
    if (FakeEl._doc) {
      FakeEl._doc.activeElement = this;
    }
  }

  get tabIndex(): number { return this._tabIndex; }
  set tabIndex(v: number) { this._tabIndex = v; }

  get _focusable(): boolean { return this._overrideFocusable ?? true; }
}

// ── Fake document ─────────────────────────────────────────────
class FakeDoc {
  activeElement: FakeEl | null = null;
  body: FakeEl;
  _listeners = new Map<string, Set<(e: KeyboardEvent) => void>>();

  constructor() {
    this.body = new FakeEl();
    this.body._inDoc = true;
    // Keep a reference in FakeEl for focus() to update activeElement
    (FakeEl as unknown as { _doc: FakeDoc })._doc = this;
  }

  addEventListener(name: string, cb: (e: KeyboardEvent) => void): void {
    if (!this._listeners.has(name)) this._listeners.set(name, new Set());
    this._listeners.get(name)!.add(cb);
  }
  removeEventListener(name: string, cb: (e: KeyboardEvent) => void): void {
    this._listeners.get(name)?.delete(cb);
    if (this._listeners.get(name)?.size === 0) this._listeners.delete(name);
  }
  _fire(name: string, event?: KeyboardEvent): void {
    this._listeners.get(name)?.forEach((cb) => cb(event as KeyboardEvent));
  }
  contains(node: unknown): boolean {
    if (node === this.body) return true;
    if (node instanceof FakeEl) return (node as FakeEl)._inDoc;
    return false;
  }
}

// ── Helpers ───────────────────────────────────────────────────
function assertOk(cond: boolean, msg?: string): void {
  if (!cond) throw new Error(msg ?? "assertion failed");
}

function label(name: string): void {
  console.log(`  ✓ ${name}`);
}

// ── Single fake document for the entire campaign ──────────────
const origDoc = globalThis.document;
const origHTMLElement = globalThis.HTMLElement;
const fakeDoc = new FakeDoc();

function installFake(): void {
  (globalThis as unknown as { document: unknown }).document = fakeDoc as unknown as Document;
  (globalThis as unknown as { HTMLElement: unknown }).HTMLElement = FakeEl;
}

function uninstallFake(): void {
  (globalThis as unknown as { document: unknown }).document = origDoc;
  (globalThis as unknown as { HTMLElement: unknown }).HTMLElement = origHTMLElement;
}

// ── Active set and helpers ────────────────────────────────────
type FocusTrapFn = typeof import("../src/lib/focus-trap").focusTrap;
type ApiType = ReturnType<FocusTrapFn>;
let focusTrapFn: FocusTrapFn;
const activeApis = new Set<ApiType>();

function mountFocusTrap(node: HTMLElement, options?: import("../src/lib/focus-trap").FocusTrapOptions) {
  const api = focusTrapFn(node, options ?? {});
  activeApis.add(api);
  return api;
}

function destroyFocusTrap(api: ApiType) {
  api.destroy();
  activeApis.delete(api);
}

// ── Scenario builder ──────────────────────────────────────────
function makeScenario() {
  const container = new FakeEl();
  const input = new FakeEl();
  input.attributes.set("data-autofocus", "");
  (input as { _overrideFocusable: boolean | null })._overrideFocusable = true;
  const btn = new FakeEl();
  (btn as { _overrideFocusable: boolean | null })._overrideFocusable = true;
  (container as FakeEl).children = [input, btn];

  const trigger = new FakeEl();
  trigger._inDoc = true;
  (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = trigger as unknown as FakeEl | null;

  return { container, input, btn, trigger };
}

// ── Entire campaign wrapped in try/finally ────────────────────
try {
  installFake();
  focusTrapFn = (await import("../src/lib/focus-trap")).focusTrap;

  // ── Scenario 1: ouverture → [data-autofocus] ──────────────
  {
    const { container, input } = makeScenario();
    const api = mountFocusTrap(container as unknown as HTMLElement);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === input,
      "le focus va au [data-autofocus]",
    );
    destroyFocusTrap(api);
    label("ouverture → [data-autofocus]");
  }

  // ── Scenario 2: dernier + Tab → premier ───────────────────
  {
    const { container, input, btn } = makeScenario();
    const api = mountFocusTrap(container as unknown as HTMLElement);
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = btn as unknown as FakeEl | null;
    const rawHandlers = fakeDoc._listeners.get("keydown") ?? new Set();
    let handler: ((e: KeyboardEvent) => void) | null = null;
    for (const h of rawHandlers) { handler = h; break; }
    let prevented = false;
    const fakeEvent = { key: "Tab" } as KeyboardEvent;
    (fakeEvent as { preventDefault?: () => void }).preventDefault = () => { prevented = true; };
    if (handler) handler(fakeEvent as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === input,
      "Tab depuis le dernier → premier",
    );
    assertOk(prevented, "Tab depuis le dernier → preventDefault appelé");
    destroyFocusTrap(api);
    label("dernier + Tab → premier");
  }

  // ── Scenario 3: premier + Maj+Tab → dernier ───────────────
  {
    const { container, input, btn } = makeScenario();
    const api = mountFocusTrap(container as unknown as HTMLElement);
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = input as unknown as FakeEl | null;
    const rawHandlers = fakeDoc._listeners.get("keydown") ?? new Set();
    let handler: ((e: KeyboardEvent) => void) | null = null;
    for (const h of rawHandlers) { handler = h; break; }
    let prevented = false;
    const fakeEvent = { key: "Tab", shiftKey: true } as KeyboardEvent;
    (fakeEvent as { preventDefault?: () => void }).preventDefault = () => { prevented = true; };
    if (handler) handler(fakeEvent as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === btn,
      "Maj+Tab depuis le premier → dernier",
    );
    assertOk(prevented, "Maj+Tab depuis le premier → preventDefault appelé");
    destroyFocusTrap(api);
    label("premier + Maj+Tab → dernier");
  }

  // ── Scenario 4: focus sur body après disparition → retour dans la modal ──
  {
    assertOk(activeApis.size === 0, "avant S4, tous les pièges sont détruits");
    const { container, input } = makeScenario();
    const api = mountFocusTrap(container as unknown as HTMLElement);
    const body = new FakeEl();
    body._inDoc = true;
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = body as unknown as FakeEl | null;
    const rawHandlers = fakeDoc._listeners.get("keydown") ?? new Set();
    let handler: ((e: KeyboardEvent) => void) | null = null;
    for (const h of rawHandlers) { handler = h; break; }
    let prevented = false;
    const fakeEvent = { key: "Tab" } as KeyboardEvent;
    (fakeEvent as { preventDefault?: () => void }).preventDefault = () => { prevented = true; };
    if (handler) handler(fakeEvent as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === input,
      "Tab depuis body → retour dans la modal",
    );
    assertOk(prevented, "Tab depuis body → preventDefault appelé");
    destroyFocusTrap(api);
    label("focus sur body après disparition → retour dans la modal");
  }

  // ── Scenario 5: liste recalculée après disabled ────────────
  {
    const { container, input, btn, trigger } = makeScenario();
    const api = mountFocusTrap(container as unknown as HTMLElement);
    (btn as { _overrideFocusable: boolean | null })._overrideFocusable = false;
    // Simulate browser after removal/disabled: activeElement on body, not on disabled btn
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = fakeDoc.body as unknown as FakeEl | null;
    const rawHandlers = fakeDoc._listeners.get("keydown") ?? new Set();
    let handler: ((e: KeyboardEvent) => void) | null = null;
    for (const h of rawHandlers) { handler = h; break; }
    let prevented = false;
    const fakeEvent = { key: "Tab" } as KeyboardEvent;
    (fakeEvent as { preventDefault?: () => void }).preventDefault = () => { prevented = true; };
    if (handler) handler(fakeEvent as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === input,
      "btn disabled → Tab saute au premier focusable",
    );
    assertOk(prevented, "Tab depuis disabled → preventDefault appelé");
    // Restore activeElement for next tests
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = trigger as unknown as FakeEl | null;
    destroyFocusTrap(api);
    label("liste recalculée après disabled");
  }

  // ── Scenario 6: zéro contrôle → conteneur ─────────────────
  {
    const empty = new FakeEl();
    empty._inDoc = true;
    const trigger2 = new FakeEl();
    trigger2._inDoc = true;
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = trigger2 as unknown as FakeEl | null;

    const api = mountFocusTrap(empty as unknown as HTMLElement);
    const rawHandlers2 = fakeDoc._listeners.get("keydown") ?? new Set();
    let handler2: ((e: KeyboardEvent) => void) | null = null;
    for (const h of rawHandlers2) { handler2 = h; break; }
    let prevented = false;
    const fakeEvent2 = { key: "Tab" } as KeyboardEvent;
    (fakeEvent2 as { preventDefault?: () => void }).preventDefault = () => { prevented = true; };
    if (handler2) handler2(fakeEvent2 as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === empty,
      "zéro contrôle → le conteneur lui-même reçoit le focus",
    );
    assertOk(prevented, "zéro contrôle → preventDefault appelé");
    destroyFocusTrap(api);
    label("zéro contrôle → conteneur");
  }

  // ── Scenario 7: visionneuse imbriquée ─────────────────────
  {
    const { container: outer, input: outerInput } = makeScenario();
    const api1 = mountFocusTrap(outer as unknown as HTMLElement);

    const inner = new FakeEl();
    inner.attributes.set("data-autofocus", "");
    (inner as { _overrideFocusable: boolean | null })._overrideFocusable = true;
    inner._inDoc = true;
    const innerBtn = new FakeEl();
    (innerBtn as { _overrideFocusable: boolean | null })._overrideFocusable = true;
    (inner as FakeEl).children = [innerBtn];

    const innerTrigger = new FakeEl();
    innerTrigger._inDoc = true;
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = innerTrigger as unknown as FakeEl | null;

    const api2 = mountFocusTrap(inner as unknown as HTMLElement);

    const rawInnerHandlers = fakeDoc._listeners.get("keydown") ?? new Set();
    let innerHandler: ((e: KeyboardEvent) => void) | null = null;
    for (const h of rawInnerHandlers) { innerHandler = h; break; }
    let innerPrevented = false;
    const innerFakeEvent = { key: "Tab" } as KeyboardEvent;
    (innerFakeEvent as { preventDefault?: () => void }).preventDefault = () => { innerPrevented = true; };
    if (innerHandler) innerHandler(innerFakeEvent as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === innerBtn,
      "le piège intérieur gagne le Tab",
    );
    assertOk(innerPrevented, "Tab dans piège intérieur → preventDefault appelé");

    destroyFocusTrap(api2);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === innerTrigger,
      "fermeture du piège intérieur → restauration au déclencheur",
    );

    destroyFocusTrap(api1);
    label("visionneuse imbriquée : le piège intérieur gagne, puis rend la main");
  }

  // ── Scenario 8: destruction → restauration du focus ───────
  {
    const { container, input: trigger } = makeScenario();
    // Set activeElement to trigger before mounting so helper captures it as previous
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = trigger as unknown as FakeEl | null;
    const api = mountFocusTrap(container as unknown as HTMLElement);
    destroyFocusTrap(api);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === trigger,
      "destruction → le focus revient au déclencheur initial",
    );
    label("destruction → restauration du focus");
  }

  // ── Scenario 9: correction 2 — cible explicite valide ─────
  {
    const explicitTarget = new FakeEl();
    explicitTarget._inDoc = true;
    const { container } = makeScenario();
    const removedTrigger = new FakeEl();
    removedTrigger._inDoc = false;
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = removedTrigger as unknown as FakeEl | null;

    const api = mountFocusTrap(container as unknown as HTMLElement, { returnFocus: explicitTarget as unknown as HTMLElement });
    destroyFocusTrap(api);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === explicitTarget,
      "returnFocus explicite valide → restauration dessus même si activeElement est parti",
    );
    label("cible explicite valide avec transitoire retiré du document");
  }

  // ── Scenario 10: correction 2 — cible explicite invalide → fallback ──
  {
    const badTarget = new FakeEl();
    badTarget._inDoc = false;
    const { container, input: trigger } = makeScenario();
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = trigger as unknown as FakeEl | null;

    const api = mountFocusTrap(container as unknown as HTMLElement, { returnFocus: badTarget as unknown as HTMLElement });
    destroyFocusTrap(api);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === trigger,
      "returnFocus invalide → fallback sur activeElement valide",
    );
    label("cible explicite invalide → fallback sur activeElement");
  }

  // ── Scenario 11: deux cycles Tab/Maj+Tab ──────────────────
  {
    const { container, input, btn } = makeScenario();
    const api = mountFocusTrap(container as unknown as HTMLElement);

    // Cycle 1: Tab depuis dernier → premier
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = btn as unknown as FakeEl | null;
    const rawHandlers = fakeDoc._listeners.get("keydown") ?? new Set();
    let handler: ((e: KeyboardEvent) => void) | null = null;
    for (const h of rawHandlers) { handler = h; break; }
    let c1Prevented = false;
    const c1FakeEvent = { key: "Tab" } as KeyboardEvent;
    (c1FakeEvent as { preventDefault?: () => void }).preventDefault = () => { c1Prevented = true; };
    if (handler) handler(c1FakeEvent as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === input,
      "cycle 1: Tab dernier → premier",
    );

    // Cycle 2: Maj+Tab depuis premier → dernier
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = input as unknown as FakeEl | null;
    let c2Prevented = false;
    const c2FakeEvent = { key: "Tab", shiftKey: true } as KeyboardEvent;
    (c2FakeEvent as { preventDefault?: () => void }).preventDefault = () => { c2Prevented = true; };
    if (handler) handler(c2FakeEvent as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === btn,
      "cycle 2: Maj+Tab premier → dernier",
    );

    destroyFocusTrap(api);
    label("deux cycles Tab/Maj+Tab");
  }

  // ── Scenario 12: piège extérieur/intérieur puis retour ────
  {
    const outer = new FakeEl();
    (outer as { _overrideFocusable: boolean | null })._overrideFocusable = true;
    outer._inDoc = true;
    const outerBtn = new FakeEl();
    (outerBtn as { _overrideFocusable: boolean | null })._overrideFocusable = true;
    (outer as FakeEl).children = [outerBtn];

    const outerTrigger = new FakeEl();
    outerTrigger._inDoc = true;
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = outerTrigger as unknown as FakeEl | null;

    const api1 = mountFocusTrap(outer as unknown as HTMLElement);

    const inner = new FakeEl();
    inner.attributes.set("data-autofocus", "");
    (inner as { _overrideFocusable: boolean | null })._overrideFocusable = true;
    inner._inDoc = true;
    const innerBtn = new FakeEl();
    (innerBtn as { _overrideFocusable: boolean | null })._overrideFocusable = true;
    (inner as FakeEl).children = [innerBtn];

    const innerTrigger = new FakeEl();
    innerTrigger._inDoc = true;
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = innerTrigger as unknown as FakeEl | null;

    const api2 = mountFocusTrap(inner as unknown as HTMLElement);

    // Tab depuis extérieur du piège intérieur (body)
    const body = new FakeEl();
    body._inDoc = true;
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = body as unknown as FakeEl | null;
    const rawHandlers = fakeDoc._listeners.get("keydown") ?? new Set();
    let handler: ((e: KeyboardEvent) => void) | null = null;
    for (const h of rawHandlers) { handler = h; break; }
    let ext1Prevented = false;
    const ext1FakeEvent = { key: "Tab" } as KeyboardEvent;
    (ext1FakeEvent as { preventDefault?: () => void }).preventDefault = () => { ext1Prevented = true; };
    if (handler) handler(ext1FakeEvent as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === innerBtn,
      "Tab depuis extérieur → premier du piège intérieur",
    );

    // Fermer le piège intérieur
    destroyFocusTrap(api2);

    // Tab depuis extérieur du piège extérieur (body)
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = body as unknown as FakeEl | null;
    let ext2Prevented = false;
    const ext2FakeEvent = { key: "Tab" } as KeyboardEvent;
    (ext2FakeEvent as { preventDefault?: () => void }).preventDefault = () => { ext2Prevented = true; };
    if (handler) handler(ext2FakeEvent as KeyboardEvent);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === outerBtn,
      "Tab depuis extérieur → premier du piège extérieur",
    );

    destroyFocusTrap(api1);
    label("pièges extérieur/intérieur puis retour au piège extérieur");
  }

  // ── Scenario 13: cible explicite connectée avec cible transitoire déconnectée ──
  {
    const stableTarget = new FakeEl();
    stableTarget._inDoc = true;
    const transient = new FakeEl();
    transient._inDoc = false;
    const { container } = makeScenario();
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = transient as unknown as FakeEl | null;

    const api = mountFocusTrap(container as unknown as HTMLElement, { returnFocus: stableTarget as unknown as HTMLElement });
    destroyFocusTrap(api);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === stableTarget,
      "cible explicite connectée avec transitoire déconnectée → restauration sur cible explicite",
    );
    label("cible explicite connectée avec cible transitoire déconnectée");
  }

  // ── Scenario 14: cible explicite déconnectée avec repli connecté ──
  {
    const badTarget = new FakeEl();
    badTarget._inDoc = false;
    const goodFallback = new FakeEl();
    goodFallback._inDoc = true;
    const { container } = makeScenario();
    (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement = goodFallback as unknown as FakeEl | null;

    const api = mountFocusTrap(container as unknown as HTMLElement, { returnFocus: badTarget as unknown as HTMLElement });
    destroyFocusTrap(api);
    assertOk(
      (fakeDoc as unknown as { activeElement: FakeEl | null }).activeElement === goodFallback,
      "cible explicite déconnectée → fallback sur activeElement connecté",
    );
    label("cible explicite déconnectée avec repli connecté");
  }

  // ── Scenario 15: removeEventListener reçoit le même callback (S4) ──
  // After destroying the last trap the keydown Set must be empty/absent.
  // This is the only trap active at this point.
  {
    const { container: c15 } = makeScenario();
    const api15 = mountFocusTrap(c15 as unknown as HTMLElement);
    assertOk(
      fakeDoc._listeners.has("keydown"),
      "le listener keydown existe après montage",
    );
    destroyFocusTrap(api15);
    const keydownSet = fakeDoc._listeners.get("keydown");
    assertOk(
      !keydownSet || keydownSet.size === 0,
      "après destruction du dernier piège, le Set keydown est absent ou vide",
    );
    label("removeEventListener reçoit le même callback → fuite nulle");
  }

  console.log("\nTous les tests focus-trap passent.");
} finally {
  // Destroy any remaining active actions, clear tracking, restore globals
  for (const api of activeApis) { try { api.destroy(); } catch { /* already destroyed */ } }
  activeApis.clear();
  uninstallFake();
}

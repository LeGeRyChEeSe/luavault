/**
 * Unit tests for theme.svelte.ts — appearance store guards.
 *
 * No Vitest, no jsdom, no dependency.  tsx + node:assert (hand-rolled) only.
 * The module is imported dynamically so the shim runs first; a static import
 * would hoist the import before the shim and hit `$state is not defined`.
 *
 * The harnais (shim + timer control + Tauri mock) was executed and verified
 * on 2026-08-07 — its outputs are reproduced in the brief.
 *
 * NOTE: all test logic is inside an async IIFE to avoid top-level await,
 * which tsx treats as an error (exit code 13).
 */

// ── Shim for Svelte runes ───────────────────────────────────────
(globalThis as Record<string, unknown>).$state = (v: unknown) => v;

// ── Minimal DOM — three literals, no jsdom ──────────────────────
const g = globalThis as Record<string, unknown>;
const html = { dataset: {} as Record<string, string>, style: { setProperty() {} } };
g.document = { documentElement: html };
g.window = g;

// matchMedia behaviour is mutable per-test.
let _darkMode = false;
let _reducedMotion = true;
g.matchMedia = ((query: string) => ({
  matches: query === "(prefers-color-scheme: dark)" ? _darkMode :
           query === "(prefers-reduced-motion: reduce)" ? _reducedMotion : false,
})) as unknown as typeof matchMedia;

g.innerWidth = 1000;
g.innerHeight = 800;

// ── Tauri v2 invoke mock ────────────────────────────────────────
const invokeCalls: { cmd: string; args?: Record<string, unknown> }[] = [];
let resolveNext: ((v: unknown) => void) | null = null;
// Mutable wrapper — TypeScript tracks property access, not callback assignments.
const _reject = { fn: null as ((e: unknown) => void) | null };
g.__TAURI_INTERNALS__ = {
  invoke: (cmd: string, args?: Record<string, unknown>) => {
    invokeCalls.push({ cmd, args: args ?? {} });
    return new Promise((res, rej) => { resolveNext = res; _reject.fn = rej; });
  },
  transformCallback: (cb: unknown) => cb,
};

// ── Timer control — deterministic, no fake timers ───────────────
const timers: (() => void)[] = [];
g.setTimeout = ((fn: () => void) => {
  timers.push(fn);
  return timers.length;
}) as unknown as typeof setTimeout;
g.clearTimeout = ((id: number) => {
  if (id >= 1 && id - 1 < timers.length) timers[id - 1] = () => {};
}) as typeof clearTimeout;

// ── Minimal assert (no node:assert — @types/node is not a dep) ──
const assert = {
  ok(cond: unknown, msg?: string): void {
    if (!cond) throw new Error(msg ?? "assertion failed");
  },
  equal(actual: unknown, expected: unknown, msg?: string): void {
    if (actual !== expected) {
      const where = msg ? `${msg} — ` : "";
      throw new Error(`${where}expected ${expected}, got ${actual}`);
    }
  },
};

// ── Helper: flush the microtask queue ───────────────────────────
const tick = () => Promise.resolve();

// ── Helper: resolve the next pending invoke promise ─────────────
function resolveInvoke(val: unknown): void {
  const r = resolveNext;
  resolveNext = null;
  r?.(val);
}

// Cases actually completed. The runner asserts the exact figure — a suite that
// stops halfway must not read as green.
let cases = 0;
const EXPECTED_CASES = 8;
function label(name: string) {
  cases++;
  console.log(`  ✓ ${name}`);
}

export const themeStateCases = () => cases;

// ── Async IIFE — all test logic lives here to avoid top-level await ──
export const themeStateSuite = (async () => {
  try {
  // ── Import the real module (dynamic — shim runs first) ────────
  const { themeStore, originOf } = await import("../src/lib/theme.svelte");

  // ── 1. hydrate valide l'identifiant ───────────────────────────
  {
    invokeCalls.length = 0;

    themeStore.hydrate("xyz", false);
    assert.equal(themeStore.theme, "azur", "thème inconnu → azur");

    themeStore.hydrate("", false);
    assert.equal(themeStore.theme, "azur", "chaîne vide → azur");

    themeStore.hydrate(null, false);
    assert.equal(themeStore.theme, "azur", "null → azur");

    themeStore.hydrate("emeraude", false);
    assert.equal(themeStore.theme, "emeraude", "thème valide → emeraude");

    label("hydrate valide l'identifiant (inconnu/vide/null → azur)");
  }

  // ── 2. hydrate(null) suit le système, hydrate(false) ne le suit pas ──
  {
    invokeCalls.length = 0;

    // System prefers dark
    _darkMode = true;

    themeStore.hydrate(null, null);
    assert.equal(themeStore.dark, true, "hydrate(null, null) suit le système (dark)");

    // Explicit false must NOT fall through to system
    themeStore.hydrate(null, false);
    assert.equal(themeStore.dark, false, "hydrate(null, false) respecte false (pas le système)");

    // Restore default
    _darkMode = false;

    label("hydrate(null) suit le système, hydrate(false) ne le suit pas");
  }

  // ── 3. paint écrit les DEUX attributs ─────────────────────────
  {
    invokeCalls.length = 0;

    themeStore.hydrate("ambre", true);

    assert.equal(html.dataset.theme, "ambre", "paint écrit data-theme");
    assert.equal(html.dataset.mode, "dark", "paint écrit data-mode");

    label("paint écrit les DEUX attributs (data-theme + data-mode)");
  }

  // ── 4. setTheme(id courant) ne fait rien ──────────────────────
  {
    invokeCalls.length = 0;
    timers.length = 0;

    themeStore.hydrate("azur", false);
    themeStore.setTheme("azur");

    assert.equal(invokeCalls.length, 0, "setTheme(id courant) → zéro invoke");

    label("setTheme(id courant) ne fait rien (pas de repeinte, pas de sauvegarde)");
  }

  // ── 5. toggleDark inverse réellement ──────────────────────────
  {
    invokeCalls.length = 0;
    timers.length = 0;

    themeStore.hydrate("azur", false);
    assert.equal(html.dataset.mode, "light", "mode initial = light");

    themeStore.toggleDark();

    assert.equal(html.dataset.mode, "dark", "toggleDark → dark");
    assert.equal(themeStore.dark, true, "themeStore.dark = true après toggle");

    label("toggleDark inverse réellement et écrit data-mode");
  }

  // ── 6. originOf — trois branches ──────────────────────────────
  {
    // Branche 1 : élément présent → centre du rect
    const mockElement = {
      getBoundingClientRect: () => ({ left: 100, top: 200, width: 200, height: 100 }),
    } as unknown as HTMLElement;

    const ev1 = { currentTarget: mockElement, detail: 0 } as unknown as MouseEvent;
    const o1 = originOf(ev1);
    assert.equal(o1.x, 200, "élément → x = centre rect");
    assert.equal(o1.y, 250, "élément → y = centre rect");

    // Branche 2 : pas d'élément, detail === 0 → centre viewport
    const ev2 = { currentTarget: null, detail: 0, clientX: 0, clientY: 0 } as unknown as MouseEvent;
    const o2 = originOf(ev2);
    assert.equal(o2.x, 500, "detail=0 sans élément → x = viewport/2");
    assert.equal(o2.y, 400, "detail=0 sans élément → y = viewport/2");

    // Branche 3 : pas d'élément, detail !== 0 → pointeur
    const ev3 = { currentTarget: null, detail: 1, clientX: 300, clientY: 150 } as unknown as MouseEvent;
    const o3 = originOf(ev3);
    assert.equal(o3.x, 300, "detail≠0 sans élément → x = clientX");
    assert.equal(o3.y, 150, "detail≠0 sans élément → y = clientY");

    label("originOf : trois branches (élément, clavier, pointeur)");
  }

  // ── 7. Sauvegarde qui échoue ne remonte jamais ────────────────
  {
    invokeCalls.length = 0;

    themeStore.hydrate("azur", false);

    // Trigger persist() via toggleDark (non-transition path, synchronous)
    themeStore.toggleDark();

    // The invoke is now pending — reject it
    if (_reject.fn) _reject.fn(new Error("save failed"));

    // Wait for the promise to settle
    await tick();
    await tick();

    // No error should have surfaced
    assert.equal(invokeCalls.length, 1, "persist a appelé invoke");
    assert.equal(invokeCalls[0].cmd, "set_appearance", "l'invoke est set_appearance");

    label("sauvegarde qui échoue ne remonte jamais");
  }

  // ── 8. View Transition : persist() sauvegarde quelle valeur ? ──
  // Le point à trancher : persist() est HORS du callback startViewTransition.
  // Si le callback est asynchrone (comme l'API réelle), persist() lit this.theme
  // AVANT la mutation → sauvegarde l'ANCIEN thème.
  {
    invokeCalls.length = 0;
    timers.length = 0;

    // Activer le chemin View Transition
    _reducedMotion = false;

    // Bouchon startViewTransition asynchrone (comme l'API réelle)
    (g.document as any).startViewTransition = (cb: () => void) => {
      Promise.resolve().then(cb);
      return { finished: Promise.resolve() };
    };

    themeStore.hydrate("azur", false);

    // setTheme déclenche transition() → startViewTransition(cb) → persist()
    themeStore.setTheme("emeraude");

    // À ce stade : persist() a été appelé AVANT que cb() ne s'exécute.
    // Flush microtask pour exécuter le callback
    await tick();
    await tick();

    // Quelle valeur est partie dans setAppearance ?
    const appearanceCall = invokeCalls.find(c => c.cmd === "set_appearance");
    assert.ok(appearanceCall, "set_appearance a été appelé");

    const args = appearanceCall!.args as { theme: string; dark: boolean } | undefined;
    const savedTheme = args?.theme ?? "INCONNU";

    // Le point est tranché : c'était un bug, il est corrigé. Cette assertion
    // était volontairement permissive le temps de le mesurer — elle acceptait
    // l'ancien comme le nouveau thème, donc elle ne gardait rien. Elle est
    // maintenant stricte : persist() doit envoyer la valeur d'APRÈS la mutation.
    // Remettre `this.persist()` hors du callback de startViewTransition doit
    // faire rougir cette ligne.
    assert.equal(
      savedTheme,
      "emeraude",
      "persist() sauvegarde le thème choisi, pas celui d'avant la transition",
    );

    // Restaurer
    _reducedMotion = true;
    delete (g.document as any).startViewTransition;

    label("View Transition : persist() sauvegarde quelle valeur ?");
  }

  console.log("\nTous les tests theme-state passent.");
  } catch (e) {
    console.error("FATAL:", e);
    throw e;
  }
})();

// ── Structural tripwire ─────────────────────────────────────────
export const __themeStateRan = true;

// ── Completion guard ────────────────────────────────────────────
// A suite that dies mid-way — including by silently exiting on an unresolved
// await — must not read as green. beforeExit fires exactly then, and it is the
// last moment an exit code can still be set.
const proc = (globalThis as { process?: { exitCode?: number; on?: (e: string, cb: () => void) => void } })
  .process;
proc?.on?.("beforeExit", () => {
  if (cases !== EXPECTED_CASES) {
    console.error(
      `FATAL: test-theme-state s'est arrêtée après ${cases} cas sur ${EXPECTED_CASES} ` +
        `— arrêt silencieux (await non résolu ?), pas un succès.`,
    );
    if (proc) proc.exitCode = 1;
  }
});

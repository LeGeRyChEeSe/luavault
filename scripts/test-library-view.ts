/**
 * Structural guard rails for the virtual-scroll wiring in LibraryView.svelte.
 *
 * These are NOT behaviour tests. They read the component source and pin the
 * shape of three wirings that no test in this stack can execute. They stop a
 * revert; they do not prove the wiring works. What does prove it is the manual
 * bench (`npm run bench:library`) — see CLAUDE.md.
 *
 * This stack has no DOM test runner and no npm dependency may be added, so
 * the observer lifecycle (D4), the matchMedia breakpoints (D6) and the
 * threshold gating (D7) are pinned by reading the component source. Each
 * assertion goes red if the wiring it protects is reverted — the failing
 * outputs are recorded in .orchestration/logs/LOT-09-fix01.done.
 *
 * Runs from the project root (validate.ps1 and `npm run test:ts` both do).
 */

// @ts-expect-error — `node:fs` has no types here: @types/node is not a project
// dependency, and an ambient `declare module "node:fs"` is GLOBAL to the
// program, not scoped to scripts/. Measured: with such a file present, a
// frontend module could `import { readFileSync } from "node:fs"` and
// svelte-check still reported 0 errors. This suppression is confined to one
// line, and it turns into an error of its own the day @types/node lands.
import { readFileSync as readFileSyncRaw } from "node:fs";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

function assertOk(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

const src = readFileSync("src/views/LibraryView.svelte", { encoding: "utf8" });

// ── D4 — the ResizeObserver follows the node ─────────────────
// The scroll container is destroyed and recreated whenever the list goes
// through zero results, and may not exist at mount time (search state is
// persisted across navigation). The observer must be installed by a $effect
// that captures the node, not conditionally in onMount.
// Deliberately loose on shape: pinning the exact first statement made a
// correct rewrite (variable renamed, guards reordered) fail, and a guard that
// cries wolf is a guard someone disables. What is pinned is what actually
// breaks: the observer lives inside an effect, it measures something, and it
// is disconnected on teardown.
assertOk(
  /\$effect\([\s\S]{0,400}?ro\.observe\(/.test(src),
  "D4: le ResizeObserver doit être posé dans un $effect (il suit le nœud, qui est recréé)",
);
assertOk(
  /new ResizeObserver\(\(\) => measureRowHeight\(\)\)/.test(src),
  "D4: l'observateur doit re-mesurer la hauteur de ligne — un callback vide passe tout le reste",
);
assertOk(
  src.includes("ro.disconnect()"),
  "D4: l'observateur doit être déconnecté au nettoyage, sinon il fuit à chaque recréation",
);
assertOk(
  !src.includes("if (scrollEl) ro.observe(scrollEl)"),
  "D4: l'observateur ne doit plus être posé conditionnellement au montage",
);

// ── D6 — matchMedia thresholds match Tailwind's ──────────────
// Tailwind's max-lg / max-xl variants switch at 1023.98 / 1279.98 px. Integer
// thresholds leave a gap where the CSS grid is 1-column while vCols believes
// 2 — reachable with fractional viewport widths under Windows 125% scaling.
assertOk(
  src.includes("(max-width: 1023.98px)"),
  "D6: le seuil lg doit être (max-width: 1023.98px) pour s'aligner sur Tailwind",
);
assertOk(
  src.includes("(max-width: 1279.98px)"),
  "D6: le seuil xl doit être (max-width: 1279.98px) pour s'aligner sur Tailwind",
);
assertOk(
  !src.includes("(max-width: 1023px)"),
  "D6: (max-width: 1023px) laisse un écart avec Tailwind entre 1023 et 1024 px",
);
assertOk(
  !src.includes("(max-width: 1279px)"),
  "D6: (max-width: 1279px) laisse un écart avec Tailwind entre 1279 et 1280 px",
);

// ── D7 — no instrumentation below the threshold ──────────────
// Under VIRTUAL_THRESHOLD the grid renders normally: the scroll handler must
// not rewrite vScrollTop every frame, and neither the observer nor the
// breakpoint listeners may be installed.
assertOk(
  /function onGridScroll\(\) \{[\s\S]{0,200}?if \(!useVirtual\) return;/.test(src),
  "D7: onGridScroll doit sortir immédiatement sous le seuil de virtualisation",
);
assertOk(
  src.includes("if (!el || !useVirtual) return;"),
  "D7: l'effet ResizeObserver ne doit s'installer qu'au-dessus du seuil",
);
assertOk(
  /\$effect\(\(\) => \{\s*if \(!useVirtual\) return;\s*const mqLg/.test(src),
  "D7: les écouteurs matchMedia ne doivent s'installer qu'au-dessus du seuil",
);

console.log("  ✓ tripwires structurels LibraryView.svelte (D4/D6/D7)");

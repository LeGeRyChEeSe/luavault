/**
 * Structural guard rails for the offline-mode wiring (LOT-11).
 *
 * Same contract as test-dlc-wiring.ts: these are NOT behaviour tests. They
 * read the sources and pin the shape of wirings no test in this stack can
 * execute — the reachability event listener removed, the indicator no longer
 * gated on the online state, a network action that forgot its `!online` gate.
 *
 * Two copies of each source are read:
 * - STRIPPED (comments and string literals removed) for code-structure motifs.
 * - RAW (untouched) for contract strings (event names, icon names) that are
 *   string literals and would be erased by stripping.
 *
 * Deliberately loose on identifiers, tight on structure: each assertion goes
 * red on a behaviour-breaking revert, not on a rename, a reorder, or a
 * restyle. A guard that cries wolf gets disabled.
 */

// @ts-expect-error — `node:fs` has no types here: @types/node is not a
// project dependency. See the identical note in test-library-view.ts.
import { readFileSync as readFileSyncRaw } from "node:fs";
import { stripCommentsAndStrings } from "./test-dlc-wiring";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

function assertOk(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

// ── Read sources: stripped for code motifs, raw for contract strings ──
const appRaw = readFileSync("src/App.svelte", { encoding: "utf8" });
const app = stripCommentsAndStrings(appRaw);
const api = stripCommentsAndStrings(readFileSync("src/lib/api.ts", { encoding: "utf8" }));
const store = stripCommentsAndStrings(
  readFileSync("src/lib/app-state.svelte.ts", { encoding: "utf8" }),
);
const menu = stripCommentsAndStrings(readFileSync("src/lib/game-menu.ts", { encoding: "utf8" }));
const libraryRaw = readFileSync("src/views/LibraryView.svelte", { encoding: "utf8" });


// ── O1 — the shell seeds the state, then reacts to transitions ──

// O1-1: the reachability event is applied to the store.
// Correct: renaming the handler, reordering the body, extracting a function.
// Broken: removing the listener so transitions never reach the UI.
assertOk(
  /setReachability\(event\.payload\)/.test(app),
  "O1-1: l'événement de joignabilité doit alimenter le store (setReachability(event.payload))",
);

// O1-2: the initial state is seeded once at startup.
// Correct: moving the call within onMount.
// Broken: removing the seed — the indicator would be wrong until the first change.
assertOk(
  /refreshReachability\(\)/.test(app),
  "O1-2: l'état initial doit être lu au démarrage (refreshReachability)",
);

// ── J4 — the event name is pinned on the RAW source ──────────────
// Renaming the event on one side only (Rust or frontend) must fail here.
// This reads the raw file because the event name is a string literal that
// stripping would erase.

// J4-1: the frontend listener uses the exact contract string.
assertOk(
  appRaw.includes("reachability://changed"),
  "J4-1: le littéral « reachability://changed » doit apparaître dans App.svelte (source brute)",
);

// ── O2 — the indicator is a gated state, not an always-on widget ──

// O2-1: the indicator only renders while offline. Anchored on the {#if}
// condition and the presence of an Icon (structure), NOT on a French label
// (which a rename would legitimately change).
// Correct: renaming the pill text, restyling, moving the icon.
// Broken: removing the {#if} so the indicator shows permanently.
{
  const ifIdx = app.indexOf("{#if !appState.online}");
  const afterIf = ifIdx >= 0 ? app.slice(ifIdx, ifIdx + 600) : "";
  const hasIcon = /Icon\s+name=/.test(afterIf);
  assertOk(
    ifIdx >= 0 && hasIcon,
    "O2-1: l'indicateur hors ligne doit être rendu sous {#if !appState.online} avec une icône",
  );
}

// ── O3 — the typed invoke wrapper exists ──────────────────────

// O3-1: getReachability invokes the command with the Reachability type.
// Correct: renaming the command string (stripped anyway), reformatting.
// Broken: removing the wrapper or dropping the typed invoke.
assertOk(
  /export const getReachability[\s\S]{0,80}?invoke<Reachability>/.test(api),
  "O3-1: getReachability doit invoquer la commande typée (invoke<Reachability>)",
);

// ── O4 — the store carries the online state + the explanation ──

// O4-1: the online flag is reactive and defaults to online.
// Correct: moving the field, renaming the getter.
// Broken: a non-reactive flag, or defaulting to offline (banner at startup).
assertOk(
  /online = \$state\(true\)/.test(store),
  "O4-1: l'état en ligne doit être réactif et vrai par défaut (online = $state(true))",
);

// O4-2: the store exposes the offline explanation used by the data-tips.
// Correct: rewording the message.
// Broken: removing offlineTip — the suspended actions lose their explanation.
assertOk(
  /offlineTip/.test(store) && /setReachability/.test(store),
  "O4-2: le store doit exposer offlineTip et setReachability",
);

// ── O5 — local menu actions remain available offline ───────────

// O5-3: a local entry (copy to Steam) is NOT gated on online — offline mode
// must never disable a local action.
// Correct: any rewrite of the copy-to-Steam entry that stays local.
// Broken: gating a local action on `!online`.
{
  const anchor = menu.indexOf("copyToSteam(status.app_id)");
  const pushStart = anchor >= 0 ? menu.lastIndexOf("items.push({", anchor) : -1;
  const body = pushStart >= 0 && anchor > pushStart ? menu.slice(pushStart, anchor) : "";
  assertOk(
    anchor >= 0 && !body.includes("disabled: !online"),
    "O5-3: une action locale (copier vers Steam) ne doit pas être gated sur online",
  );
}

// ── J1 — bulk patch buttons and carousel are gated ────────────

// J1-1: "Installer tous les patchs" and "Tout installer" in LibraryView
// are separately gated.  Each exact onclick anchor is located in the raw
// source, then its own ActionButton and disabled expression are inspected.
// This cannot be satisfied by the `repair` tooltip's unrelated online text.
{
  function bulkActionButton(onclick: string): string {
    const anchor = libraryRaw.indexOf(onclick);
    const start = anchor >= 0 ? libraryRaw.lastIndexOf("<ActionButton", anchor) : -1;
    const nextButton = anchor >= 0 ? libraryRaw.indexOf("<ActionButton", anchor) : -1;
    const end = anchor >= 0 ? libraryRaw.indexOf("/>", anchor) : -1;
    return start >= 0 && end >= anchor && (nextButton < 0 || end < nextButton)
      ? libraryRaw.slice(start, end + 2)
      : "";
  }

  function disabledExpression(button: string): string {
    return button.match(/disabled=\{([^}]*)\}/)?.[1] ?? "";
  }

  const fixes = bulkActionButton('onclick={() => void startBulk("fixes")}');
  const all = bulkActionButton('onclick={() => void startBulk("all")}');
  assertOk(
    disabledExpression(fixes).includes("!appState.online"),
    "J1-1a: le bouton bulk « Installer tous les patchs » doit être gated sur !appState.online",
  );
  assertOk(
    disabledExpression(all).includes("!appState.online"),
    "J1-1b: le bouton bulk « Tout installer » doit être gated sur !appState.online",
  );
}

console.log(
  "  ✓ tripwires structurels hors ligne (App/api/app-state/game-menu/LibraryView : O1–O4, O5-3, J1, J4)",
);

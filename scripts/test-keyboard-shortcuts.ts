/**
 * LOT-20 — keyboard-shortcuts pure-function tests and wiring guard rails.
 *
 * Structure:
 *  1. Pure-function tests for resolveShortcut and isEditableTarget
 *  2. Wiring guards on stripped App.svelte source (each branch isolated)
 *  3. Wiring guards on the help modal (ARIA, focusTrap, three lines, close button,
 *     backdrop click, onkeydown binding)
 *  4. Adjacent guards: no global Escape handler, existing Escape owners remain,
 *     stores not reset
 *
 * Uses stripCommentsAndStrings / stripComments from test-dlc-wiring.ts.
 */

// @ts-expect-error — `node:fs` has no types here.
import { readFileSync as readFileSyncRaw } from "node:fs";
import { stripComments, stripCommentsAndStrings } from "./test-dlc-wiring";
import { resolveShortcut, isEditableTarget } from "../src/lib/keyboard-shortcuts";
import { fr } from "../src/lib/i18n/fr";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

function assertOk(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

// ── Pure-function tests ───────────────────────────────────────

// T1 — Ctrl+F has no application-wide action.
assertOk(
  resolveShortcut({ key: "f", ctrlKey: true, altKey: false, editable: false }) === null,
  "T1: Ctrl+F → null",
);

// T2 — resolveShortcut returns "open-logs" for Ctrl+L
assertOk(
  resolveShortcut({ key: "l", ctrlKey: true, altKey: false, editable: false }) === "open-logs",
  "T2: Ctrl+L → open-logs",
);

// T3 — resolveShortcut returns "open-help" for ?
assertOk(
  resolveShortcut({ key: "?", ctrlKey: false, altKey: false, editable: false }) === "open-help",
  "T3: ? → open-help",
);

// T4 — case-insensitive letter
assertOk(
  resolveShortcut({ key: "F", ctrlKey: true, altKey: false, editable: false }) === null,
  "T4: Ctrl+F uppercase → null",
);
assertOk(
  resolveShortcut({ key: "L", ctrlKey: true, altKey: false, editable: false }) === "open-logs",
  "T4: Ctrl+L uppercase → open-logs",
);

// T5 — null when Ctrl absent
assertOk(
  resolveShortcut({ key: "f", ctrlKey: false, altKey: false, editable: false }) === null,
  "T5: f without Ctrl → null",
);

// T6 — null when Alt present
assertOk(
  resolveShortcut({ key: "f", ctrlKey: true, altKey: true, editable: false }) === null,
  "T6: Ctrl+Alt+F → null",
);

// T7 — null when editable
assertOk(
  resolveShortcut({ key: "f", ctrlKey: true, altKey: false, editable: true }) === null,
  "T7: Ctrl+F with editable → null",
);

// T8 — no action for other keys
assertOk(
  resolveShortcut({ key: "x", ctrlKey: true, altKey: false, editable: false }) === null,
  "T8: Ctrl+X → null",
);
assertOk(
  resolveShortcut({ key: "a", ctrlKey: false, altKey: false, editable: false }) === null,
  "T8: a → null",
);

// ── isEditableTarget tests ────────────────────────────────────

// T9 — INPUT is editable
assertOk(
  isEditableTarget({ tagName: "INPUT", isContentEditable: false } as unknown as EventTarget),
  "T9: INPUT → editable",
);

// T10 — TEXTAREA is editable
assertOk(
  isEditableTarget({ tagName: "TEXTAREA", isContentEditable: false } as unknown as EventTarget),
  "T10: TEXTAREA → editable",
);

// T11 — SELECT is editable
assertOk(
  isEditableTarget({ tagName: "SELECT", isContentEditable: false } as unknown as EventTarget),
  "T11: SELECT → editable",
);

// T12 — DIV with isContentEditable is editable
assertOk(
  isEditableTarget({ tagName: "DIV", isContentEditable: true } as unknown as EventTarget),
  "T12: DIV contenteditable → editable",
);

// T13 — DIV without isContentEditable is not editable
assertOk(
  !isEditableTarget({ tagName: "DIV", isContentEditable: false } as unknown as EventTarget),
  "T13: DIV not contenteditable → not editable",
);

// T14 — null target is not editable
assertOk(
  !isEditableTarget(null),
  "T14: null → not editable",
);

// T15 — non-object target is not editable
assertOk(
  !isEditableTarget("string" as unknown as EventTarget),
  "T15: string → not editable",
);

// T16 — descendant inherits isContentEditable from ancestor
// In a real browser, a child of contenteditable=true inherits the property.
// Our structural contract uses isContentEditable directly, so this covers it.
assertOk(
  isEditableTarget({ tagName: "P", isContentEditable: true } as unknown as EventTarget),
  "T16: P contenteditable (inherited) → editable",
);

// ── T17-T20 — wiring guards on stripped App.svelte ────────────
// Each branch is isolated and verified with its own preventDefault() and effect.

const appStripped = stripCommentsAndStrings(
  readFileSync("src/App.svelte", { encoding: "utf8" }),
);
const appNoComments = stripComments(
  readFileSync("src/App.svelte", { encoding: "utf8" }),
);
// Shared backtick character for template-literal decoys.
const bt = String.fromCharCode(96);

// T17 — exactly one declaration of `function handleKeydown` and exactly one
//       binding `<svelte:window onkeydown={handleKeydown}>`.  Each pattern
//       must appear once — a simple global count is not enough.
const declCount = (appNoComments.match(/function\s+handleKeydown\s*\(/g) || []).length;
const bindCount = (appNoComments.match(/<svelte:window[^>]*onkeydown=\{handleKeydown\}/g) || []).length;
assertOk(
  declCount === 1,
  `T17a: exactly one handleKeydown declaration (found ${declCount})`,
);
assertOk(
  bindCount === 1,
  `T17b: exactly one <svelte:window onkeydown={handleKeydown}> binding (found ${bindCount})`,
);

// T18 — resolver called in handleKeydown body with object arg, result assigned
// to `const action`. A naked call to resolveShortcut followed by a constant
// action must fail.  The `const action = resolveShortcut({` pattern proves the
// wiring is complete.
// Uses findPatternOutsideStrings so that a decoy like
// `const action = "resolveShortcut({"` is rejected.
const hkBody = extractNamedFunction(appNoComments, "handleKeydown");
// Pass the full body to findPatternOutsideStrings; the ^ anchor ensures
// the match starts at position 0 of the scanned slice (the body itself).
const t18Idx = hkBody !== null && findPatternOutsideStrings(
  hkBody,
  /^const\s+action\s*=\s*resolveShortcut\s*\(\s*\{/,
) >= 0;
assertOk(
  t18Idx,
  "T18: wiring — const action = resolveShortcut({ inside handleKeydown body",
);

// ── T19 — modal guard present inside handleKeydown body ─────────
// Must find the complete conditional: if (document.querySelector(...) !== null)
// return;  This binds the guard to the return so that
// `if (false) return` after resolveShortcut doesn't satisfy this guard
// (mutation 28).  The !== null proves the guard direction (mutation 27).
// The guard+return must appear before the resolver call.
//
// findPatternOutsideStrings scans char-by-char (normal / single / double /
// template state), only matching the anchored regex when in the "normal"
// state.  This rejects a guard that has been moved inside a string literal
// while accepting a real conditional.
function findPatternOutsideStrings(
  src: string,
  anchoredRegex: RegExp,
): number {
  let pos = 0;
  while (pos < src.length) {
    const ch = src[pos];
    let state: "normal" | "single" | "double" | "template" = "normal";
    let delim: string = "";
    if (ch === "'") { state = "single"; delim = "'"; }
    else if (ch === '"') { state = "double"; delim = '"'; }
    else if (ch === "`") { state = "template"; delim = "`"; }
    else {
      // In normal state, try the anchored regex at this position.
      if (anchoredRegex.test(src.slice(pos))) return pos;
      pos++;
      continue;
    }
    // Skip until closing quote (respecting backslash escapes).
    pos++;
    while (pos < src.length) {
      const inner = src[pos];
      if (inner === "\\") { pos += 2; continue; }
      if (state === "template" && inner === "$") {
        // Skip ${...} interpolations (naive brace counting).
        pos++;
        let depth = 1;
        while (pos < src.length && depth > 0) {
          if (src[pos] === "{") depth++;
          else if (src[pos] === "}") depth--;
          pos++;
        }
        continue;
      }
      if (inner === delim) break;
      pos++;
    }
    pos++;
  }
  return -1;
}

let guardReturnIdx = -1, resolverIdx = -1;
if (hkBody !== null) {
  const guardedSrc = hkBody.slice(0, hkBody.indexOf("resolveShortcut"));
  guardReturnIdx = findPatternOutsideStrings(
    guardedSrc,
    /^if\s*\(\s*document\.querySelector\([^)]*\[aria-modal\s*=\s*"true"\][^)]*\)\s*!==\s*null\s*\)\s*return\s*;/,
  );
  resolverIdx = hkBody.indexOf("resolveShortcut");
}
assertOk(
  hkBody !== null && guardReturnIdx >= 0 && resolverIdx >= 0 &&
    guardReturnIdx < resolverIdx,
  "T19: wiring — if (querySelector [aria-modal] !== null) return; before resolveShortcut in handleKeydown body",
);

// Auto-tests positifs / négatifs sur findPatternOutsideStrings
// Vraie garde acceptée (déjà prouvée par l'assert ci-dessus).
// M32 : garde complète dans un template literal refusée.
const guardText = 'if (document.querySelector(\'[aria-modal="true"]\') !== null) return;';
const templateDecoy = 'const decoy = ' + bt + guardText + bt + '; if (false) return;';
assertOk(
  findPatternOutsideStrings(
    templateDecoy,
    /^if\s*\(\s*document\.querySelector\([^)]*\[aria-modal\s*=\s*"true"\][^)]*\)\s*!==\s*null\s*\)\s*return\s*;/,
  ) === -1,
  "T19-auto: M32 — full guard inside backtick template rejected",
);
// Vrai code accepté.
assertOk(
  findPatternOutsideStrings(
    `    if (document.querySelector('[aria-modal="true"]') !== null) return;\n`,
    /^if\s*\(\s*document\.querySelector\([^)]*\[aria-modal\s*=\s*"true"\][^)]*\)\s*!==\s*null\s*\)\s*return\s*;/,
  ) >= 0,
  "T19-auto: real guard accepted",
);

// T20 — each branch has its own preventDefault() **and** its own effect.
// extractBranch finds action === "open-*" outside strings before extracting braces.
// Each expression check uses findPatternOutsideStrings so that a lure like
// `void 'view = "logs"'` or `void "e.preventDefault()"` is rejected.
function extractBranch(body: string, actionName: string): string | null {
  // Find the comparison outside strings first.
  const actionPattern = `action === "${actionName}"`;
  const idx = findPatternOutsideStrings(body, new RegExp(`^\\baction\\s*===\\s*"${actionName.replace(/[.*+?^=(){}|[\]\\]/g, '\\$&')}"`));
  if (idx === -1) return null;
  const start = body.indexOf("{", idx);
  if (start === -1) return null;
  let depth = 0;
  let pos = start;
  while (pos < body.length) {
    const ch = body[pos];
    if (ch === "{") depth++;
    else if (ch === "}") { depth--; if (depth === 0) return body.slice(start, pos + 1); }
    pos++;
  }
  return null;
}
function findCodeIndex(body: string, anchoredRegex: RegExp): number {
  return findPatternOutsideStrings(body, anchoredRegex);
}
// Search was intentionally removed from the public edition: ShortcutAction in
// src/lib/keyboard-shortcuts.ts only permits open-logs, open-help, and null, and
// App.svelte has no open-search branch. Do not retain assertions for that removed contract.
const logsBranch = extractBranch(appNoComments, "open-logs");
const logsPDIx = logsBranch !== null && findCodeIndex(logsBranch, /^e\.preventDefault\(\)/) >= 0;
const logsViewIdx = logsBranch !== null && findCodeIndex(logsBranch, /^view\s*=\s*"logs"/) >= 0;
assertOk(
  logsBranch !== null && logsPDIx,
  "T20b: open-logs branch has its own preventDefault()",
);
assertOk(
  logsBranch !== null && logsViewIdx,
  "T20b: open-logs branch sets view = \"logs\"",
);
if (logsBranch) {
  const pdIdx = findCodeIndex(logsBranch, /^e\.preventDefault\(\)/);
  const viewIdx = findCodeIndex(logsBranch, /^view\s*=\s*"logs"/);
  assertOk(
    pdIdx >= 0 && viewIdx >= 0 && pdIdx < viewIdx,
    "T20b: open-logs branch: preventDefault before view = \"logs\"",
  );
}
const helpBranch = extractBranch(appNoComments, "open-help");
const helpPDIx = helpBranch !== null && findCodeIndex(helpBranch, /^e\.preventDefault\(\)/) >= 0;
const helpShIdx = helpBranch !== null && findCodeIndex(helpBranch, /^showHelp\s*=\s*true/) >= 0;
assertOk(
  helpBranch !== null && helpPDIx,
  "T20c: open-help branch has its own preventDefault()",
);
assertOk(
  helpBranch !== null && helpShIdx,
  "T20c: open-help branch sets showHelp = true",
);
if (helpBranch) {
  const pdIdx = findCodeIndex(helpBranch, /^e\.preventDefault\(\)/);
  const shIdx = findCodeIndex(helpBranch, /^showHelp\s*=\s*true/);
  assertOk(
    pdIdx >= 0 && shIdx >= 0 && pdIdx < shIdx,
    "T20c: open-help branch: preventDefault before showHelp = true",
  );
}

// ── T22-T26 — wiring guards on the help modal ─────────────────
// Isolate the {#if showHelp} ... {/if} block and verify everything
// inside it only. The local handler proof (T27) remains separate.

// Helper: extract the {#if showHelp} ... {/if} block.
// {#if showHelp} opens and closes on the same token so depth counting
// returns immediately.  Use explicit start/end markers instead.
function extractShowHelpBlock(body: string): string | null {
  const start = body.indexOf("{#if showHelp}");
  if (start === -1) return null;
  const end = body.indexOf("{/if}", start);
  if (end === -1) return null;
  return body.slice(start, end + 5); // +5 for length of "{/if}"
}
const helpBlock = extractShowHelpBlock(appNoComments);
assertOk(
  helpBlock !== null,
  "T22: {#if showHelp} block exists",
);

// T22 — ARIA contract inside the help block only
assertOk(
  helpBlock !== null && /role="dialog"/.test(helpBlock),
  "T22a: modal — role=dialog inside help block",
);
assertOk(
  helpBlock !== null && /aria-modal="true"/.test(helpBlock),
  "T22b: modal — aria-modal inside help block",
);
assertOk(
  helpBlock !== null && /aria-labelledby=["']help-dialog-title["']/.test(helpBlock),
  "T22c: modal — aria-labelledby inside help block",
);
assertOk(
  helpBlock !== null && /tabindex=["']-1["']/.test(helpBlock),
  "T22d: modal — tabindex=-1 inside help block",
);
assertOk(
  helpBlock !== null && /use:focusTrap/.test(helpBlock),
  "T22e: modal — use:focusTrap inside help block",
);

// T23 — three independent help lines inside the help block only
// Each line must contain its key combo AND its purpose text.
// Extract all <li>...</li> from the help block.
function extractLiBlocks(body: string): string[] {
  const blocks: string[] = [];
  let pos = 0;
  while (pos < body.length) {
    const openIdx = body.indexOf("<li", pos);
    if (openIdx === -1) break;
    const closeIdx = body.indexOf("</li>", openIdx);
    if (closeIdx === -1) break;
    blocks.push(body.slice(openIdx, closeIdx + 5));
    pos = closeIdx + 5;
  }
  return blocks;
}
const liBlocks = extractLiBlocks(helpBlock ?? "");
// I18N-26 : le texte vit dans le catalogue, le <li> ne porte plus que sa clé.
// Les DEUX moitiés sont exigées — le <li> qui cite ses touches ET sa clé, et la
// valeur du catalogue qui porte le mot. La troisième ligne cite deux clés : la
// touche « Échap » est traduite elle aussi (« Esc » en anglais).
const lineChecks = [
  { tokens: ["Ctrl", "L"], key: "shell.help.logs", text: "journaux" },
  { tokens: ["?"], key: "shell.help.help", text: "aide" },
  { tokens: ["shell.help.key.escape"], key: "shell.help.escape", text: "Fermer" },
] as const;
let foundLines = 0;
for (const check of lineChecks) {
  const wired = liBlocks.some((li) =>
    check.tokens.every((k) => li.includes(k)) && li.includes(check.key),
  );
  if (wired && fr[check.key].toLowerCase().includes(check.text.toLowerCase())) foundLines++;
}
assertOk(
  foundLines >= 3,
  `T23: modal — all 3 help lines present inside help block (found ${foundLines}/3)`,
);

// T24 — close button text and its action inside the help block only
// Isolate the <button>...</button> block containing "Fermer" and require
// its own onclick with showHelp=false.
// An onclick="" (empty) with showHelp=false in a data-* attribute must fail.
// The helper below extracts the actual onclick expression from a button block.
function extractOnclickExpr(buttonBlock: string): string | null {
  // Match onclick={...} and extract the expression.
  const m = buttonBlock.match(/onclick\s*=\s*\{([^}]*)\}/);
  if (m) return m[1];
  // Also handle onclick="handler(...)" style.
  const m2 = buttonBlock.match(/onclick\s*=\s*"([^"]*)"/);
  if (m2) return m2[1];
  return null;
}
// Auto-tests positifs/negatifs pour extractOnclickExpr
assertOk(
  extractOnclickExpr('<button onclick={() => (showHelp = false)}>') !== null &&
    /showHelp\s*=\s*false/.test(extractOnclickExpr('<button onclick={() => (showHelp = false)}>')!),
  "extractOnclickExpr: vrai handler {…} accepte",
);
assertOk(
  !/showHelp\s*=\s*false/.test(extractOnclickExpr('<button onclick="" data-decoy="showHelp = false">') ?? ""),
  "extractOnclickExpr: handler vide + data-decoy refuse",
);
const fermerButtonBlocks = extractButtonBlocks(helpBlock ?? "");
const fermerButton = fermerButtonBlocks.find((b) => /shell\.help\.close/.test(b));
const fermerOnclickRaw = extractOnclickExpr(fermerButton ?? "");
// Use findPatternOutsideStrings so that a lure like `void "showHelp = false"`
// is neutralised while a real `showHelp = false` survives.
const fermerOnclick = fermerOnclickRaw !== null
  ? findPatternOutsideStrings(fermerOnclickRaw, /^showHelp\s*=\s*false/) >= 0
  : false;
assertOk(
  fermerButton !== undefined,
  "T24a: modal — close button wired to t(\"shell.help.close\") inside help block",
);
assertOk(
  /Fermer/.test(fr["shell.help.close"]),
  "T24a-bis: la valeur du catalogue doit rester un libellé de fermeture",
);
assertOk(
  fermerOnclick,
  "T24b: modal — close button onclick expression sets showHelp=false inside help block",
);
// Negative: M33 — lure inside double quotes rejected.
// Pass the exact onclick expression body that M33 produces.
const m33 = '() => { void "showHelp = false"; }';
assertOk(
  findPatternOutsideStrings(m33, /^showHelp\s*=\s*false/) === -1,
  "T24-auto: M33 — double-quoted lure rejected",
);

// T25 — backdrop click: require onclick + e.target === e.currentTarget
// and showHelp=false isolated in the backdrop div only.
// Extract from the last <div before role="presentation" to the next <div
// (which opens the dialog). This slice contains only the backdrop element.
const backdropSlice = helpBlock !== null ? helpBlock.slice(
  helpBlock.lastIndexOf("<div", helpBlock.indexOf('role="presentation"')),
  helpBlock.indexOf("<div", helpBlock.indexOf('role="presentation"') + 1),
) : null;
const backdropOnclickRaw = backdropSlice !== null ? extractOnclickExpr(backdropSlice) : null;
// Same findPatternOutsideStrings pass as T24: neutralises string-literal lures.
const backdropOk = backdropOnclickRaw !== null
  ? findPatternOutsideStrings(backdropOnclickRaw, /^e\.target\s*===\s*e\.currentTarget/) >= 0 &&
    findPatternOutsideStrings(backdropOnclickRaw, /^showHelp\s*=\s*false/) >= 0
  : false;
assertOk(
  backdropSlice !== null && /role="presentation"/.test(backdropSlice) &&
    backdropOk,
  "T25: modal — backdrop div has role, onclick expression with e.target===e.currentTarget and showHelp=false",
);
// Negative: M34 — lure inside single quotes rejected.
// Pass the exact onclick expression body that M34 produces.
const m34 = "() => { void 'e.target === e.currentTarget; showHelp = false'; }";
assertOk(
  findPatternOutsideStrings(m34, /^showHelp\s*=\s*false/) === -1,
  "T25-auto: M34 — single-quoted lure rejected",
);

// T26 — onkeydown={handleHelpEscape} on the dialog div inside the help block
const dialogOnKeyDown = helpBlock !== null && /role="dialog"[\s\S]*?onkeydown=\{handleHelpEscape\}/.test(helpBlock);
assertOk(
  dialogOnKeyDown,
  "T26: modal — onkeydown={handleHelpEscape} on dialog inside help block",
);

// T27 — handleHelpEscape body: Escape key → preventDefault + stopPropagation + showHelp=false
// Each expression uses findCodeIndex so that a lure like
// `void 'e.preventDefault()'` or `void 'e.stopPropagation()'` is rejected.
const handleHelpEscapeBody = extractNamedFunction(appNoComments, "handleHelpEscape");
const t27Escape = handleHelpEscapeBody !== null &&
  findCodeIndex(handleHelpEscapeBody, /^\s*if\s*\(\s*e\.key\s*===?\s*["']Escape["']/) >= 0;
const t27Pd = handleHelpEscapeBody !== null && findCodeIndex(handleHelpEscapeBody, /^e\.preventDefault\(\)/) >= 0;
const t27Ss = handleHelpEscapeBody !== null && findCodeIndex(handleHelpEscapeBody, /^e\.stopPropagation\(\)/) >= 0;
const t27Sh = handleHelpEscapeBody !== null && findCodeIndex(handleHelpEscapeBody, /^showHelp\s*=\s*false/) >= 0;
assertOk(
  t27Escape && t27Pd && t27Ss && t27Sh,
  "T27: modal — handleHelpEscape owns Escape (preventDefault, stopPropagation, closes showHelp)",
);

// ── T28-T30 — adjacent guards ─────────────────────────────────

// T28 — no global Escape handler in App.svelte
// The global handleKeydown should not have a case for Escape
const handlerBody = extractNamedFunction(appNoComments, "handleKeydown");
const globalEscapeFound = handlerBody && /e\.key\s*===?\s*["']Escape["']/.test(handlerBody);
assertOk(
  !globalEscapeFound,
  "T28: adjacent — no global Escape handler in App.svelte",
);

// T29 — local Escape owner on help modal
const handleHelpEscapeFound = /handleHelpEscape/.test(appNoComments);
assertOk(
  handleHelpEscapeFound,
  "T29: adjacent — local Escape handler on help modal",
);

// T30 — existing Escape owners remain with precise patterns
// extractNamedFunction uses brace-depth counting to capture the full body.
function extractNamedFunction(body: string, name: string): string | null {
  const idx = body.indexOf(`function ${name}`);
  if (idx === -1) return null;
  const start = body.indexOf("{", idx);
  if (start === -1) return null;
  let depth = 0;
  let pos = start;
  while (pos < body.length) {
    const ch = body[pos];
    if (ch === "{") depth++;
    else if (ch === "}") { depth--; if (depth === 0) return body.slice(start, pos + 1); }
    pos++;
  }
  return null;
}
// extractEscapeBranch: isolate the Escape case/branch within a handler body.
// For switch-case: finds `case "Escape"` and extracts that case block.
// For if-else: finds `e.key === "Escape"` and extracts that if block.
function extractEscapeBranch(body: string, handlerName: string): string | null {
  const handlerBody = extractNamedFunction(body, handlerName);
  if (!handlerBody) return null;
  // Try switch-case first: case "Escape"
  const caseIdx = handlerBody.indexOf(`case "Escape"`);
  if (caseIdx >= 0) {
    // Extract from case to next case/default or closing brace of switch.
    const nextCase = handlerBody.indexOf("case ", caseIdx + 6);
    const nextDefault = handlerBody.indexOf("default", caseIdx + 6);
    const nextBr = Math.min(
      nextCase >= 0 ? nextCase : Infinity,
      nextDefault >= 0 ? nextDefault : Infinity,
    );
    return handlerBody.slice(caseIdx, nextBr);
  }
  // Try if-branch: e.key === "Escape"
  const ifIdx = handlerBody.indexOf("e.key === \"Escape\"");
  if (ifIdx >= 0) {
    const start = handlerBody.indexOf("{", ifIdx);
    if (start === -1) return null;
    let depth = 0;
    let pos = start;
    while (pos < handlerBody.length) {
      const ch = handlerBody[pos];
      if (ch === "{") depth++;
      else if (ch === "}") { depth--; if (depth === 0) return handlerBody.slice(start, pos + 1); }
      pos++;
    }
    return null;
  }
  return null;
}
const contextMenu = stripComments(
  readFileSync("src/components/ContextMenu.svelte", { encoding: "utf8" }),
);
const tagEditor = stripComments(
  readFileSync("src/components/TagEditor.svelte", { encoding: "utf8" }),
);
const gameSpotlight = stripComments(
  readFileSync("src/components/GameSpotlight.svelte", { encoding: "utf8" }),
);
// ContextMenu: Escape branch must have preventDefault AND onclose()
const cmEscape = extractEscapeBranch(contextMenu, "handleKeydown");
assertOk(
  cmEscape !== null && /e\.preventDefault\(\)/.test(cmEscape),
  "T30a: adjacent — ContextMenu Escape branch has preventDefault",
);
assertOk(
  cmEscape !== null && /onclose\(\)/.test(cmEscape),
  "T30a: adjacent — ContextMenu Escape branch calls onclose()",
);
// TagEditor: Escape branch must have preventDefault AND onclose()
const teEscape = extractEscapeBranch(tagEditor, "handleKeydown");
assertOk(
  teEscape !== null && /e\.preventDefault\(\)/.test(teEscape),
  "T30b: adjacent — TagEditor Escape branch has preventDefault",
);
assertOk(
  teEscape !== null && /onclose\(\)/.test(teEscape),
  "T30b: adjacent — TagEditor Escape branch calls onclose()",
);
// GameSpotlight: onKeyDown with Escape → close, and shotIndex before close
const gsBody = extractNamedFunction(gameSpotlight, "onKeyDown");
const gsEscapeOwner = gsBody !== null && /if\s*\(\s*event\.key\s*===?\s*["']Escape["']\s*\)\s*close\s*\(/.test(gsBody);
assertOk(
  gsEscapeOwner,
  "T30c: adjacent — GameSpotlight onKeyDown owns Escape (closes card)",
);
// Verify shotIndex assignment precedes close() in the Escape branch
if (gsBody) {
  const shotIdx = gsBody.indexOf("shotIndex");
  const closeIdx = gsBody.indexOf("close()");
  assertOk(
    shotIdx >= 0 && closeIdx >= 0 && shotIdx < closeIdx,
    "T30d: adjacent — GameSpotlight shotIndex set before close()",
  );
}

// T31 — handler body must not touch searchState or logsState
// The navigation handler must not reset or reassign either store.
assertOk(
  handlerBody !== null && !/searchState/.test(handlerBody),
  "T31a: adjacent — handleKeydown does not reference searchState",
);
assertOk(
  handlerBody !== null && !/logsState/.test(handlerBody),
  "T31b: adjacent — handleKeydown does not reference logsState",
);

// T32 — button visible: help button with onclick sets showHelp=true
// Use a brace-depth counter to extract each <button>...</button> block.
function extractButtonBlocks(body: string): string[] {
  const blocks: string[] = [];
  let pos = 0;
  while (pos < body.length) {
    const openIdx = body.indexOf("<button", pos);
    if (openIdx === -1) break;
    const closeIdx = body.indexOf("</button>", openIdx);
    if (closeIdx === -1) break;
    blocks.push(body.slice(openIdx, closeIdx + 9));
    pos = closeIdx + 9;
  }
  return blocks;
}
const buttonBlocks = extractButtonBlocks(appNoComments);
const helpButtonFound = buttonBlocks.some(
  (b) => /data-tip=\{t\("shell\.help\.title"\)\}/.test(b) &&
         /onclick/.test(b) &&
         /showHelp\s*=\s*true/.test(b),
);
assertOk(
  helpButtonFound,
  "T32: button — visible help button with onclick sets showHelp=true",
);
assertOk(
  fr["shell.help.title"] === "Raccourcis clavier",
  "T32b: la valeur doit rester « Raccourcis clavier » au caractère près — le banc " +
    "graphique sélectionne ce bouton par son data-tip rendu (shell.e2e.ts, shortcuts.e2e.ts)",
);

// ── All tests passed ──────────────────────────────────────────
console.log("LOT-20 keyboard-shortcuts: all tests passed");

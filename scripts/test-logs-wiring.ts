/**
 * Structural guard rails for the log view (LOT-18): filter-before-truncate,
 * the honest counter, the two copies and the confirmed clear of
 * LogView.svelte.
 *
 * Same contract as test-search-wiring.ts: these are NOT behaviour tests.
 * The behaviour is pinned by the pure-function tests in
 * test-virtual-scroll.ts (log-filter.ts: level buckets, case- and
 * accent-insensitive search, filter-before-truncate, the honest counter,
 * the copy format — each goes red when the invariant it protects is
 * mutated). The guards here pin the WIRING neither can see: which list
 * the view renders, which numbers the counter reads, which list each
 * copy button serialises, and that the clear goes through ConfirmButton.
 *
 * Every pattern runs on a STRIPPED copy of the source
 * (stripCommentsAndStrings / stripComments from test-dlc-wiring.ts), so a
 * commented-out call or a tooltip quoting the markup never satisfies a
 * guard. Expression pins run on the strings-stripped copy; pins that must
 * still see string literals (import paths, the toast kind) run on the
 * stripComments copy.
 *
 * Deliberately loose on shape: each assertion goes red on a
 * behaviour-breaking revert, not on a rename or a reformat.
 */

// @ts-expect-error — `node:fs` has no types here: @types/node is not a
// project dependency. See the identical note in test-search-wiring.ts.
import { readFileSync as readFileSyncRaw } from "node:fs";
import { stripComments, stripCommentsAndStrings } from "./test-dlc-wiring";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

function assertOk(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

// ── Read and strip the sources ────────────────────────────────
const logView = stripCommentsAndStrings(
  readFileSync("src/views/LogView.svelte", { encoding: "utf8" }),
);
const logViewStr = stripComments(
  readFileSync("src/views/LogView.svelte", { encoding: "utf8" }),
);
const logsStateStr = stripComments(
  readFileSync("src/lib/logs-state.svelte.ts", { encoding: "utf8" }),
);

// ── U1 — filter and search BEFORE the truncation, never after ─
// U1-1: the rendered list comes from displayLogs applied to the WHOLE
// buffer — displayLogs filters first and slices last (pinned by the pure
// tests). A direct slice of the buffer here is the trap: searching a word
// that only appears past the limit would come up empty and lie about it.
assertOk(
  /displayLogs\(allLogs,[\s\S]{0,120}?LOG_DISPLAY_LIMIT\)/.test(logView),
  "U1-1: la liste rendue passe par displayLogs(allLogs, …, LOG_DISPLAY_LIMIT) — filtrer avant tronquer",
);
// U1-2: the counter's source is filtered the same way, on the whole buffer.
// Anchored on `filtered`'s OWN definition: filterLogs(allLogs, …) also
// appears inside countFor, so the unanchored pin survived a mutation of
// the counter's source — filterLogs(allLogs.slice(-200), …) stayed green
// while the counter lied again.
assertOk(
  /const filtered = \$derived\(filterLogs\(allLogs,[\s\S]{0,120}?logsState\.search\)\)/.test(logView),
  "U1-2: la source du compteur (filtered) est filtrée sur le tampon entier, pas sur une liste déjà tronquée",
);
// U1-3: the view renders `logs` — the filtered, then truncated, slice.
assertOk(
  /\{#each logs as entry/.test(logView),
  "U1-3: la vue rend logs, la tranche filtrée puis tronquée",
);

// ── U2 — the counter tells the truth ─
// What matches, over what exists, and what is actually on screen: the
// three lists the view really holds, none of them swapped. Depuis I18N-28
// la phrase vit dans le catalogue : épingler la seule composition
// laisserait les valeurs se vider, épingler les seules valeurs laisserait
// la vue les câbler sur les mauvaises longueurs.
assertOk(
  /const n = filtered\.length;\s*const total = allLogs\.length;\s*const shown = logs\.length;/.test(
    logViewStr,
  ),
  "U2-1: le compteur lit ce qui correspond (filtered), ce qui existe (allLogs) et ce qui est affiché (logs), dans cet ordre",
);
assertOk(
  /shown < n[\s\S]{0,200}?logs\.count\.all-truncated[\s\S]{0,200}?logs\.count\.filtered-truncated/.test(
    logViewStr,
  ),
  "U2-2: la troncature choisit une clé qui la déclare",
);
// U2-3: la branche SANS troncature, que U2-2 ne regarde pas. Intervertir ses
// deux clés laissait toute la suite verte (mesuré à la relecture d'I18N-28) —
// et l'écran filtré affichait alors « 12 entrée(s) » en taisant le « sur 480 »,
// c'est-à-dire le compteur menteur que LOT-18 avait justement fermé. Épingler
// la correspondance condition ↔ clé, pas la présence des deux clés : c'est la
// seule forme qui attrape une interversion.
assertOk(
  /return n === total\s*\?\s*t\("logs\.count\.all"[\s\S]{0,80}?:\s*t\("logs\.count\.filtered"/.test(
    logViewStr,
  ),
  "U2-3: sans troncature, n === total annonce le total seul et le cas filtré annonce « sur {total} » — jamais l'inverse",
);

// ── U3 — the copies: what is on screen, and a visible failure ─
assertOk(
  /writeClipboard\(formatLogText\(logs\), logs\.length/.test(logView),
  "U3-1: « Copier » sérialise la liste AFFICHÉE (logs), jamais le tampon entier",
);
assertOk(
  /writeClipboard\(formatLogText\(allLogs\), allLogs\.length/.test(logView),
  "U3-2: « Tout copier » est l'action distincte et libellée qui prend le tampon complet",
);
assertOk(
  /await withTimeout\(\s*navigator\.clipboard\.writeText\(text\),\s*CLIPBOARD_TIMEOUT_MS,[\s\S]{0,80}?\);[\s\S]{0,300}?catch \(e\) \{[\s\S]{0,120}?toast\("error"/.test(
    logViewStr,
  ),
  "U3-3: un échec du presse-papier se voit — toast d'erreur autour de writeText",
);
assertOk(
  /disabled=\{logs\.length === 0\}/.test(logView),
  "U3-4: rien à afficher → le bouton « Copier » est désactivé",
);
assertOk(
  /await withTimeout\(\s*navigator\.clipboard\.writeText\(text\),\s*CLIPBOARD_TIMEOUT_MS,[\s\S]{0,80}?\);[\s\S]{0,200}?appState\.toast\("success"[\s\S]{0,400}?catch \(e\) \{[\s\S]{0,160}?appState\.toast\("error"/.test(
    logViewStr,
  ),
  "U3-5: le toast de succès vient APRÈS l'await du presse-papier — annoncer « copié » avant d'avoir écrit est un mensonge",
);

// ── U4 — the clear goes through ConfirmButton, never directly ─
assertOk(
  /onconfirm=\{[^}]*appState\.logs = \[\]/.test(logView),
  "U4-1: l'effacement passe par le onconfirm de ConfirmButton",
);
assertOk(
  !/onclick=\{[^}]*appState\.logs = \[\]/.test(logView),
  "U4-2: aucun onclick n'efface directement — la confirmation est obligatoire",
);

// ── U5 — the filter/search state lives in the module, survives navigation ─
assertOk(
  /bind:value=\{logsState\.search\}/.test(logView),
  "U5-1: la recherche est liée à logsState.search",
);
assertOk(
  /logsState\.level = item\.id/.test(logView),
  "U5-2: les puces de niveau écrivent dans logsState.level",
);
assertOk(
  /bind:checked=\{logsState\.autoScroll\}/.test(logView),
  "U5-3: le défilement auto est lié à logsState.autoScroll",
);
assertOk(
  /from "\.\.\/lib\/log-filter"/.test(logViewStr),
  "U5-4: la vue importe la logique pure de log-filter, elle ne la réinvente pas",
);
assertOk(
  /level = \$state<LogLevelFilter>\("all"\)/.test(logsStateStr),
  "U5-5: logs-state porte le niveau en $state",
);
assertOk(/search = \$state\(""\)/.test(logsStateStr), "U5-6: logs-state porte la recherche en $state");
assertOk(
  /autoScroll = \$state\(true\)/.test(logsStateStr),
  "U5-7: logs-state porte le défilement auto en $state",
);

// ── U6 — auto-scroll follows the FILTERED list, and only from the bottom ─
// One effect, dependent on the rendered list itself. Three parts pinned:
// the logs.length read (the real dependency — reassigning a tick with the
// same value notifies nothing, which is exactly the bug LOT-18-fix01
// closed), the nearBottom re-arm when nothing overflows (no scroll event
// fires then, so nothing else can re-arm it), and the conditional jump.
assertOk(
  /\$effect\(\(\) => \{[\s\S]{0,80}?logs\.length;[\s\S]{0,160}?container\.scrollHeight - container\.clientHeight <= 0\) nearBottom = true;[\s\S]{0,160}?if \(logsState\.autoScroll && nearBottom\) container\.scrollTop = container\.scrollHeight;/.test(
    logView,
  ),
  "U6-1: un seul effet dépend de la liste rendue — lecture de logs.length, réarmement de nearBottom sans débordement, défilement conditionnel",
);
assertOk(
  /if \(!container\) return;[\s\S]{0,200}?if \(logsState\.autoScroll && nearBottom\) container\.scrollTop = container\.scrollHeight;/.test(
    logView,
  ),
  "U6-2: la vue ne saute en bas que si l'utilisateur y est déjà",
);
assertOk(/onscroll=\{onScroll\}/.test(logView), "U6-3: la position de lecture est suivie par onscroll");

// ── U7 — the numeric plugin-log codes are translated once, at ingestion ─
// Without levelName at the attachLogger call site, every real entry
// carries "3" and the whole level vocabulary (rows, chips, colours) is
// dead on real data — exactly what a live run showed before the fix.
const mainStr = stripComments(
  readFileSync("src/main.ts", { encoding: "utf8" }),
);
assertOk(
  /addLog\(\s*levelName\(String\(level\)\),\s*resolveI18nLogMessage\(message, isKnownI18nKey, \(key, args\) => t\(key, args\)\),\s*\)/.test(mainStr),
  "U7: attachLogger traduit les codes numériques via levelName puis résout le message avant addLog",
);
// U7-2/U7-3: ingestion is not the only door — a raw code that reached
// addLog through a future caller must not be DISPLAYED or COPIED either.
// Both read levelName, like levelBucket and levelColor already do.
assertOk(
  /levelName\(entry\.level\)\.slice\(0, 5\)/.test(logView),
  "U7-2: la colonne niveau de la ligne passe par levelName — un code brut ne s'affiche jamais",
);
const logFilterSrc = stripCommentsAndStrings(
  readFileSync("src/lib/log-filter.ts", { encoding: "utf8" }),
);
assertOk(
  /const level = levelName\(entry\.level\);/.test(logFilterSrc),
  "U7-3: le format de copie passe par levelName — un code brut ne s'écrit jamais dans le presse-papier",
);

// ── U8 — the behaviour guards LOT-18 left open ─
// One surviving mutation each: the empty state without isFilterActive,
// a frozen row colour, chip counts hardwired to zero, a nearBottom
// threshold that no longer means "near the bottom", all stayed green.
assertOk(
  /\{#if isFilterActive\(logsState\.level, logsState\.search\)\}/.test(logView),
  "U8-1: l'état vide distingue « rien ne correspond au filtre » de « tampon vide » via isFilterActive",
);
// The row colour lives inside a class="…" attribute — a string the
// strings-stripped copy removes, so this one reads the stripComments copy.
assertOk(
  /class="[^"]*\{levelColor\(entry\.level\)\}[^"]*"/.test(logViewStr),
  "U8-2: la couleur de la ligne lit levelColor(entry.level) — jamais figée",
);
assertOk(
  /\{countFor\(item\.id\)\}/.test(logView),
  "U8-3: les puces affichent countFor(item.id), pas un compte figé",
);
assertOk(
  /function countFor\([\s\S]{0,120}?return filterLogs\(allLogs, id, logsState\.search\)\.length;/.test(logView),
  "U8-4: countFor compte sur le tampon entier, sous la recherche courante",
);
assertOk(
  /container\.scrollHeight - container\.scrollTop - container\.clientHeight < 48/.test(logView),
  "U8-5: nearBottom = à moins de 48 px du bas — un seuil plus large ferait sauter la vue pendant la lecture",
);

console.log(
  "  ✓ tripwires structurels journaux (LOT-18 : U1 filtrer-avant-tronquer, U2 compteur, U3 copies, U4 confirmation, U5 état, U6 défilement, U7 niveaux, U8 gardes de comportement)",
);

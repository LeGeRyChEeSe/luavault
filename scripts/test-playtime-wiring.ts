/**
 * Structural guard rails for local playtime (LOT-13).
 *
 * Same contract as test-news-wiring.ts: these are NOT behaviour tests. They
 * read the sources and pin the shape of wirings no test in this stack can
 * execute — the playtime line dropped from the game card, the stats section
 * deleted, the sort mode removed from the selector. The pure behaviour
 * (formatPlaytime's three states, sortStatuses' unknowns-last rule) is
 * tested directly in test-virtual-scroll.ts; the guards here pin the WIRING
 * expressions that carry it into the views.
 *
 * Every pattern runs on a STRIPPED copy of the source (see
 * stripCommentsAndStrings in test-dlc-wiring.ts): comments and string
 * literals are removed first, so a commented-out call or a comment quoting
 * the markup never satisfies a guard. What the stripping erases (the French
 * labels, the tooltip texts) is deliberately out of scope — the identifiers
 * carry the wiring. The one exception is P4: the <option>'s value attribute
 * IS a string, and it is the only place the sort mode reaches the user —
 * so P4 runs on stripComments (same comment branches, strings KEPT). A
 * commented-out option is still dead markup, and stripComments removes
 * it too: preserving strings and removing comments are two different
 * needs, conflating them once is what left P4 blind to HTML comments.
 *
 * Deliberately loose on shape: each assertion goes red on a
 * behaviour-breaking revert, not on a rename or a reorder. A guard that
 * cries wolf is a guard someone disables.
 *
 * Runs from the project root (validate.ps1 and `npm run test:ts` both reach
 * it through test-virtual-scroll.ts).
 */

// @ts-expect-error — `node:fs` has no types here: @types/node is not a
// project dependency. See the identical note in test-library-view.ts.
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
const gameCardRaw = readFileSync("src/views/GameCard.svelte", { encoding: "utf8" });
const gameCard = stripCommentsAndStrings(gameCardRaw);
const spotlight = stripCommentsAndStrings(
  readFileSync("src/components/GameSpotlight.svelte", { encoding: "utf8" }),
);
const statsView = stripCommentsAndStrings(
  readFileSync("src/views/StatsView.svelte", { encoding: "utf8" }),
);
const libraryViewRaw = readFileSync("src/views/LibraryView.svelte", { encoding: "utf8" });
// P4 must keep the string literals (the option's value) but refuse
// commented-out markup — stripComments, not the raw source.
const libraryView = stripComments(libraryViewRaw);
const libraryState = stripCommentsAndStrings(
  readFileSync("src/lib/library-state.svelte.ts", { encoding: "utf8" }),
);

// ── P1 — the game card renders the playtime AND the last session ──
// Pins the WIRING expression: formatPlaytime called with the two GameStatus
// fields, and the session date through the Unix-seconds formatter (feeding
// it to formatDate or a bare new Date brings the 1970 bug back, LOT-12).
// Correct: restyling the line, moving it inside the card.
// Broken: removing the line, or rendering one field without the other.
// The trailing comma is optional on purpose: a multi-line call with a
// trailing comma is the same wiring, and a guard that cries wolf on a
// reformat is a guard someone disables.
assertOk(
  /formatPlaytime\(\s*status\.playtime_minutes\s*,\s*status\.last_played\s*,?\s*\)/.test(gameCard),
  "P1-1: GameCard rend le temps de jeu via formatPlaytime(status.playtime_minutes, status.last_played)",
);
assertOk(
  /formatUnixDate\(\s*status\.last_played\s*\)/.test(gameCard),
  "P1-2: GameCard rend la dernière session via formatUnixDate (secondes Unix, pas formatDate)",
);
// The line carries its origin story on hover: the div it lives under must
// own the data-tip, IN ITS OWN OPENING TAG — `[^<>]*` on both sides of the
// attribute allows neither `<` nor `>`, so the attribute can only sit in
// that one tag. Then no closing </div> may come between that tag and the
// call, or a tooltip on a preceding SIBLING div would satisfy the guard
// while the playtime line itself has none.
assertOk(
  /<div[^<>]*data-tip=[^<>]*>(?:(?!<\/div>)[\s\S]){0,250}?formatPlaytime\(/.test(gameCard),
  "P1-3: la ligne temps de jeu de GameCard porte un data-tip expliquant la source locale",
);

// ── P2 — the big card shows the same data ────────────────────────
// Correct: restyling the panel.
// Broken: removing the panel from the spotlight.
assertOk(
  /formatPlaytime\(\s*status\.playtime_minutes\s*,\s*status\.last_played\s*,?\s*\)/.test(spotlight),
  "P2-1: GameSpotlight rend le temps de jeu via formatPlaytime(status.playtime_minutes, status.last_played)",
);
assertOk(
  /formatUnixDate\(\s*status\.last_played\s*\)/.test(spotlight),
  "P2-2: GameSpotlight rend la dernière session via formatUnixDate",
);
assertOk(
  /<div[^<>]*data-tip=[^<>]*>(?:(?!<\/div>)[\s\S]){0,250}?formatPlaytime\(/.test(spotlight),
  "P2-3: le panneau temps de jeu de GameSpotlight porte un data-tip expliquant la source locale",
);

// ── P3 — the stats view aggregates the playtime ──────────────────
// Correct: restyling the section, moving it in the page.
// Broken: deleting the section, rendering the total as raw minutes, or
// dropping the unknown-games count.
assertOk(
  /formatTotalPlaytime\(\s*stats\.playtime_total_minutes\s*,\s*stats\.playtime_unknown\s*,\s*stats\.total\s*,?\s*\)/.test(statsView),
  "P3-1: StatsView rend le temps total via formatTotalPlaytime(total, inconnus, total de jeux) — quand aucun jeu n'a de donnée, le total est « temps inconnu », pas « jamais joué »",
);
assertOk(
  /stats\.most_played[\s\S]{0,200}?formatPlaytime\([\s\S]{0,60}?most_played/.test(statsView),
  "P3-2: StatsView rend le jeu le plus joué (stats.most_played passé à formatPlaytime)",
);
assertOk(
  /stats\.playtime_unknown/.test(statsView),
  "P3-3: StatsView affiche combien de jeux sont sans donnée (stats.playtime_unknown)",
);
assertOk(
  /<div[^<>]*data-tip=[^<>]*>(?:(?!<\/div>)[\s\S]){0,400}?formatTotalPlaytime\(/.test(statsView),
  "P3-4: la section temps de jeu de StatsView porte un data-tip expliquant la source locale",
);

// ── P4 — the library selector offers the sort mode ───────────────
// On the stripComments copy: the option's value is a string literal and
// IS the wiring, so strings must survive — but a commented-out option is
// dead markup, and it used to keep this guard green while the user could
// no longer pick the sort (pitfall 32, verbatim).
// Correct: moving the option inside the select, relabelling it.
// Broken: removing the option — or commenting it out.
assertOk(
  libraryView.includes('<option value="playtime">'),
  'P4: LibraryView propose le tri « temps de jeu » (<option value="playtime">, pas en commentaire)',
);

// ── P5 — LibraryView's import source still exports sortStatuses ──
// LibraryView imports sortStatuses from library-state.svelte; the pure
// logic may live elsewhere (library-sort.ts, for the tsx runner) but the
// re-export is the wiring. Correct: inlining the function back, renaming
// the module it comes from. Broken: dropping the re-export.
assertOk(
  /export\s*\{\s*sortStatuses\s*\}\s*from/.test(libraryState)
    || /export function sortStatuses/.test(libraryState),
  "P5: library-state.svelte.ts expose toujours sortStatuses (ré-export ou définition)",
);

console.log(
  "  ✓ tripwires structurels Temps de jeu (GameCard/GameSpotlight/StatsView/LibraryView : P1–P5)",
);

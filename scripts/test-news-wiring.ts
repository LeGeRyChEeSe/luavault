/**
 * Structural guard rails for the aggregated changelog feed (LOT-12).
 *
 * Same contract as test-dlc-wiring.ts: these are NOT behaviour tests. They
 * read the sources and pin the shape of wirings no test in this stack can
 * execute — the tab removed from the sidebar, the offline gate dropped so
 * opening the tab fires forty requests, the patch filter or the
 * browser-open button deleted.
 *
 * Every pattern runs on a STRIPPED copy of the source (see
 * stripCommentsAndStrings in test-dlc-wiring.ts): comments and string
 * literals are removed first, so a commented-out call or a comment quoting
 * the markup never satisfies a guard. What the stripping erases (French
 * labels, the command name string) is deliberately out of scope — the
 * identifiers carry the wiring.
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
import { readFileSync as readFileSyncRaw, readdirSync as readdirSyncRaw, statSync as statSyncRaw } from "node:fs";
import { stripCommentsAndStrings } from "./test-dlc-wiring";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;
const readdirSync = readdirSyncRaw as (path: string) => string[];
const statSync = statSyncRaw as (path: string) => { isDirectory(): boolean };

function assertOk(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

// ── Read and strip the sources ────────────────────────────────
const newsRaw = readFileSync("src/views/NewsView.svelte", { encoding: "utf8" });
const news = stripCommentsAndStrings(newsRaw);
const app = stripCommentsAndStrings(readFileSync("src/App.svelte", { encoding: "utf8" }));
const api = stripCommentsAndStrings(readFileSync("src/lib/api.ts", { encoding: "utf8" }));
const spotlight = stripCommentsAndStrings(
  readFileSync("src/components/GameSpotlight.svelte", { encoding: "utf8" }),
);

// ── N1 — the typed wrapper exists and reaches the command ─────
// Correct: renaming the command string (stripped anyway), reformatting.
// Broken: removing the wrapper or dropping the typed invoke.
{
  const start = api.indexOf("export const changelogFeed");
  const next = api.indexOf("export const", start + 1);
  const body = start >= 0 ? api.slice(start, next > start ? next : undefined) : "";
  assertOk(
    start >= 0 && /invoke<FeedReport>/.test(body),
    "N1: api.ts doit exposer changelogFeed via un invoke typé (invoke<FeedReport>)",
  );
}

// ── N2 — the tab is wired in the shell ────────────────────────
// Correct: restyling the view, moving the import.
// Broken: removing the tab — import or render — from App.svelte.
assertOk(
  (app.match(/NewsView/g) ?? []).length >= 2,
  "N2: App.svelte doit importer ET rendre NewsView (l'onglet Nouveautés)",
);

// ── N3 — offline: the first load reads the cache, never the network ──
// Pins the WIRING, not the co-presence of two identifiers: the call inside
// onMount must receive an argument derived from appState.online. Replacing
// `refresh(false, !appState.online)` with `refresh(false, false)` sends
// forty requests on an offline open — appState.online still appears on the
// retry button, so a presence check stayed green on exactly that mutation.
// Correct: renaming the loader, rewriting the derivation
// (`appState.online === false`).
// Broken: the initial call no longer derives any argument from
// appState.online.
assertOk(
  /onMount\([\s\S]{0,400}?\w+\s*\([^)]*appState\.online/.test(news),
  "N3: l'appel de chargement initial (onMount) passe un argument dérivé d'appState.online",
);

// ── N4 — the patch filter and the browser open survive ────────
// Pins the FILTERING itself: a conditional `.filter(...is_patch_notes...)`.
// Deleting the filter left the old guard green — is_patch_notes survived in
// the « Correctif » badge and patchesOnly in the toggle. And the old guard
// cried wolf on a plain rename of patchesOnly, which changed nothing.
// Correct: renaming the toggle's state, restyling the toggle, reordering
// the ternary's operands' surroundings.
// Broken: removing the filter (the toggle does nothing), or making it
// unconditional (the feed shows only patch notes forever).
assertOk(
  /\w+\s*\?\s*[\w$.]+\s*\.\s*filter\([\s\S]{0,80}?is_patch_notes/.test(news),
  "N4-1: le filtre « correctifs seulement » s'applique via un .filter(...is_patch_notes...) conditionnel",
);
assertOk(/openUrl\(/.test(news), "N4-2: l'annonce s'ouvre dans le navigateur (openUrl)");

// ── N5 — changelog dates render through the Unix-seconds formatter ──
// Correct: restyling the date span.
// Broken: feeding Unix seconds to formatDate (ISO) or a local new Date —
// the 1970 bug returns.
assertOk(/formatUnixDate\(/.test(news), "N5-1: la vue rend les dates par formatUnixDate");
assertOk(
  /formatUnixDate\([^)]*date/.test(spotlight),
  "N5-2: GameSpotlight rend la date du changelog par formatUnixDate",
);

// ── N6 — third-party text is rendered as text, never markup ───
// Scans every .svelte source under src/: {@html} must appear nowhere. The
// feed shows Steam's own words; one {@html} would turn them into active
// markup inside the app. This reads the RAW sources on purpose — {@html}
// is markup, not a comment or a string, and the guard must see the file
// exactly as the compiler does.
{
  const offenders: string[] = [];
  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir)) {
      const path = `${dir}/${entry}`;
      if (statSync(path).isDirectory()) {
        walk(path);
      } else if (entry.endsWith(".svelte") && readFileSync(path, { encoding: "utf8" }).includes("{@html")) {
        offenders.push(path);
      }
    }
  };
  walk("src");
  assertOk(
    offenders.length === 0,
    `N6: aucun {@html} — le texte tiers est rendu comme du texte (${offenders.join(", ")})`,
  );
}

console.log(
  "  ✓ tripwires structurels Nouveautés (App/api/NewsView/GameSpotlight : N1–N6)",
);

/**
 * Frontend unit tests — run with: npx --yes tsx scripts/test-virtual-scroll.ts
 * Replayed by scripts/validate.ps1 (step "Frontend: tests unitaires") and by
 * `npm run test:ts`.
 *
 * Deliberately does NOT import node:assert — @types/node is not a project
 * dependency and this file is typed by svelte-check (tsconfig.json includes
 * scripts/**). An uncaught throw makes tsx exit 1, which is what Run-Step
 * detects through $LASTEXITCODE.
 *
 * The structural tripwires for LibraryView.svelte's wiring live in
 * test-library-view.ts, the shared source-stripping helpers in
 * test-dlc-wiring.ts, the GameSpotlight guards in test-spotlight-wiring.ts,
 * the offline-mode guards in
 * test-offline-wiring.ts, the changelog-feed guards (LOT-12) in
 * test-news-wiring.ts, the local-playtime guards (LOT-13) in
 * test-playtime-wiring.ts, the artwork-cache guards (LOT-14) in
 * test-artwork-wiring.ts, the targeted-repair guards (LOT-15) in
 * test-repair-wiring.ts, the multi-select guards (LOT-16) in
 * test-selection-wiring.ts, and the log-view guards (LOT-18) in
 * test-logs-wiring.ts; all are imported at the bottom so this single
 * entry point runs everything.
 */
// FIRST, and it has to stay first: it installs the `$state` global that every
// `.svelte.ts` module needs at import time. See scripts/rune-shim.ts for the
// round this cost.
import "./rune-shim";
import { virtualWindow } from "../src/lib/virtual-scroll";
import type { GameStatus } from "../src/lib/api";

/** Minimal stand-in for node:assert/strict — same call signature, throws. */
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

const ROW = 170; // px — card height + gap
const VIEWPORT = 800; // px

function label(name: string) {
  console.log(`  ✓ ${name}`);
}

// ── Empty list ──────────────────────────────────────────────
{
  const w = virtualWindow(0, VIEWPORT, ROW, 0, 3);
  assert.equal(w.startIndex, 0);
  assert.equal(w.endIndex, 0);
  assert.equal(w.totalHeight, 0);
  label("liste vide → rien à rendre");
}

// ── First screen (30 items, 3 cols → 10 rows) ──────────────
{
  const w = virtualWindow(0, VIEWPORT, ROW, 30, 3, 3);
  assert.equal(w.startRow, 0, "starts at row 0");
  assert.equal(w.startIndex, 0);
  // visible rows ≈ ceil(800/170) = 5, + 3 overscan below = 8
  assert.ok(w.endIndex <= 30, "does not exceed total");
  assert.ok(w.endIndex >= 15, "renders at least the visible rows");
  assert.equal(w.offsetTop, 0, "no top spacer at the top");
  assert.equal(w.totalHeight, 10 * ROW);
  label("premier écran (30 entrées, 3 col)");
}

// ── Middle of the list (500 items, 3 cols) ─────────────────
{
  const total = 500;
  const totalRows = Math.ceil(total / 3); // 167
  const scrollMid = Math.floor(totalRows / 2) * ROW;
  const w = virtualWindow(scrollMid, VIEWPORT, ROW, total, 3, 3);
  assert.ok(w.startRow > 0, "not at the top");
  assert.ok(w.endRow < totalRows, "not at the bottom");
  assert.ok(w.startIndex < 250 && w.endIndex > 250, "window straddles the midpoint");
  assert.ok(w.offsetTop > 0, "top spacer present");
  assert.ok(w.offsetBottom > 0, "bottom spacer present");
  assert.equal(w.totalHeight, totalRows * ROW);
  label("milieu de liste (500 entrées, 3 col)");
}

// ── End of the list ────────────────────────────────────────
{
  const total = 500;
  const totalRows = Math.ceil(total / 3);
  const scrollEnd = totalRows * ROW; // past the bottom
  const w = virtualWindow(scrollEnd, VIEWPORT, ROW, total, 3, 3);
  assert.equal(w.endRow, totalRows, "clamped to last row");
  assert.equal(w.endIndex, total, "last item included");
  assert.equal(w.offsetBottom, 0, "no bottom spacer at the end");
  label("fin de liste (500 entrées, 3 col)");
}

// ── Column change: 1 column ────────────────────────────────
{
  const total = 500;
  const w = virtualWindow(0, VIEWPORT, ROW, total, 1, 3);
  const totalRows = 500;
  assert.equal(w.totalHeight, totalRows * ROW);
  // visible ≈ 5 rows + 3 overscan = 8 items max
  assert.ok(w.endIndex <= 9, `1-col renders few items (got ${w.endIndex})`);
  label("changement colonnes → 1 col");
}

// ── Column change: 2 columns ───────────────────────────────
{
  const total = 500;
  const w = virtualWindow(0, VIEWPORT, ROW, total, 2, 3);
  const totalRows = 250;
  assert.equal(w.totalHeight, totalRows * ROW);
  assert.ok(w.endIndex <= 18, `2-col renders few items (got ${w.endIndex})`);
  label("changement colonnes → 2 col");
}

// ── Small list below threshold still works ─────────────────
{
  const w = virtualWindow(0, VIEWPORT, ROW, 5, 3, 3);
  assert.equal(w.startIndex, 0);
  assert.equal(w.endIndex, 5, "all 5 items rendered");
  assert.equal(w.totalHeight, 2 * ROW, "2 rows");
  label("petite liste (5 entrées)");
}

// ── Invariants over a sweep, not four hand-picked values ───
// `rendered` is measured from the indices the window would actually mount —
// not from the same (endRow - startRow) expression offsetTop and offsetBottom
// are derived from, which made the old assertion true by construction even on
// an inverted slice. The two bound assertions are the ones an unclamped
// startRow violates: they catch the overshoot when scrollTop runs past the
// content (list shrunk by a search, window widened so columns ate rows).
{
  const total = 500;
  for (const cols of [1, 2, 3]) {
    for (const overscan of [0, 3, 5]) {
      for (let scroll = 0; scroll <= 200_000; scroll += 100) {
        const w = virtualWindow(scroll, VIEWPORT, ROW, total, cols, overscan);
        assert.ok(w.startIndex <= w.endIndex, `tranche inversée à scroll=${scroll} cols=${cols} overscan=${overscan}`);
        assert.ok(w.offsetTop <= w.totalHeight, `offsetTop hors bornes à scroll=${scroll} cols=${cols} overscan=${overscan}`);
        // The equality alone proves nothing: on an inverted slice
        // `renderedItems` goes negative and the sum lands back on
        // totalHeight — which is exactly how the previous version of this
        // test walked straight past a window it was meant to catch. Guard it
        // on a non-empty slice, and assert the guard is never taken, so the
        // two statements cannot cover for each other.
        assert.ok(
          w.startIndex < w.endIndex || w.totalHeight === 0,
          `tranche vide à scroll=${scroll} cols=${cols} overscan=${overscan}`,
        );
        if (w.startIndex < w.endIndex) {
          const renderedRows = Math.ceil((w.endIndex - w.startIndex) / cols);
          assert.equal(
            w.offsetTop + renderedRows * ROW + w.offsetBottom,
            w.totalHeight,
            `height invariant at scroll=${scroll} cols=${cols} overscan=${overscan}`,
          );
        }
      }
    }
  }
  label("invariants en balayage (0 → 200 000 px, 1/2/3 col, overscan 0/3/5)");
}

// ── formatUnixDate — Unix seconds, never milliseconds (LOT-12) ──
// Steam reports changelog dates in Unix seconds, and `new Date` interprets
// a number as milliseconds. 1738000000 s is 2025-01-27T16:26:40Z — fed raw
// to `new Date` it lands in January 1970, which is exactly what every
// changelog date showed before the fix. Pinned under TZ=UTC so the expected
// day holds on any machine.
// @ts-expect-error — process.env has no types here: @types/node is not a
// project dependency. Same suppression as above.
const savedTzUnix = process.env.TZ;
// @ts-expect-error — process.env assignment, same suppression as above.
process.env.TZ = "UTC";
const { formatUnixDate } = await import("../src/lib/format");
// I18N-07 : fixer la locale pour que les dates rendues soient prévisibles.
await import("../src/lib/i18n.svelte").then(({ i18n }) => i18n.hydrate("fr"));
{
  const rendered = formatUnixDate(1738000000);
  assert.ok(!rendered.includes("1970"), `LOT-12: 1738000000 ne doit plus rendre une date de 1970 (rendu : ${rendered})`);
  assert.equal(rendered, "27 janv. 2025", "LOT-12: 1738000000 s = 27 janvier 2025");
  assert.equal(formatUnixDate(null), "—", "null → —");
  assert.equal(formatUnixDate(undefined), "—", "undefined → —");
  assert.equal(formatUnixDate(0), "—", "0 (date absente) → —");
  label("formatUnixDate (secondes Unix, pas millisecondes — TZ UTC)");
}
// Restore the previous TZ, same pattern as D7 above.
if (savedTzUnix === undefined) {
  // @ts-expect-error — delete on process.env, same suppression as above.
  delete process.env.TZ;
} else {
  // @ts-expect-error — process.env assignment, same suppression as above.
  process.env.TZ = savedTzUnix;
}

// ── formatPlaytime — LOT-13: jamais joué ≠ temps inconnu ──────
// Pure minutes arithmetic, no timezone in play. The three states must stay
// distinct: a measured time, "jamais joué" (a known zero with no session)
// and "temps inconnu" (no readable data — never rendered as a zero).
// I18N-08 : fixer la locale pour que les chaînes rendues soient prévisibles.
await import("../src/lib/i18n.svelte").then(({ i18n }) => i18n.hydrate("fr"));
{
  const { formatPlaytime } = await import("../src/lib/format");
  assert.equal(formatPlaytime(217), "3 h 37", "217 minutes = 3 h 37");
  assert.equal(formatPlaytime(45), "45 min", "moins d'une heure reste en minutes");
  assert.equal(formatPlaytime(120), "2 h", "un compte rond n'affiche pas 00");
  assert.equal(formatPlaytime(61), "1 h 01", "les minutes repassent sur deux chiffres");
  assert.equal(formatPlaytime(0), "jamais joué", "zéro sans session = jamais joué, pas 0 min");
  assert.equal(formatPlaytime(null), "temps inconnu", "null = on ne sait pas");
  assert.equal(formatPlaytime(undefined), "temps inconnu", "undefined = on ne sait pas");
  assert.equal(
    formatPlaytime(0, 1712725190),
    "< 1 min",
    "zéro minute mais une session réelle : Steam n'a pas consigné les minutes",
  );
  assert.equal(
    formatPlaytime(null, 1712725190),
    "temps inconnu",
    "des sessions sans minutes restent un temps inconnu",
  );
  label("formatPlaytime (3 h 37, 45 min, jamais joué, temps inconnu, < 1 min)");
}

// ── formatTotalPlaytime — LOT-13-fix01: un total tout-inconnu ≠ « jamais joué » ──
// The backend sums only KNOWN playtimes, so a zero total must not always
// read "jamais joué": when no game has any data, the total is unknown too.
// I18N-08 : fixer la locale pour que les chaînes rendues soient prévisibles.
await import("../src/lib/i18n.svelte").then(({ i18n }) => i18n.hydrate("fr"));
{
  const { formatTotalPlaytime } = await import("../src/lib/format");
  assert.equal(
    formatTotalPlaytime(260, 1, 4),
    "4 h 20",
    "une somme connue s'affiche normalement, jeux sans donnée inclus",
  );
  assert.equal(
    formatTotalPlaytime(0, 4, 4),
    "temps inconnu",
    "aucun jeu avec donnée → total inconnu, pas « jamais joué »",
  );
  assert.equal(
    formatTotalPlaytime(0, 1, 3),
    "jamais joué",
    "des zéros CONNUS (jeux jamais joués) restent « jamais joué »",
  );
  assert.equal(
    formatTotalPlaytime(0, 0, 2),
    "jamais joué",
    "total réellement nul, tout le monde connu",
  );
  label("formatTotalPlaytime (somme connue, tout-inconnu ≠ jamais joué)");
}

// ── sortStatuses "playtime" — LOT-13: unknowns last, ties stable ─
// The pure comparator lives in library-sort.ts so this runner can import it
// (library-state.svelte.ts carries runes). The rule under test: descending
// on KNOWN minutes, unknowns never lead, and ties fall back to name then
// app_id so two games never swap places on a refresh.
const { sortStatuses } = await import("../src/lib/library-sort");

/** A full GameStatus with every field a comparator might read. */
function makeStatus(
  overrides: Partial<GameStatus> & Pick<GameStatus, "app_id" | "name">,
): GameStatus {
  return {
    icon: null,
    updated_at: null,
    added_at: null,
    in_library: true,
    lua_in_steam: true,
    fix_downloaded: false,
    hidden: false,
    tags: [],
    game: {
      app_id: overrides.app_id,
      known_to_steam: true,
      installed: true,
      fully_installed: true,
      install_dir: null,
      steam_name: null,
      state_flags: 4,
      size_on_disk: 0,
    },
    fix: {
      app_id: overrides.app_id,
      health: "not_installed",
      installed_at: null,
      game_dir: null,
      file_count: 0,
      missing: [],
      modified: [],
      has_backup: false,
      foreign: [],
    },
    stage: "ready",
    playtime_minutes: null,
    last_played: null,
    ...overrides,
  };
}

{
  const alpha = makeStatus({ app_id: "1", name: "Alpha", playtime_minutes: 100 });
  const beta = makeStatus({ app_id: "2", name: "Beta", playtime_minutes: 500 });
  const gamma = makeStatus({ app_id: "3", name: "Gamma", playtime_minutes: 200 });
  const delta = makeStatus({ app_id: "4", name: "Delta" }); // sans donnée
  const epsilon = makeStatus({ app_id: "5", name: "Epsilon" }); // sans donnée
  const jamais = makeStatus({ app_id: "6", name: "Jamais", playtime_minutes: 0 });

  const order = sortStatuses([alpha, beta, gamma, delta, epsilon, jamais], "playtime")
    .map((s) => s.app_id)
    .join(",");
  // Descending on known minutes; zero is a KNOWN value ("jamais joué") and
  // stays ahead of the unknowns; the unknowns close the list, alphabetical.
  assert.equal(
    order,
    "2,3,1,6,4,5",
    "les connus en décroissant, le zéro connu avant les inconnus, les inconnus en fin de liste",
  );
  label("tri temps de jeu : décroissant, jamais joué avant les inconnus, inconnus en fin");
}

{
  const zeta = makeStatus({ app_id: "10", name: "Zeta", playtime_minutes: 200 });
  const alpha2 = makeStatus({ app_id: "9", name: "Alpha", playtime_minutes: 200 });
  const forward = sortStatuses([zeta, alpha2], "playtime").map((s) => s.app_id).join(",");
  const backward = sortStatuses([alpha2, zeta], "playtime").map((s) => s.app_id).join(",");
  assert.equal(
    forward,
    backward,
    "à égalité de temps, l'ordre ne doit pas dépendre de l'ordre d'entrée",
  );
  assert.equal(forward, "9,10", "le départage passe par le nom puis l'app_id");
  label("tri temps de jeu : égalité départagée (nom puis app_id)");
}

// ── LOT-16: la sélection multiple — fonctions pures de selection.ts ──
// The state lives in library-state.svelte.ts (runes); the logic under test
// here is rune-free. Counts, purge and "select all" are what the bar's
// buttons and the passes agree on — mutating any of them (counting a
// fix_external, letting "select all" jump over the current filter, keeping
// an AppID that left the library) turns one of these red.
const {
  eligibleForSelectionAction,
  purgeSelection,
  selectAllVisible,
  deselectVisible,
  hasTag,
  withAddedTag,
  withRemovedTag,
  SELECTION_FIX_STAGES,
} = await import("../src/lib/selection");

{
  const fixables = [
    makeStatus({ app_id: "2", name: "Deux", stage: "fix_downloaded", fix_downloaded: true }),
    makeStatus({ app_id: "3", name: "Trois", stage: "fix_damaged", fix_downloaded: true }),
    makeStatus({ app_id: "4", name: "Quatre", stage: "fix_external" }),
    makeStatus({
      app_id: "5",
      name: "Cinq",
      stage: "installing",
      game: {
        app_id: "5",
        known_to_steam: true,
        installed: true,
        fully_installed: false,
        install_dir: null,
        steam_name: null,
        state_flags: 6,
        size_on_disk: 0,
      },
    }),
    makeStatus({ app_id: "6", name: "Six", stage: "ready" }),
    makeStatus({ app_id: "7", name: "Sept", stage: "fix_installed", fix_downloaded: true }),
    makeStatus({ app_id: "8", name: "Huit", stage: "ready", lua_in_steam: true }),
    makeStatus({ app_id: "9", name: "Neuf", stage: "fix_game_moved", fix_downloaded: true }),
    makeStatus({ app_id: "10", name: "Dix", stage: "needs_steam_install", lua_in_steam: true }),
  ];

  const fixes = eligibleForSelectionAction(fixables, "fixes").map((s) => s.app_id);
  assert.equal(
    fixes.join(","),
    "2,3,9",
    "le compte du bouton « patchs » est exactement les jeux que la passe traitera",
  );
  assert.ok(
    !fixes.includes("4"),
    "un fix_external ne compte jamais — aucune sauvegarde n'existe pour ces fichiers",
  );
  assert.ok(!fixes.includes("5"), "un téléchargement Steam en cours ne se patche pas");

  // Le compte change avec l'action : chaque bouton a le sien.
  const verify = eligibleForSelectionAction(fixables, "verify").map((s) => s.app_id);
  assert.equal(verify.join(","), "2,3,7,9", "vérifier : téléchargé + installé, rien d'autre");

  const copy = eligibleForSelectionAction([
    makeStatus({ app_id: "20", name: "CopyA", lua_in_steam: false }),
    makeStatus({ app_id: "21", name: "CopyB", lua_in_steam: true }),
    makeStatus({ app_id: "22", name: "CopyC", in_library: false, lua_in_steam: false }),
  ], "copy").map((s) => s.app_id);
  assert.equal(copy.join(","), "20", "copier : en bibliothèque et pas encore dans Steam");

  const hide = eligibleForSelectionAction([
    makeStatus({ app_id: "30", name: "HideA" }),
    makeStatus({ app_id: "31", name: "HideB", in_library: false }),
  ], "hide").map((s) => s.app_id);
  assert.equal(hide.join(","), "30", "masquer : seulement ce qui est en bibliothèque");

  const tagged = [
    makeStatus({ app_id: "40", name: "TagA", tags: ["coop"] }),
    makeStatus({ app_id: "41", name: "TagB", tags: ["COOP"] }),
    makeStatus({ app_id: "42", name: "TagC", tags: [] }),
  ];
  assert.equal(
    eligibleForSelectionAction(tagged, "add-tag", "Coop").map((s) => s.app_id).join(","),
    "42",
    "ajouter un tag ignore les jeux qui l'ont déjà (insensible à la casse)",
  );
  assert.equal(
    eligibleForSelectionAction(tagged, "remove-tag", "coop").map((s) => s.app_id).join(","),
    "40,41",
    "retirer un tag ne compte que les jeux qui le portent",
  );
  assert.equal(
    eligibleForSelectionAction(tagged, "add-tag", "  ").length,
    0,
    "pas de tag saisi → rien à ajouter",
  );

  // The set itself is pinned too — it mirrors INSTALLABLE_FIX_STAGES.
  assert.equal(SELECTION_FIX_STAGES.length, 3);
  assert.ok(!SELECTION_FIX_STAGES.includes("fix_external"), "fix_external n'entre jamais");
  label("comptes de sélection (3 patchables sur 10, vérifier/copier/masquer/tags)");
}

{
  // La purge : un AppID retiré de la bibliothèque disparaît de la sélection.
  const library = [
    makeStatus({ app_id: "1", name: "Reste" }),
    makeStatus({ app_id: "2", name: "Masqué", hidden: true }),
  ];
  assert.equal(
    purgeSelection(["1", "2", "3"], library).join(","),
    "1,2",
    "l'AppID qui a quitté la bibliothèque est purgé ; le masqué reste",
  );
  label("purge de la sélection (les sortants disparaissent, les masqués restent)");
}

{
  // « Tout sélectionner » s'arrête au filtre : shown = ce qui est visible,
  // jamais la bibliothèque entière. Un débordement ressusciterait le piège
  // des jeux masqués sous une autre forme.
  const shown = [
    makeStatus({ app_id: "1", name: "VisibleA" }),
    makeStatus({ app_id: "2", name: "VisibleB" }),
  ];
  const hiddenByFilter = makeStatus({ app_id: "3", name: "HorsFiltre" });
  const all = selectAllVisible(shown, []);
  assert.equal(all.join(","), "1,2", "tout sélectionner ne prend que le visible");
  assert.ok(!all.includes(hiddenByFilter.app_id), "un jeu hors filtre n'entre jamais");

  // Sélection existante hors filtre : elle reste (inerte), rien ne se perd.
  const withPrior = selectAllVisible(shown, ["3"]);
  assert.equal(withPrior.join(","), "3,1,2", "la sélection invisible existante survit, inerte");

  // Désélectionner ne retire que le visible.
  assert.equal(
    deselectVisible(shown, ["1", "2", "3"]).join(","),
    "3",
    "tout désélectionner porte sur le visible seulement",
  );
  label("tout sélectionner / désélectionner = borné au filtre courant");
}

{
  // Arithmétique des tags — la convention de normalize_tags côté Rust.
  assert.ok(hasTag(makeStatus({ app_id: "1", name: "T", tags: ["CoOp"] }), "coop"));
  assert.equal(withAddedTag(["a"], "A").join(","), "a", "pas de doublon insensible à la casse");
  assert.equal(withAddedTag(["a"], "  ").join(","), "a", "un tag vide n'ajoute rien");
  assert.equal(withAddedTag(["a"], " b ").join(","), "a,b", "le tag est rogné");
  assert.equal(withRemovedTag(["A", "b"], "a").join(","), "b", "retrait insensible à la casse");
  label("tags en sélection (casse, espaces, vide)");
}

// ── LOT-18: log view — the pure rules of src/lib/log-filter.ts ──
// The runes store (logs-state.svelte.ts) holds the level filter, the
// search and the auto-scroll switch; it cannot run under tsx, so the
// behaviour itself lives one module over and is tested here for real.
// The wiring (which list the view renders, which numbers the counter
// reads, which list each copy serialises) is pinned by
// test-logs-wiring.ts.
const {
  LOG_DISPLAY_LIMIT,
  I18N_LOG_SEPARATOR,
  decodeI18nLogMessage,
  displayLogs,
  filterLogs,
  formatLogLine,
  formatLogText,
  isFilterActive,
  levelBucket,
  levelColor,
  levelName,
  logMatches,
  normalizeText,
  resolveI18nLogMessage,
} = await import("../src/lib/log-filter");

interface TestLog {
  id: number;
  level: string;
  message: string;
  timestamp?: string;
}

function makeLog(id: number, level: string, message: string, timestamp?: string): TestLog {
  return { id, level, message, timestamp };
}

{
  // Rust adds metadata only after the readable French prefix. The parser is
  // total: malformed or legacy messages retain that prefix and never expose
  // raw JSON to the viewer.
  const structured = `cache horodaté${I18N_LOG_SEPARATOR}logs.cache.touch-failed${I18N_LOG_SEPARATOR}{"path":"C:\\\\cache","n":2}`;
  const decodedStructured = decodeI18nLogMessage(structured);
  assert.equal(decodedStructured.fallback, "cache horodaté");
  assert.equal(decodedStructured.payload?.key, "logs.cache.touch-failed");
  assert.equal(JSON.stringify(decodedStructured.payload?.args), '{"path":"C:\\\\cache","n":2}');
  assert.equal(
    resolveI18nLogMessage(
      structured,
      (key): key is "logs.cache.touch-failed" => key === "logs.cache.touch-failed",
      (_key, args) => `cache timestamp failed (${args.path}): ${args.n}`,
    ),
    "cache timestamp failed (C:\\cache): 2",
    "une clé connue est traduite avec ses arguments JSON typés",
  );

  const ordinary = "sync_from_steam: adopté 123456";
  const decodedOrdinary = decodeI18nLogMessage(ordinary);
  assert.equal(decodedOrdinary.fallback, ordinary);
  assert.equal(decodedOrdinary.payload, null);
  assert.equal(
    resolveI18nLogMessage(ordinary, (_key): _key is never => false, () => "impossible"),
    ordinary,
    "un ancien message sans charge reste strictement inchangé",
  );

  const malformed = `texte français${I18N_LOG_SEPARATOR}logs.cache.touch-failed${I18N_LOG_SEPARATOR}{pas du JSON}`;
  const decodedMalformed = decodeI18nLogMessage(malformed);
  assert.equal(decodedMalformed.fallback, "texte français");
  assert.equal(decodedMalformed.payload, null);
  assert.equal(
    resolveI18nLogMessage(malformed, (_key): _key is never => false, () => "impossible"),
    "texte français",
    "un JSON invalide ne laisse ni exception ni charge brute",
  );

  const unknown = `texte français${I18N_LOG_SEPARATOR}logs.removed-key${I18N_LOG_SEPARATOR}{}`;
  let translatedUnknown = false;
  assert.equal(
    resolveI18nLogMessage(
      unknown,
      (_key): _key is never => false,
      () => {
        translatedUnknown = true;
        throw new Error("t() would throw for an absent catalogue key");
      },
    ),
    "texte français",
    "une clé absente retombe sur le préfixe avant d'appeler t()",
  );
  assert.equal(translatedUnknown, false, "la garde de catalogue précède la traduction");

  assert.equal(
    resolveI18nLogMessage(
      `texte${I18N_LOG_SEPARATOR}logs.partial${I18N_LOG_SEPARATOR}{"present":1}`,
      (key): key is "logs.partial" => key === "logs.partial",
      (_key, args) => `valeur {missing} et ${args.present}`,
    ),
    "valeur {missing} et 1",
    "un argument absent conserve le placeholder, conformément à t(), sans exception",
  );
  label("décodage i18n des logs (charge, legacy, JSON invalide, clé inconnue, argument absent)");
}

{
  // Le vocabulaire des niveaux : ERROR / WARN / INFO / le reste.
  assert.equal(levelBucket("ERROR"), "error");
  assert.equal(levelBucket("error"), "error", "insensible à la casse");
  assert.equal(levelBucket("WARN"), "warn");
  assert.equal(levelBucket("warn"), "warn");
  assert.equal(levelBucket("INFO"), "info");
  assert.equal(levelBucket("TRACE"), "other", "TRACE tombe dans « le reste »");
  assert.equal(levelBucket("DEBUG"), "other", "DEBUG tombe dans « le reste »");
  assert.equal(levelBucket("n'importe quoi"), "other", "un niveau inconnu aussi");
  label("familles de niveaux (ERROR / WARN / INFO / le reste)");
}

{
  // plugin-log livre des codes numériques (Trace=1 … Error=5) : le même
  // vocabulaire doit les reconnaître, sinon le filtre par niveau est mort
  // sur les vraies entrées (constaté en conditions réelles au LOT-18).
  assert.equal(levelBucket("3"), "info", "le code 3 de plugin-log est INFO");
  assert.equal(levelBucket("4"), "warn", "le code 4 est WARN");
  assert.equal(levelBucket("5"), "error", "le code 5 est ERROR");
  assert.equal(levelBucket("1"), "other", "TRACE (1) tombe dans « le reste »");
  assert.equal(levelBucket("2"), "other", "DEBUG (2) tombe dans « le reste »");
  assert.equal(levelName("3"), "INFO");
  assert.equal(levelName("warn"), "WARN", "un nom passe en majuscules");
  assert.equal(levelName("inconnu"), "INCONNU", "l'inconnu n'est pas inventé");
  label("codes numériques de plugin-log reconnus (1–5)");
}

{
  // Le filtre par niveau ne laisse jamais passer un autre niveau.
  const entries = [
    makeLog(1, "ERROR", "panique"),
    makeLog(2, "warn", "prudence"),
    makeLog(3, "INFO", "tout va bien"),
    makeLog(4, "DEBUG", "détails"),
  ];
  assert.equal(filterLogs(entries, "error", "").map((e) => e.id).join(","), "1");
  assert.equal(filterLogs(entries, "warn", "").map((e) => e.id).join(","), "2");
  assert.equal(filterLogs(entries, "info", "").map((e) => e.id).join(","), "3");
  assert.equal(
    filterLogs(entries, "other", "").map((e) => e.id).join(","),
    "4",
    "DEBUG tombe dans « le reste »",
  );
  assert.equal(filterLogs(entries, "all", "").length, 4, "« Tous » ne perd rien");
  label("filtre par niveau (chaque famille seule, jamais un autre niveau)");
}

{
  // Recherche : insensible à la casse ET aux accents, sur le message.
  const entry = makeLog(1, "INFO", "Réseau Steam inaccessible (délai dépassé)");
  assert.ok(logMatches(entry, "reseau"), "minuscules sans accent trouvent « Réseau »");
  assert.ok(logMatches(entry, "RÉSEAU"), "majuscules accentuées aussi");
  assert.ok(logMatches(entry, "  délai  "), "la requête est rognée");
  assert.ok(!logMatches(entry, "réseaux"), "pas de correspondance approximative");
  assert.ok(logMatches(entry, ""), "requête vide = tout passe");
  assert.ok(logMatches(entry, "   "), "requête blanche = tout passe");
  // L'entrée est de niveau INFO : la requête « info » correspondrait si la
  // recherche portait aussi sur le niveau. « ERROR » ne le pouvait pas —
  // l'ancienne requête ne pouvait donc jamais échouer.
  assert.ok(!logMatches(entry, "info"), "la recherche porte sur le message, pas le niveau");
  assert.equal(normalizeText("Élévation"), "elevation", "NFD + diacritiques + minuscules");
  label("recherche plein texte (casse, accents, rognage, vide)");
}

{
  // Les deux se combinent.
  const entries = [
    makeLog(1, "ERROR", "Réseau coupé"),
    makeLog(2, "WARN", "Réseau instable"),
    makeLog(3, "ERROR", "Disque plein"),
  ];
  assert.equal(
    filterLogs(entries, "error", "reseau").map((e) => e.id).join(","),
    "1",
    "niveau ET texte à la fois",
  );
  label("filtre par niveau et recherche combinés");
}

{
  // Piège 1 du brief : filtrer AVANT de tronquer. Le tampon déborde la
  // limite d'affichage et l'entrée cherchée est ancienne — une pipeline
  // qui tronque d'abord la perdrait et jurerait qu'elle n'existe pas.
  const entries: TestLog[] = [makeLog(0, "ERROR", "demarrage: erreur d'origine")];
  for (let i = 1; i < 300; i++) entries.push(makeLog(i, "INFO", `bruit ${i}`));
  assert.equal(entries.length, 300, "300 entrées, la limite d'affichage en rend 200");
  const found = displayLogs(entries, "error", "origine", LOG_DISPLAY_LIMIT);
  assert.equal(found.length, 1, "l'entrée ancienne survit : le filtre passe avant la troncature");
  assert.equal(found[0].id, 0);
  const foundNoLevel = displayLogs(entries, "all", "origine", LOG_DISPLAY_LIMIT);
  assert.equal(foundNoLevel.length, 1, "pareil sans filtre de niveau");
  // Et la troncature s'applique bien — sur le résultat filtré.
  const tail = displayLogs(entries, "all", "", LOG_DISPLAY_LIMIT);
  assert.equal(tail.length, LOG_DISPLAY_LIMIT);
  assert.equal(tail[0].id, 100, "les 200 dernières du tampon, rien d'autre");
  assert.equal(displayLogs(entries, "all", "", 0).length, 0, "limite nulle → rien");
  label("filtrer avant tronquer (l'entrée ancienne reste trouvable)");
}

{
  // Le compteur dit la vérité : ce qui correspond, sur ce qui existe, et
  // la partie réellement affichée quand la limite s'en mêle. Depuis
  // I18N-28 la phrase vit dans les catalogues et la composition dans la
  // vue : ces assertions sont le seul juge de fidélité de ces quatre
  // valeurs — aucun cas du banc n'ouvre cet écran.
  const { fr } = await import("../src/lib/i18n/fr");
  const { en } = await import("../src/lib/i18n/en");
  assert.equal(fr["logs.count.all"], "{n} entrée(s)");
  assert.equal(
    fr["logs.count.filtered"],
    "{n} entrée(s) sur {total}",
    "le filtre réduit : on compte ce qui correspond, pas le tampon",
  );
  assert.equal(
    fr["logs.count.all-truncated"],
    "{n} entrée(s) — les {shown} dernières affichées",
    "la troncature se déclare",
  );
  assert.equal(
    fr["logs.count.filtered-truncated"],
    "{n} entrée(s) sur {total} — les {shown} dernières affichées",
  );
  for (const key of ["logs.count.filtered", "logs.count.filtered-truncated"] as const) {
    assert.ok(en[key].includes("{total}"), `${key} : l'anglais garde son {total}`);
  }
  for (const key of ["logs.count.all-truncated", "logs.count.filtered-truncated"] as const) {
    assert.ok(en[key].includes("{shown}"), `${key} : l'anglais garde son {shown}`);
  }
  label("compteur honnête (correspond / existe / affiché)");
}

{
  assert.equal(isFilterActive("all", ""), false);
  assert.equal(isFilterActive("all", "   "), false, "une recherche blanche n'est pas un filtre");
  assert.equal(isFilterActive("error", ""), true);
  assert.equal(isFilterActive("all", "x"), true);
  label("état du filtre (actif ou non)");
}

{
  // Le format de copie reflète l'écran : l'heure en slice(11, 19), le
  // niveau en majuscules, une entrée par ligne.
  assert.equal(
    formatLogLine({ level: "error", message: "boom", timestamp: "2026-08-03T14:03:22Z" }),
    "14:03:22 ERROR boom",
  );
  assert.equal(
    formatLogLine({ level: "info", message: "ok" }),
    "INFO ok",
    "sans horodatage, pas de colonne vide",
  );
  assert.equal(
    formatLogLine({ level: "3", message: "ok" }),
    "INFO ok",
    "un code numérique de plugin-log est traduit jusque dans la copie, jamais copié brut",
  );
  assert.equal(
    formatLogText([
      { level: "error", message: "boom", timestamp: "2026-08-03T14:03:22Z" },
      { level: "info", message: "ok" },
    ]),
    "14:03:22 ERROR boom\nINFO ok",
  );
  assert.equal(formatLogText([]), "");
  label("format de copie (la même heure que l'écran, une entrée par ligne)");
}

{
  // Le presse-papier peut rejeter OU ne jamais se régler : les deux
  // doivent devenir une erreur visible, jamais un silence. Le second cas
  // n'a pas été observé en conditions réelles — c'est une précaution, et
  // le test verrouille que le délai tient cet engagement.
  const { withTimeout, CLIPBOARD_TIMEOUT_MS } = await import("../src/lib/log-filter");
  assert.equal(CLIPBOARD_TIMEOUT_MS, 2000);
  const fast = await withTimeout(Promise.resolve("ok"), 50, "trop tard");
  assert.equal(fast, "ok", "une promesse rapide passe");
  let seen = "";
  try {
    await withTimeout(Promise.reject(new Error("boom")), 50, "trop tard");
  } catch (e) {
    seen = String((e as Error).message);
  }
  assert.equal(seen, "boom", "l'erreur d'origine est propagée");
  let timedOutMessage = "";
  try {
    await withTimeout(new Promise(() => {}), 30, "trop tard");
  } catch (e) {
    timedOutMessage = String((e as Error).message);
  }
  assert.equal(
    timedOutMessage,
    "trop tard",
    "le délai rejette AVEC le message de l'appelant — sinon la vue ne décide plus de ce qui s'affiche",
  );
  label("presse-papier : le délai transforme le silence en erreur");
}

{
  // Le vocabulaire de couleurs ne bouge pas : les lignes et les puces du
  // filtre lisent la même fonction.
  assert.equal(levelColor("ERROR"), "text-rose-deep");
  assert.equal(levelColor("warn"), "text-peach-deep");
  assert.equal(levelColor("INFO"), "text-sky-deep");
  assert.equal(levelColor("debug"), "text-azure-900/70", "le reste garde la teinte neutre");
  label("couleurs des niveaux (le vocabulaire unique de levelColor)");
}

// ── Structural tripwires for LibraryView.svelte (D4/D6/D7) ──
// Dynamic import so it runs here, after the pure-function tests (a static
// import would be hoisted and run first).
await import("./test-library-view");

// ── Structural tripwires for the DLC wiring (H1/H2/H4/M4) ───
await import("./test-dlc-wiring");
await import("./test-password-wiring");

// ── Structural tripwires for the restored GameSpotlight (SP1–SP4) ──
await import("./test-spotlight-wiring");

// ── Structural tripwires for the offline-mode wiring (O1–O6) ─


// ── Structural tripwires for the changelog feed (N1–N6) ───────
await import("./test-news-wiring");

// ── Structural tripwires for local playtime (P1–P5, LOT-13) ───
await import("./test-playtime-wiring");

// ── Structural tripwires for the artwork cache (A1–A6, LOT-14) ──

// ── Structural tripwires for the targeted repair (R1–R5, LOT-15) ──
await import("./test-repair-wiring");

// ── Structural tripwires for the multi-select (S1–S5, LOT-16) ──
await import("./test-selection-wiring");

// ── Structural tripwires for the search UX (T1–T5, LOT-17) ──


// ── Structural tripwires for the log view (U1–U6, LOT-18) ──
await import("./test-logs-wiring");

// ── LOT-19: structural tripwires for a11y wiring (V1–V7) ────
await import("./test-a11y-wiring");

// ── LOT-19: behavioural focus-trap tests ────────────────────
await import("./test-focus-trap");

// ── LOT-19: pure tests for toast-duration.ts ─────────────────
const { resolveToastDuration, DEFAULT_TOAST_MS, LONG_TOAST_MS } =
  await import("../src/lib/toast-duration");
{
  assert.equal(
    resolveToastDuration(undefined),
    DEFAULT_TOAST_MS,
    "undefined → défaut",
  );
  assert.equal(
    resolveToastDuration(null),
    DEFAULT_TOAST_MS,
    "null → défaut",
  );
  assert.equal(
    resolveToastDuration(0),
    DEFAULT_TOAST_MS,
    "zéro → défaut",
  );
  assert.equal(
    resolveToastDuration(-1),
    DEFAULT_TOAST_MS,
    "négatif → défaut",
  );
  assert.equal(
    resolveToastDuration(NaN),
    DEFAULT_TOAST_MS,
    "NaN → défaut",
  );
  assert.equal(
    resolveToastDuration(Infinity),
    DEFAULT_TOAST_MS,
    "Infini → défaut",
  );
  assert.equal(
    resolveToastDuration(3000),
    3000,
    "override positif fini → override",
  );
  assert.equal(
    resolveToastDuration(LONG_TOAST_MS),
    LONG_TOAST_MS,
    "LONG_TOAST_MS respecté",
  );
  label("resolveToastDuration (défaut, override, zéro/négatif, NaN, infini)");
}

// ── LOT-19: pure tests for eta.ts / estimateEta / formatEta ──
const { estimateEta, formatEta, isAutoPhase } =
  await import("../src/lib/eta");
{
  assert.equal(isAutoPhase("games"), false, "games n'est pas automatique");
  assert.equal(isAutoPhase("fixes"), true, "fixes est automatique");
  assert.equal(isAutoPhase("repair"), true, "repair est automatique");
  assert.equal(isAutoPhase("selection"), true, "selection est automatique");

  // phases automatiques vs games
  assert.equal(
    estimateEta([], 0).label,
    null,
    "aucun événement → pas d'ETA",
  );

  // exclusion de games
  let r = estimateEta(
    [{ phase: "games", app_id: "1", status: "ok", total: 1, cancelled: false, at: 0 }],
    1000,
  );
  assert.equal(r.label, null, "phase games → pas d'ETA");

  // zéro terminé
  r = estimateEta(
    [{ phase: "fixes", app_id: "1", status: "working", total: 2, cancelled: false, at: 0 }],
    1000,
  );
  assert.equal(r.label, null, "zéro terminé → pas d'ETA");
  assert.equal(r.completed, 0, "zéro terminé compté");

  // vitesse / reste — elapsed=120 s pour que (2*120000)/2=120000 ms → '~2 min'
  r = estimateEta(
    [
      { phase: "fixes", app_id: "1", status: "ok", total: 4, cancelled: false, at: 0 },
      { phase: "fixes", app_id: "2", status: "ok", total: 4, cancelled: false, at: 1000 },
    ],
    120000,
  );
  assert.equal(r.completed, 2, "deux terminés");
  assert.equal(r.remaining, 2, "deux restants");
  assert.equal(r.total, 4, "total correct");
  assert.ok(r.ms !== null, "ETA calculée");
  assert.equal(formatEta(r.ms!), "~2 min", "~2 min pour 2000 ms restants");

  // erreurs / ignorés comptés comme terminés
  r = estimateEta(
    [
      { phase: "repair", app_id: "1", status: "error", total: 3, cancelled: false, at: 0 },
      { phase: "repair", app_id: "2", status: "skipped", total: 3, cancelled: false, at: 500 },
    ],
    1000,
  );
  assert.equal(r.completed, 2, "erreur et skip comptés comme terminés");

  // déduplication — deux événements terminaux pour le même app_id
  // (ok puis error) prouvent que le Map retire les doublons ;
  // sans le Map, phaseSamples.filter((s) => s.status !== "working").length
  // compte ok+error (2) au lieu du seul error (1) gardé par le Map.
  r = estimateEta(
    [
      { phase: "fixes", app_id: "1", status: "ok", total: 3, cancelled: false, at: 0 },
      { phase: "fixes", app_id: "1", status: "error", total: 3, cancelled: false, at: 500 },
      { phase: "fixes", app_id: "2", status: "working", total: 3, cancelled: false, at: 0 },
    ],
    1000,
  );
  assert.equal(r.completed, 1, "le Map ne garde que le dernier événement par app_id — un seul compte terminal");
  assert.equal(r.remaining, 2, "deux restants (total 3 moins un terminé)");

  // nouvelle fenêtre au changement de phase
  r = estimateEta(
    [
      { phase: "fixes", app_id: "1", status: "ok", total: 2, cancelled: false, at: 0 },
      { phase: "repair", app_id: "2", status: "working", total: 4, cancelled: false, at: 2000 },
    ],
    3000,
  );
  assert.equal(r.completed, 0, "seule la nouvelle phase compte — remise a zero");
  assert.equal(r.label, null, "nouvelle phase working → pas d'ETA");
  assert.equal(r.remaining, 0, "reste nul quand completed=0");

  // annulation / fin
  r = estimateEta(
    [
      { phase: "fixes", app_id: "1", status: "ok", total: 2, cancelled: false, at: 0 },
      { phase: "fixes", app_id: "2", status: "ok", total: 2, cancelled: false, at: 1000 },
    ],
    2000,
  );
  assert.equal(r.label, null, "tous terminés → pas d'ETA");
  assert.equal(r.remaining, 0, "zéro restants");

  // formatEta
  assert.equal(formatEta(30000), "< 1 min", "< 1 min pour 30 s");
  assert.equal(formatEta(60000), "~1 min", "~1 min pour 60 s");
  assert.equal(formatEta(120000), "~2 min", "~2 min pour 120 s");
  assert.equal(formatEta(3600000), "~1 h", "~1 h pour 3600 s");
  assert.equal(formatEta(3900000), "~1 h 05", "~1 h 5 pour 3900 s — padStart(2) => 05");
  assert.equal(formatEta(7200000), "~2 h", "~2 h pour 7200 s");
  assert.equal(formatEta(Infinity), "< 1 min", "Infini → < 1 min");
  assert.equal(formatEta(NaN), "< 1 min", "NaN → < 1 min");
  assert.equal(formatEta(-1), "< 1 min", "négatif → < 1 min");

  label("estimateEta/formatEta (phases, exclusion games, zéro, vitesse, erreurs, dédup, nouvelle fenêtre, annulation, format)");
}

// ── LOT-21: structural tripwires for backup wiring (H1–H5, P0, H4) ──
// We use a counter exported by the suite itself. If the import is
// removed (mutation B1'), the counter is never incremented and this
// assertion turns red. This is the only mechanical proof that the
// guard suite is wired into the runner.
let backupWiringImported = false;
{
  const mod = await import("./test-backup-wiring");
  // The suite sets a marker on the module namespace when it runs.
  backupWiringImported = (mod as unknown as { __backupWiringRan: boolean }).__backupWiringRan === true;
}
assert.ok(
  backupWiringImported,
  "B1: la suite test-backup-wiring doit être importée par le runner (détection mutation B1')",
);

// ── LOT-23-B: unit tests for theme.svelte.ts (appearance guards) ──
const THEME_STATE_CASES = 8;
{
  const mod = (await import("./test-theme-state")) as unknown as {
    __themeStateRan: boolean;
    themeStateSuite: Promise<void>;
    themeStateCases: () => number;
  };
  assert.ok(
    mod.__themeStateRan === true,
    "LOT-23-B: la suite test-theme-state doit être importée par le runner (détection mutation)",
  );
  await mod.themeStateSuite;
  assert.equal(
    mod.themeStateCases(),
    THEME_STATE_CASES,
    `LOT-23-B: la suite doit aller au bout de ses ${THEME_STATE_CASES} cas`,
  );
}

// ── I18N-02: nav réactive dans App.svelte (wiring guard) ──
const I18N_WIRING_CASES = 13;
{
  const mod = (await import("./test-i18n-wiring")) as unknown as {
    __i18nWiringRan: boolean;
    i18nWiringSuite: Promise<void>;
    i18nWiringCases: () => number;
  };
  assert.ok(
    mod.__i18nWiringRan === true,
    "I18N-02: la suite test-i18n-wiring doit être importée par le runner (détection mutation)",
  );
  await mod.i18nWiringSuite;
  assert.equal(
    mod.i18nWiringCases(),
    I18N_WIRING_CASES,
    `I18N-02: la suite doit aller au bout de ses ${I18N_WIRING_CASES} cas`,
  );
}

// ── I18N-01: unit tests for i18n.svelte.ts (i18n guards) ──
const I18N_STATE_CASES = 2;
{
  const mod = (await import("./test-i18n-state")) as unknown as {
    __i18nStateRan: boolean;
    i18nStateSuite: Promise<void>;
    i18nStateCases: () => number;
  };
  assert.ok(
    mod.__i18nStateRan === true,
    "I18N-01: la suite test-i18n-state doit être importée par le runner (détection mutation)",
  );
  await mod.i18nStateSuite;
  assert.equal(
    mod.i18nStateCases(),
    I18N_STATE_CASES,
    `I18N-01: la suite doit aller au bout de ses ${I18N_STATE_CASES} cas`,
  );
}

// ── I18N-55: no French UI literal can bypass t() ────────────────
const I18N_LITERAL_CASES = 3;
{
  const mod = (await import("./test-i18n-literals")) as unknown as {
    __i18nLiteralsRan: boolean;
    i18nLiteralsSuite: Promise<void>;
    i18nLiteralsCases: () => number;
  };
  assert.ok(mod.__i18nLiteralsRan === true, "I18N-55: la garde des littéraux doit être importée par le runner");
  await mod.i18nLiteralsSuite;
  assert.equal(mod.i18nLiteralsCases(), I18N_LITERAL_CASES, "I18N-55: la garde doit aller au bout de son cas");
}

// ── MORT-10: local-import guidance must not point to the removed Search tab ──
const MORT_10_I18N_CASES = 4;
{
  const mod = (await import("./test-mort-10-i18n")) as unknown as {
    __mort10I18nRan: boolean;
    mort10I18nSuite: Promise<void>;
    mort10I18nCases: () => number;
  };
  assert.ok(mod.__mort10I18nRan === true, "MORT-10: la garde des conseils d’import local doit être importée par le runner");
  await mod.mort10I18nSuite;
  assert.equal(mod.mort10I18nCases(), MORT_10_I18N_CASES, "MORT-10: la garde doit aller au bout de ses quatre clés ciblées");
}

await import("./test-keyboard-shortcuts");

// ── MORT-06: structural guards retained after the public-edition cleanup ──
await import("./test-artwork-wiring");
await import("./test-offline-wiring");

// ── MORT-01: app-store fallback when the reachability command fails ──
const REACHABILITY_STATE_CASES = 1;
{
  const mod = (await import("./test-reachability-state")) as unknown as {
    __reachabilityStateRan: boolean;
    reachabilityStateSuite: Promise<void>;
    reachabilityStateCases: () => number;
  };
  assert.ok(mod.__reachabilityStateRan === true, "MORT-01: la suite reachability doit être importée par le runner");
  await mod.reachabilityStateSuite;
  assert.equal(
    mod.reachabilityStateCases(),
    REACHABILITY_STATE_CASES,
    `MORT-01: la suite doit aller au bout de ses ${REACHABILITY_STATE_CASES} cas`,
  );
}

// ── DMG-01: integrity-report recovery wiring ─────────────────
const DEFENDER_WIRING_CASES = 5;
{
  const mod = (await import("./test-defender-wiring")) as unknown as {
    __defenderWiringRan: boolean;
    defenderWiringSuite: Promise<void>;
    defenderWiringCases: () => number;
  };
  assert.ok(mod.__defenderWiringRan === true, "DMG-01: la garde Defender doit être importée par le runner");
  await mod.defenderWiringSuite;
  assert.equal(
    mod.defenderWiringCases(),
    DEFENDER_WIRING_CASES,
    `DMG-01: la garde doit aller au bout de ses ${DEFENDER_WIRING_CASES} cas`,
  );
}

// ── IMPORT-01: shared patch filename table and error wiring ───
const PATCH_IMPORT_CASES = 22;
{
  const mod = (await import("./test-patch-import")) as unknown as {
    __patchImportRan: boolean;
    patchImportSuite: Promise<void>;
    patchImportCases: () => number;
  };
  assert.ok(mod.__patchImportRan === true, "IMPORT-01: la suite doit être importée par le runner");
  await mod.patchImportSuite;
  assert.equal(mod.patchImportCases(), PATCH_IMPORT_CASES, `IMPORT-01: la suite doit aller au bout de ses ${PATCH_IMPORT_CASES} cas`);
}

// ── PATCH-01: secondary install-patch action decision and delegation ──
const PATCH_STATUS_CASES = 5;
{
  const mod = (await import("./test-patch-status")) as unknown as {
    __patchStatusRan: boolean;
    patchStatusSuite: Promise<void>;
    patchStatusCases: () => number;
  };
  assert.ok(mod.__patchStatusRan === true, "PATCH-01: la suite patch-status doit être importée par le runner");
  await mod.patchStatusSuite;
  assert.equal(
    mod.patchStatusCases(),
    PATCH_STATUS_CASES,
    `PATCH-01: la suite doit aller au bout de ses ${PATCH_STATUS_CASES} cas`,
  );
}

// ── OPENLUA-01: SteamTools repair endpoint and attribution ──────
const OPENLUA_STEAMTOOLS_CASES = 6;
{
  const mod = (await import("./test-openlua-steamtools")) as unknown as {
    __openluaSteamtoolsRan: boolean;
    openluaSteamtoolsSuite: Promise<void>;
    openluaSteamtoolsCases: () => number;
  };
  assert.ok(mod.__openluaSteamtoolsRan === true, "OPENLUA-01: la garde doit être importée par le runner");
  await mod.openluaSteamtoolsSuite;
  assert.equal(
    mod.openluaSteamtoolsCases(),
    OPENLUA_STEAMTOOLS_CASES,
    `OPENLUA-01: la garde doit aller au bout de ses ${OPENLUA_STEAMTOOLS_CASES} cas`,
  );
}

// ── CREDITS-01: remove stale attributions and correct LuaVault's role ──
const CREDITS_STALE_ENTRIES_CASES = 11;
{
  const mod = (await import("./test-credits-stale-entries")) as unknown as {
    __creditsStaleEntriesRan: boolean;
    creditsStaleEntriesSuite: Promise<void>;
    creditsStaleEntriesCases: () => number;
  };
  assert.ok(mod.__creditsStaleEntriesRan === true, "CREDITS-01: la garde des crédits doit être importée par le runner");
  await mod.creditsStaleEntriesSuite;
  assert.equal(
    mod.creditsStaleEntriesCases(),
    CREDITS_STALE_ENTRIES_CASES,
    `CREDITS-01: la garde doit aller au bout de ses ${CREDITS_STALE_ENTRIES_CASES} cas`,
  );
}

// ── TAURIDOCS-01: default capability and stale credit wording ──
const TAURIDOCS_CAPABILITY_CASES = 9;
{
  const mod = (await import("./test-tauridocs-capability")) as unknown as {
    __tauridocsCapabilityRan: boolean;
    tauridocsCapabilitySuite: Promise<void>;
    tauridocsCapabilityCases: () => number;
  };
  assert.ok(mod.__tauridocsCapabilityRan === true, "TAURIDOCS-01: la garde doit être importée par le runner");
  await mod.tauridocsCapabilitySuite;
  assert.equal(
    mod.tauridocsCapabilityCases(),
    TAURIDOCS_CAPABILITY_CASES,
    `TAURIDOCS-01: la garde doit aller au bout de ses ${TAURIDOCS_CAPABILITY_CASES} cas`,
  );
}

// ── STG-01: shared folder opening and visible SteamTools failures ──
const OPEN_FOLDER_CASES = 6;
{
  const mod = (await import("./test-open-folder")) as unknown as {
    __openFolderRan: boolean;
    openFolderSuite: Promise<void>;
    openFolderCases: () => number;
  };
  assert.ok(mod.__openFolderRan === true, "STG-01: la suite open-folder doit être importée par le runner");
  await mod.openFolderSuite;
  assert.equal(mod.openFolderCases(), OPEN_FOLDER_CASES, `STG-01: la suite doit aller au bout de ses ${OPEN_FOLDER_CASES} cas`);
}

// ── BANC-01: every graphical-suite import resolves on disk ───
const E2E_IMPORT_CASES = 24;
{
  const mod = (await import("./test-e2e-imports")) as unknown as {
    __e2eImportsRan: boolean;
    e2eImportsSuite: Promise<void>;
    e2eImportsCases: () => number;
  };
  assert.ok(mod.__e2eImportsRan === true, "BANC-01: la garde des imports e2e doit être importée par le runner");
  await mod.e2eImportsSuite;
  assert.equal(
    mod.e2eImportsCases(),
    E2E_IMPORT_CASES,
    `BANC-01: la garde doit aller au bout de ses ${E2E_IMPORT_CASES} cas`,
  );
}

console.log("\nTous les tests frontend passent.");

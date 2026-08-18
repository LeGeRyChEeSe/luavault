/**
 * Structural guard rails for multi-select and its actions (LOT-16).
 *
 * Same contract as test-repair-wiring.ts: these are NOT behaviour tests.
 * The Rust behaviour (the selection predicate executed by the shared loop
 * and by the preflight, the fix_external refusal, the hidden defence) is
 * pinned by the unit tests in commands.rs — each goes red when the
 * invariant it protects is removed. The selection counts, the purge and the
 * filter-bounded "select all" are pinned by the pure-function tests in
 * test-virtual-scroll.ts. The guards here pin the WIRING neither can see:
 * the predicate called where the pass and the plan select, the command
 * registered and invoked, the bar that only exists with a selection, the
 * single overlay the fifth mode reuses.
 *
 * Every pattern runs on a STRIPPED copy of the source
 * (stripCommentsAndStrings / stripComments from test-dlc-wiring.ts), so a
 * commented-out call or a tooltip quoting the markup never satisfies a
 * guard. Guards that need a string literal (labels, invoke names, stage
 * names) run on the stripComments copy — strings kept, comments gone.
 *
 * Deliberately loose on shape: each assertion goes red on a
 * behaviour-breaking revert, not on a rename or a reformat.
 */

// @ts-expect-error — `node:fs` has no types here: @types/node is not a
// project dependency. See the identical note in test-artwork-wiring.ts.
import { readFileSync as readFileSyncRaw } from "node:fs";
import { stripComments, stripCommentsAndStrings } from "./test-dlc-wiring";
import { fr } from "../src/lib/i18n/fr";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

function assertOk(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

// ── Read and strip the sources ────────────────────────────────
// Rust: stripComments keeps the stage-name string literals — the sets ARE
// strings, so the strings-stripped copy would leave nothing to pin.
const commandsRs = stripComments(
  readFileSync("src-tauri/src/commands.rs", { encoding: "utf8" }),
);
const libRs = stripCommentsAndStrings(
  readFileSync("src-tauri/src/lib.rs", { encoding: "utf8" }),
);
const libraryViewStr = stripComments(
  readFileSync("src/views/LibraryView.svelte", { encoding: "utf8" }),
);
const frCatalogue = readFileSync("src/lib/i18n/fr.ts", { encoding: "utf8" });
const libraryView = stripCommentsAndStrings(
  readFileSync("src/views/LibraryView.svelte", { encoding: "utf8" }),
);
const bulkProgressStr = stripComments(
  readFileSync("src/components/BulkProgress.svelte", { encoding: "utf8" }),
);
const gameCardStr = stripComments(
  readFileSync("src/views/GameCard.svelte", { encoding: "utf8" }),
);
const apiStr = stripComments(readFileSync("src/lib/api.ts", { encoding: "utf8" }));
const selectionStr = stripComments(
  readFileSync("src/lib/selection.ts", { encoding: "utf8" }),
);
const libraryStateStr = stripComments(
  readFileSync("src/lib/library-state.svelte.ts", { encoding: "utf8" }),
);

// ── S1 — the pure predicate, wired where the pass and the plan select ─

// S1-1: the predicate exists, a function of the AppID and the chosen list.
assertOk(
  /fn in_selection\([^)]*app_id: &str[^)]*selection: &\[String\]/.test(commandsRs),
  "S1-1: in_selection est une fonction pure de l'AppID et de la liste choisie",
);

// S1-2: the shared loop filters through it — extract the candidate filter
// itself, not the file at large.
{
  const start = commandsRs.indexOf("async fn bulk_fixes_core");
  const body = start >= 0 ? commandsRs.slice(start, start + 1800) : "";
  assertOk(
    body.includes("in_selection("),
    "S1-2: la boucle partagée filtre sur in_selection — un jeu non sélectionné n'entre pas",
  );
}

// S1-3: the preflight filters through the same predicate — the confirmation
// screen and the pass can never disagree about who is in.
{
  const start = commandsRs.indexOf("fn bulk_preflight_plan");
  const body = start >= 0 ? commandsRs.slice(start, start + 3000) : "";
  assertOk(
    body.includes("in_selection("),
    "S1-3: la pré-vérification de la sélection utilise le même prédicat que la passe",
  );
}

// S1-4: the fifth mode's command wires the installable set AND the list.
assertOk(
  /fn apply_fixes_to_selection[\s\S]{0,600}?bulk_fixes\([\s\S]{0,200}?INSTALLABLE_FIX_STAGES[\s\S]{0,80}?Some\(app_ids\)/.test(commandsRs),
  "S1-4: apply_fixes_to_selection câble la boucle partagée sur les états patchables, restreinte aux AppID choisis",
);

// S1-5: both entry points accept the selection.
assertOk(
  /fn bulk_preflight\([\s\S]{0,300}?selection: Option<Vec<String>>/.test(commandsRs),
  "S1-5: bulk_preflight accepte la sélection (cinquième mode)",
);
assertOk(
  /async fn bulk_fixes_core[\s\S]{0,400}?selection: Option<&\[String\]>/.test(commandsRs),
  "S1-5: la boucle partagée accepte la sélection (cinquième mode)",
);

// S1-6: the command is registered.
assertOk(
  /commands::apply_fixes_to_selection/.test(libRs),
  "S1-6: apply_fixes_to_selection est enregistrée dans invoke_handler",
);

// ── S2 — the selection lives in the shared store, purged, filter-bounded ─

assertOk(
  /selection = \$state<string\[\]>\(\[\]\)/.test(libraryStateStr),
  "S2-1: la sélection vit dans library-state (persiste entre navigations)",
);
assertOk(
  /selectionMode = \$state\(false\)/.test(libraryStateStr),
  "S2-1: le mode sélection s'active explicitement",
);

// S2-2: the purge — wired to the whole library, like selectedTags'.
// The SECOND argument is the load-bearing one: purging against the filtered
// list (`shown`) would wipe the selection at the first keystroke in the
// search box and drop every hidden game, the exact opposite of what
// purgeSelection's own comment promises.
assertOk(
  /purgeSelection\(libraryState\.selection,\s*statuses\)/.test(libraryViewStr),
  "S2-2: la purge porte sur la bibliothèque entière, pas sur la liste filtrée",
);

// S2-3: select all / deselect all stop at the current filter.
assertOk(
  /selectAllVisible\(shown/.test(libraryViewStr),
  "S2-3: « tout sélectionner » porte sur ce qui est visible après filtre et recherche",
);
assertOk(
  /deselectVisible\(shown/.test(libraryViewStr),
  "S2-3: « tout désélectionner » porte sur ce qui est visible après filtre et recherche",
);

// S2-4: the actions bar exists ONLY with a non-empty selection — an
// unconditional bar here is the reverted guard.
assertOk(
  /\{#if libraryState\.selection\.length > 0\}[\s\S]{0,1200}?t\(\s*"library\.multi\.fixes"\s*,\s*\{\s*n:\s*selectionCounts\.fixes\s*\}/.test(libraryViewStr),
  "S2-4: la barre d'actions de la sélection n'apparaît que si la sélection n'est pas vide",
);

// S2-5: the counts come from the eligibility, and the buttons display them.
assertOk(
  /selectionCounts = \$derived[\s\S]{0,800}?eligibleForSelectionAction\(selectedVisible/.test(libraryViewStr),
  "S2-5: les comptes des boutons comptent les jeux éligibles parmi les sélectionnés visibles",
);
// S2-5b: `selectedVisible` — the list every count and every pass starts
// from — is the SELECTION INTERSECTED WITH WHAT IS ON SCREEN. Deriving it
// from `statuses` or `visible` instead of `shown` would make the buttons
// count games the current filter hides (and hidden games, which the backend
// pass skips): a number on a button the pass would not honour, and an
// action on a game invisible at click time.
assertOk(
  /selectedVisible = \$derived\(\s*shown\.filter\(/.test(libraryViewStr),
  "S2-5b: les comptes partent de la sélection ∩ l'écran (shown), jamais de la bibliothèque entière",
);
// S2-5c: every button of the bar displays ITS OWN eligibility count —
// pinning only the patches button let the five others drift to
// `selection.length`, which is the count that lies.
// Deux moitiés depuis I18N-34 : le bouton cite SA clé ET SON compte, et la
// valeur du catalogue porte le mot de l'action. Épingler la seule clé
// laisserait le libellé annoncer une autre action ; épingler la seule valeur
// laisserait le compte dériver — c'est-à-dire le défaut d'origine.
for (const [key, field, word] of [
  ["library.multi.fixes", "fixes", /appliquer/i],
  ["library.multi.verify", "verify", /vérifier/i],
  ["library.multi.copy", "copy", /copier/i],
  ["library.multi.add", "addTag", /ajouter/i],
  ["library.multi.remove", "removeTag", /retirer/i],
  ["library.multi.hide", "hide", /masquer/i],
] as const) {
  assertOk(
    new RegExp(`t\\(\\s*"${key}"\\s*,\\s*\\{\\s*n:\\s*selectionCounts\\.${field}\\s*\\}`).test(
      libraryViewStr,
    ),
    `S2-5c: le bouton « ${key} » affiche le compte des jeux qu'il traitera (selectionCounts.${field})`,
  );
  // Les points de la clé sont échappés : sans cela `.` reste le joker des
  // expressions régulières et le motif ne dit pas ce qu'il prétend dire.
  const value = new RegExp(`"${key.replace(/\./g, "\\.")}":\\s*"([^"]*)"`).exec(frCatalogue);
  assertOk(
    value !== null && word.test(value[1]),
    `S2-5c: la valeur de ${key} doit parler de « ${word} » — trouvé « ${value?.[1] ?? "(absente)"} »`,
  );
}
// S2-5d: the pass runs on the SAME eligibility the button counted, captured
// at click time. `startSelection` handing the raw `selectedVisible` to the
// run — or `runSelectionLocal` re-reading the live list instead of the
// capture — is the recurring defect: the bar says three, the pass treats ten.
assertOk(
  /function startSelection[\s\S]{0,400}?targets = eligibleForSelectionAction\(selectedVisible, action, tag\)/.test(
    libraryViewStr,
  ),
  "S2-5d: les cibles de la passe sont exactement les jeux éligibles comptés par le bouton",
);
assertOk(
  /function runSelectionLocal[\s\S]{0,200}?targets = selectionRunTargets/.test(libraryViewStr),
  "S2-5d: la passe locale traite la capture faite au clic, jamais une liste relue depuis",
);

// S2-6: the fifth mode reaches the backend — preflight with the chosen
// AppIDs, then the pass on exactly what the plan confirmed.
assertOk(
  /bulkPreflight\(false, targets\.map/.test(libraryViewStr),
  "S2-6: la pré-vérification de la sélection transmet les AppID choisis",
);
assertOk(
  /applyFixesToSelection\(fixes\.map/.test(libraryViewStr),
  "S2-6: la passe des patchs traite exactement ce que l'écran de confirmation a validé",
);
assertOk(
  /export const applyFixesToSelection[\s\S]{0,200}?"apply_fixes_to_selection"/.test(apiStr),
  "S2-6: applyFixesToSelection appelle la commande apply_fixes_to_selection",
);
assertOk(
  /export const bulkPreflight[\s\S]{0,300}?selection: selection \?\? null/.test(apiStr),
  "S2-6: bulkPreflight transmet la sélection (ou null)",
);

// ── S3 — eligibility mirrors the pass: never fix_external, never a
//         game Steam is still downloading ─
{
  const m = selectionStr.match(/SELECTION_FIX_STAGES = \[[^\]]*\]/);
  assertOk(m !== null, "S3-1: SELECTION_FIX_STAGES est défini dans selection.ts");
  const set = m![0];
  assertOk(set.includes("fix_downloaded"), "S3-1: un patch téléchargé est applicable");
  assertOk(set.includes("fix_damaged"), "S3-1: un patch endommagé est applicable (réparation)");
  assertOk(set.includes("fix_game_moved"), "S3-1: un patch déplacé est applicable (réparation)");
  assertOk(!set.includes("fix_external"), "S3-1: aucune sauvegarde n'existe pour un patch tiers — jamais dans la sélection traitée");
}
{
  // The window stops at the NEXT case: sliced by a fixed character count it
  // reached into `case "verify":`, whose own `fully_installed` satisfied the
  // guard while the patch branch had lost its check.
  const start = selectionStr.indexOf('case "fixes":');
  const rest = start >= 0 ? selectionStr.slice(start + 1) : "";
  const end = rest.indexOf("case ");
  const body = end >= 0 ? rest.slice(0, end) : rest;
  assertOk(
    body.length > 0 && body.includes("fully_installed") && body.includes("fix_downloaded"),
    "S3-2: le compte des patchs exige un jeu installé et un patch téléchargé — les pré-vérifications de la passe",
  );
}

// ── S4 — one overlay, one progress subscription: the fifth mode reuses ─
assertOk(
  (libraryView.match(/listen<BulkProgressEvent>/g) ?? []).length === 1,
  "S4-1: une seule souscription à la progression du lot (pas de seconde boucle d'écoute)",
);
assertOk(
  (libraryView.match(/<BulkProgress\b/g) ?? []).length === 1,
  "S4-1: la sélection réutilise l'overlay BulkProgress existant, pas un second",
);
assertOk(
  /\{#if plan\.selection\.length > 0\}/.test(bulkProgressStr),
  "S4-2: l'overlay affiche la liste de confirmation d'une action locale de sélection",
);
assertOk(
  /totalSteps = \$derived[\s\S]{0,120}?plan\.selection\.length/.test(bulkProgressStr),
  "S4-2: la progression compte aussi les opérations de la sélection",
);
// S4-3: la phase selection porte son libellé dans l'overlay.
// I18N-22 l'a déplacé : le dictionnaire PHASE_LABEL a disparu, la vue compose
// `bulk.phase.${phase}` et le texte vit dans le catalogue. Les DEUX moitiés sont
// exigées — épingler la seule composition laisserait la clé sans texte, épingler
// le seul catalogue laisserait la vue cesser de l'appeler.
assertOk(
  /t\(`bulk\.phase\.\$\{[^}]*\bphase\}`\)/.test(bulkProgressStr),
  "S4-3: l'overlay compose le libellé de phase depuis le catalogue",
);
assertOk(
  (fr["bulk.phase.selection"] ?? "").length > 0,
  "S4-3: la phase selection a bien un libellé dans le catalogue",
);

// ── S5 — the cards: checkboxes only while the mode is on ─
assertOk(
  /selecting=\{libraryState\.selectionMode\}/.test(libraryViewStr),
  "S5-1: les cartes ne montrent leurs cases qu'en mode sélection",
);
assertOk(
  /\{#if selecting\}[\s\S]{0,1500}?name="checkbox"/.test(gameCardStr),
  "S5-2: la case à cocher n'existe que dans le rendu sélection",
);
assertOk(
  /\{:else\}\s*<GameActions/.test(gameCardStr),
  "S5-3: l'action individuelle de la carte s'efface devant la sélection",
);

console.log("  ✓ tripwires structurels sélection (LOT-16 : S1 prédicat, S2 état/purge/barre, S3 éligibilité, S4 overlay unique, S5 cartes)");

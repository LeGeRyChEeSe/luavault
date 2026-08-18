/**
 * Structural guard rails for the targeted repair pass (LOT-15).
 *
 * Same contract as test-artwork-wiring.ts: these are NOT behaviour tests.
 * The Rust behaviour (the stage selection, the moved-game backup survival,
 * the failure counting) is pinned by the unit tests in commands.rs and
 * fixes.rs — each one goes red when the invariant it protects is removed.
 * The guards here pin the WIRING the unit tests cannot see: the command
 * registered and called, the button that exists only when something is
 * broken, the single overlay and progress subscription the repair reuses,
 * and the report's failures wired into the final summary.
 *
 * Every pattern runs on a STRIPPED copy of the source
 * (stripCommentsAndStrings / stripComments from test-dlc-wiring.ts), so a
 * commented-out call or a tooltip quoting the markup never satisfies a
 * guard. Guards that need a string literal (the stage names, the invoke
 * command name, the button label) run on the stripComments copy — strings
 * kept, comments gone.
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
const fixesRs = stripCommentsAndStrings(
  readFileSync("src-tauri/src/fixes.rs", { encoding: "utf8" }),
);
const libRs = stripCommentsAndStrings(
  readFileSync("src-tauri/src/lib.rs", { encoding: "utf8" }),
);
// Svelte: strings kept when the wiring lives in a string (the button label,
// the startBulk mode, the invoke argument), stripped elsewhere.
const libraryView = stripCommentsAndStrings(
  readFileSync("src/views/LibraryView.svelte", { encoding: "utf8" }),
);
const libraryViewStr = stripComments(
  readFileSync("src/views/LibraryView.svelte", { encoding: "utf8" }),
);
const bulkProgressStr = stripComments(
  readFileSync("src/components/BulkProgress.svelte", { encoding: "utf8" }),
);
const apiStr = stripComments(readFileSync("src/lib/api.ts", { encoding: "utf8" }));

// ── R1 — the repair selection is wired, not duplicated ─────────

// R1-1: the repair set is exactly the two broken stages. Extract the
// declaration itself — identifiers co-present elsewhere don't count.
{
  const m = commandsRs.match(/REPAIRABLE_FIX_STAGES[^=]*=\s*&\[[^\]]*\]/);
  assertOk(m !== null, "R1-1: REPAIRABLE_FIX_STAGES doit être défini dans commands.rs");
  const set = m![0];
  assertOk(set.includes("fix_damaged"), "R1-1: un patch endommagé est réparable");
  assertOk(set.includes("fix_game_moved"), "R1-1: un patch dont le jeu a bougé est réparable");
  assertOk(!set.includes("fix_" + "available"), "R1-1: installer un patch jamais eu n'est pas réparer");
  assertOk(!set.includes("fix_downloaded"), "R1-1: un patch jamais appliqué n'est pas une réparation");
  assertOk(!set.includes("fix_external"), "R1-1: aucune sauvegarde n'existe pour un patch tiers — jamais réinstallé par-dessus");
}

// R1-2: each command wires its own set into the shared loop.
assertOk(
  /fn repair_all_fixes[\s\S]{0,400}?REPAIRABLE_FIX_STAGES/.test(commandsRs),
  "R1-2: repair_all_fixes câble le lot partagé sur les seuls états endommagés",
);
assertOk(
  /fn bulk_install_fixes[\s\S]{0,400}?INSTALLABLE_FIX_STAGES/.test(commandsRs),
  "R1-2: install_all_fixes câble le lot partagé sur tous les états patchables",
);

// R1-3: the selection is the shared predicate — preflight and both passes.
assertOk(
  (commandsRs.match(/is_fix_candidate\(/g) ?? []).length >= 3,
  "R1-3: is_fix_candidate sert la pré-vérification et les deux lots (pas de filtre dupliqué)",
);

// R1-4: the preflight takes repair_only, so the confirmation screen shows
// exactly what the repair pass will treat.
assertOk(
  /fn bulk_preflight\([\s\S]{0,200}?repair_only/.test(commandsRs),
  "R1-4: bulk_preflight accepte repair_only",
);

// R1-5: one loop, one progress emission point — a duplicated loop shows up
// as a second occurrence of its working message.
assertOk(
  (commandsRs.match(/téléchargement du patch/g) ?? []).length === 1,
  "R1-5: la boucle de lot n'est pas dupliquée (un seul point d'émission de la progression)",
);

// ── R2 — a moved game's backup survives, renamed after its folder ─
{
  const installStart = fixesRs.indexOf("pub fn install(");
  const body = installStart >= 0 ? fixesRs.slice(installStart) : "";
  assertOk(
    installStart >= 0 && /moved_backup_path\(/.test(body),
    "R2-1: install() connaît un nom de sauvegarde portant le dossier du jeu",
  );
  assertOk(
    /std::fs::rename\(&backup/.test(body),
    "R2-1: quand le dossier du jeu a changé, l'ancienne sauvegarde est renommée — pas écrasée",
  );
}

// ── R3 — the button exists only when something is broken ────────

// R3-1 : le bouton n'existe que si des patchs sont à réparer (N > 0).
// Deux moitiés depuis I18N-33 : le {#if} doit citer LA clé du bouton, et la
// valeur du catalogue doit porter le mot. Épingler la seule clé laisserait le
// libellé devenir n'importe quoi ; épingler la seule valeur laisserait le
// bouton devenir inconditionnel.
assertOk(
  /\{#if pending\.repair > 0\}[\s\S]{0,600}?ActionButton(?:(?!\/>)[\s\S])*?label=\{t\("library\.bulk\.repair"/.test(libraryViewStr),
  "R3-1: le bouton « Réparer les patchs » n'existe que si des patchs sont à réparer (N > 0)",
);
assertOk(
  /"library\.bulk\.repair":\s*"[^"]*[Rr]éparer[^"]*"/.test(
    readFileSync("src/lib/i18n/fr.ts", { encoding: "utf8" }),
  ),
  "R3-1: la valeur de library.bulk.repair doit parler de réparer",
);

// R3-2: the count mirrors the backend selection — the two broken stages,
// and only them.
{
  const start = libraryViewStr.indexOf("repair: visible.filter");
  const end = libraryViewStr.indexOf(").length,", start);
  const snippet = start >= 0 && end > start ? libraryViewStr.slice(start, end) : "";
  assertOk(snippet.includes("fix_damaged"), "R3-2: le compte du bouton répare les patchs endommagés");
  assertOk(snippet.includes("fix_game_moved"), "R3-2: le compte du bouton répare les jeux déplacés");
  assertOk(!snippet.includes('"fix_' + 'available"'), "R3-2: le compte du bouton n'inclut pas les patchs jamais eus");
  assertOk(!snippet.includes('"fix_external"'), "R3-2: le compte du bouton n'inclut jamais les patchs tiers");
}

// R3-3: the button launches the repair pass, preflighted as repair-only.
assertOk(
  /startBulk\("repair"\)/.test(libraryViewStr),
  "R3-3: le bouton lance le lot de réparation",
);
assertOk(
  /bulkPreflight\(mode === "repair"\)/.test(libraryViewStr),
  "R3-3: la pré-vérification d'une réparation ne compte que les réparations",
);

// R3-4: the repair pass runs through the SAME overlay machinery — one
// progress subscription, one <BulkProgress>, one launcher parameterised.
assertOk(
  (libraryView.match(/listen<BulkProgressEvent>/g) ?? []).length === 1,
  "R3-4: une seule souscription à la progression du lot (pas de seconde boucle d'écoute)",
);
assertOk(
  (libraryView.match(/<BulkProgress\b/g) ?? []).length === 1,
  "R3-4: la réparation réutilise l'overlay BulkProgress existant, pas un second",
);
assertOk(
  /bulkMode === "repair" \? repairAllFixes : installAllFixes/.test(libraryViewStr),
  "R3-4: le lot partagé est lancé par repairAllFixes en mode réparation",
);

// R3-5: offline, a downloaded patch still repairs — the button's disabled
// clause must not depend on appState.online (its tooltip may).
{
  const ifStart = libraryViewStr.indexOf("{#if pending.repair > 0}");
  const blockEnd = libraryViewStr.indexOf("/>", ifStart);
  const block = ifStart >= 0 && blockEnd > ifStart ? libraryViewStr.slice(ifStart, blockEnd) : "";
  const disabled = block.match(/disabled=\{([^}]*)\}/);
  assertOk(
    disabled !== null && !disabled[1].includes("appState.online"),
    "R3-5: un patch déjà téléchargé se répare sans réseau — le bouton ne se désactive pas hors ligne",
  );
}

// ── R4 — the frontend reaches the repair command ──────────────────
assertOk(
  /export const repairAllFixes[\s\S]{0,200}?"repair_all_fixes"/.test(apiStr),
  "R4-1: repairAllFixes appelle la commande repair_all_fixes",
);
assertOk(
  /export const bulkPreflight[\s\S]{0,300}?repairOnly/.test(apiStr),
  "R4-2: bulkPreflight transmet repairOnly",
);
assertOk(
  /commands::repair_all_fixes/.test(libRs),
  "R4-3: repair_all_fixes est enregistrée dans invoke_handler",
);

// ── R5 — the report never lies: failures reach the summary ────────
assertOk(
  /report\.failed > 0[\s\S]{0,400}?t\("bulk\.report\.failed",\s*\{\s*n:\s*report\.failed\s*\}\)/.test(
    bulkProgressStr,
  ),
  "R5-1: le compte-rendu final affiche les échec(s) comptés par le lot",
);
assertOk(
  /Réparation/.test(fr["bulk.repair.done"]),
  "R5-2: une réparation terminée se lit comme une réparation",
);
assertOk(
  /Réparation/.test(fr["bulk.phase.repair"]),
  "R5-2: la phase repair porte son libellé dans l'overlay",
);

console.log("  ✓ tripwires structurels réparation (LOT-15 : R1 sélection, R2 sauvegarde déplacée, R3 bouton, R4 commandes, R5 compte-rendu)");

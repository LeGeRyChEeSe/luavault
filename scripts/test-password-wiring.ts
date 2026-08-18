/**
 * Structural guards for the default archive-password flow (PWD-01).
 * They pin the two boundaries that the graphical bench cannot force: a
 * password is retained only after a successful action, and the next dialog
 * receives the retained value rather than an empty field.
 */
// @ts-expect-error — @types/node is intentionally not a project dependency.
import { readFileSync as readFileSyncRaw } from "node:fs";
import { shouldRememberArchivePassword } from "../src/lib/api";
import { stripCommentsAndStrings } from "./test-dlc-wiring";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

function assertOk(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

function functionBody(source: string, signature: string): string {
  const start = source.indexOf(signature);
  assertOk(start >= 0, `signature introuvable : ${signature}`);
  const brace = source.indexOf("{", start);
  assertOk(brace >= 0, `accolade ouvrante introuvable : ${signature}`);
  let depth = 0;
  for (let index = brace; index < source.length; index++) {
    if (source[index] === "{") depth++;
    if (source[index] === "}" && --depth === 0) return source.slice(brace, index + 1);
  }
  throw new Error(`accolade fermante introuvable : ${signature}`);
}

const actions = stripCommentsAndStrings(
  readFileSync("src/components/GameActions.svelte", { encoding: "utf8" }),
);
const run = functionBody(actions, "async function run(");
const refreshLibrary = run.indexOf("await appState.refreshLibrary()");
const remember = run.indexOf("setDefaultArchivePassword");
const catchBlock = run.indexOf("catch (e)");
const catchBody = catchBlock >= 0 ? functionBody(run.slice(catchBlock), "catch (e)") : "";

for (const [submitted, current, expected, label] of [
  ["nouveau-mot-de-passe", "mot-de-passe-actuel", true, "mot de passe différent"],
  ["mot-de-passe", "mot-de-passe", false, "mot de passe identique"],
  [undefined, "mot-de-passe", false, "mot de passe absent"],
  [null, "mot-de-passe", false, "mot de passe null"],
  ["", "mot-de-passe", false, "mot de passe vide"],
  ["premier-mot-de-passe", undefined, true, "premier mot de passe"],
] as const) {
  assertOk(
    shouldRememberArchivePassword(submitted, current) === expected,
    `PWD-01: ${label} doit produire ${expected}`,
  );
}

assertOk(refreshLibrary >= 0, "PWD-01: run doit rafraîchir la bibliothèque après succès");
assertOk(
  remember > refreshLibrary,
  "PWD-01: le mot de passe n'est mémorisé qu'après refreshLibrary dans la branche de succès",
);
assertOk(
  remember >= 0 && (catchBlock < 0 || remember < catchBlock) && !catchBody.includes("setDefaultArchivePassword"),
  "PWD-01: un échec de mot de passe ne doit jamais écraser le mot de passe mémorisé",
);
assertOk(
  /\bshouldRememberArchivePassword\s*\(\s*submittedArchivePassword\s*,\s*appState\.report\?\.default_archive_password\s*,?\s*\)/.test(run),
  "PWD-01: run doit déléguer la décision de mémorisation à shouldRememberArchivePassword",
);
assertOk(
  !/\bsubmittedArchivePassword\s*(?:!==|===)/.test(run),
  "PWD-01: run ne doit pas réécrire la comparaison de mémorisation",
);

const dialogOpen = actions.indexOf("passwordAction = retryWithPassword");
const beforeDialog = actions.slice(Math.max(0, dialogOpen - 180), dialogOpen);
assertOk(
  dialogOpen >= 0 && beforeDialog.includes("archivePassword = appState.report?.default_archive_password ??"),
  "PWD-01: le dialogue doit préremplir archivePassword depuis default_archive_password",
);

console.log("  ✓ tripwires PWD-01 (succès avant mémorisation, préremplissage du dialogue)");

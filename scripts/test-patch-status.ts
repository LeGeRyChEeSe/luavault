// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync } from "node:fs";
import { shouldOfferInstallFix, type InstallFixStatus } from "../src/lib/patch-status";
import { stripCommentsAndStrings } from "./test-dlc-wiring";

let cases = 0;

function status(
  fix_downloaded: boolean,
  stage: string,
  health: InstallFixStatus["fix"]["health"],
): InstallFixStatus {
  return { fix_downloaded, stage, fix: { health } };
}

for (const [input, expected, label] of [
  [status(true, "lua_not_in_steam", "not_installed"), true, "archive téléchargée, .lua pas encore copié"],
  [status(true, "lua_not_in_steam", "healthy"), false, "patch sain malgré le stage lua_not_in_steam"],
  [status(true, "fix_installed", "healthy"), false, "stage fix déjà traité"],
  [status(false, "ready", "not_installed"), false, "aucune archive disponible"],
] as const) {
  assert.equal(shouldOfferInstallFix(input), expected, `PATCH-01: ${label}`);
  cases++;
}

const source = readFileSync("src/components/GameActions.svelte", "utf8");
const actionLabel = 'label={t("actions.downloaded-fix.label")}';
const actionLabelIndex = source.indexOf(actionLabel);
assert.ok(actionLabelIndex >= 0, "PATCH-01: le bouton secondaire Install patch doit rester identifiable");
const ifStart = source.lastIndexOf("{#if", actionLabelIndex);
const ifEnd = source.indexOf("}", ifStart);
assert.ok(ifStart >= 0 && ifEnd > ifStart, "PATCH-01: le {#if} du bouton secondaire doit être délimité");
const installFixCondition = stripCommentsAndStrings(source.slice(ifStart, ifEnd + 1));
assert.match(
  installFixCondition,
  /^\{#if\s+shouldOfferInstallFix\s*\(\s*status\s*\)\s*\}$/,
  "PATCH-01: le bouton secondaire délègue sa décision à shouldOfferInstallFix",
);
cases++;

export const __patchStatusRan = true;
export const patchStatusSuite = Promise.resolve();
export const patchStatusCases = () => cases;

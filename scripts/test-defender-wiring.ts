// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync as readFileSyncRaw } from "node:fs";
import { stripComments, stripCommentsAndStrings } from "./test-dlc-wiring";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

let cases = 0;
function check(value: unknown, message: string): void {
  if (!value) throw new Error(message);
  cases++;
}

function bodyForIf(source: string, condition: string, label: string): string {
  const conditionStart = source.indexOf(condition);
  if (conditionStart < 0) throw new Error(`${label}: la condition existe`);
  const blockStart = source.indexOf("{", conditionStart + condition.length);
  if (blockStart < 0) throw new Error(`${label}: la condition ouvre son bloc`);

  let depth = 0;
  for (let index = blockStart; index < source.length; index++) {
    if (source[index] === "{") depth++;
    if (source[index] === "}") {
      depth--;
      if (depth === 0) return source.slice(blockStart + 1, index);
    }
  }
  throw new Error(`${label}: le bloc de la condition n'est pas ferme`);
}

function assertBranchMessage(
  source: string,
  condition: string,
  expected: string,
  unexpected: string,
  label: string,
): void {
  const body = bodyForIf(source, condition, label);
  if (!body.includes(expected) || body.includes(unexpected)) {
    throw new Error(`${label}: le texte attendu appartient a une autre branche`);
  }
}

const raw = readFileSync("src/lib/defender.ts", { encoding: "utf8" });
const decisionStart = raw.indexOf("function repairMessageFor(");
const decisionEnd = raw.indexOf("async function offerExclusionRepair", decisionStart);
const decision = stripComments(raw.slice(decisionStart, decisionEnd));
const offerEnd = raw.indexOf("export async function installFixWithRepair", decisionEnd);
const offer = stripComments(raw.slice(decisionEnd, offerEnd));

check(decisionStart >= 0 && decisionEnd > decisionStart, "D1: la decision de recuperation existe");
check(
  /report\.missing\.length\s*>\s*0[\s\S]*?defenderActive/.test(decision),
  "D1: les fichiers absents choisissent Defender actif ou antivirus tiers",
);
const missingDecision = decision.indexOf("report.missing.length");
const modifiedDecision = decision.indexOf("report.modified.length");
check(
  missingDecision >= 0
    && modifiedDecision >= 0
    && missingDecision < modifiedDecision
    && /report\.modified\.length\s*>\s*0\)\s*return\s+"modified";/.test(decision),
  "D1: les fichiers absents restent prioritaires sur les fichiers modifies",
);
check(
  offer.includes("repairMessageFor(report, status.available && status.active)"),
  "D1: la decision Defender recoit le statut disponible et actif reels",
);
const modifiedBranch = bodyForIf(offer, 'if (message === "modified")', "D1: la branche fichiers modifies");
check(
  modifiedBranch.includes('t("shell.defender.modified.body")'),
  "D1: le message modifie est selectionne par la decision, pas seulement declare",
);
assertBranchMessage(
  offer,
  'if (message === "missing")',
  't("shell.defender.repair.body")',
  't("shell.defender.passive.body")',
  "D1: la branche fichiers absents",
);
assertBranchMessage(
  offer,
  'if (message === "passive")',
  't("shell.defender.passive.body")',
  't("shell.defender.repair.body")',
  "D1: la branche antivirus tiers passif",
);

export const __defenderWiringRan = true;
export const defenderWiringSuite = Promise.resolve();
export const defenderWiringCases = () => cases;

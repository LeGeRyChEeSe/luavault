// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { existsSync, readFileSync } from "node:fs";
import { creditLines } from "../src/lib/spotlight-credits";
import { stripComments } from "./test-dlc-wiring";

let cases = 0;
function check(value: unknown, message: string) { assert.ok(value, message); cases++; }
function checkEqual(actual: unknown, expected: unknown, message: string) {
  assert.deepEqual(actual, expected, message);
  cases++;
}

check(!existsSync("src/views/SearchView.svelte"), "la vue de recherche privée est absente");
check(!existsSync("src/components/LicenceGate.svelte"), "la porte de licence est absente");
check(!/nav\.search/.test(readFileSync("src/App.svelte", "utf8")), "la navigation ne propose pas de recherche");

const MIGRATED_SOURCES = ["src/lib/defender.ts"];
for (const source of MIGRATED_SOURCES) {
  const code = stripComments(readFileSync(source, "utf8"));
  check(!/[À-ɏ]/.test(code), `${source} ne contient aucun texte francais en dur`);
  check(/\bt\(/.test(code), `${source} passe ses textes visibles par t()`);
}

const gameSpotlight = stripComments(readFileSync("src/components/GameSpotlight.svelte", "utf8"));
const template = gameSpotlight.slice(gameSpotlight.indexOf("</script>") + "</script>".length);
const creditLoop = /\{#each\s+creditLines\(\s*details\?\.developers\s*,\s*details\?\.publishers\s*\)\s+as\s+line\}([\s\S]*?)\{\/each\}/.exec(template);

check(
  creditLoop !== null,
  "GameSpotlight délègue ses crédits à creditLines(details?.developers, details?.publishers)",
);
check(
  creditLoop !== null
    && /\{#if\s+line\.key\s*===\s*"combined"\s*\}[\s\S]*?t\("spotlight\.developer"\)[\s\S]*?t\("spotlight\.publisher"\)[\s\S]*?\{:else if\s+line\.key\s*===\s*"spotlight\.developer"\s*\}[\s\S]*?t\("spotlight\.developer"\)[\s\S]*?\{:else\}[\s\S]*?t\("spotlight\.publisher"\)/.test(creditLoop[1]),
  "chaque branche de la boucle de crédits conserve son libellé i18n local",
);

checkEqual(
  creditLines(["A"], ["A"]),
  [{ key: "combined", names: "A" }],
  "crédits identiques → une ligne combinée",
);
checkEqual(
  creditLines(["A"], ["B"]),
  [
    { key: "spotlight.developer", names: "A" },
    { key: "spotlight.publisher", names: "B" },
  ],
  "crédits différents → développeur puis éditeur",
);
checkEqual(
  creditLines(["A", "B"], ["B", "A"]),
  [
    { key: "spotlight.developer", names: "A, B" },
    { key: "spotlight.publisher", names: "B, A" },
  ],
  "mêmes noms dans un ordre différent → deux lignes (comparaison positionnelle assumée)",
);
checkEqual(
  creditLines(["A"], []),
  [{ key: "spotlight.developer", names: "A" }],
  "développeur seul → aucune ligne éditeur vide",
);
checkEqual(
  creditLines(undefined, ["B"]),
  [{ key: "spotlight.publisher", names: "B" }],
  "éditeur seul → une seule ligne éditeur",
);
checkEqual(creditLines(undefined, undefined), [], "aucun crédit → le template rend son fallback AppID");

export const __i18nWiringRan = true;
export const i18nWiringSuite = Promise.resolve();
export const i18nWiringCases = () => cases;

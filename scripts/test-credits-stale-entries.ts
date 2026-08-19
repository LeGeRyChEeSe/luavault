// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync } from "node:fs";
import { en } from "../src/lib/i18n/en";
import { fr } from "../src/lib/i18n/fr";

let cases = 0;

const creditsSource = readFileSync("src/lib/credits.ts", "utf8");
const dataStart = creditsSource.indexOf("const CREDIT_GROUPS_DATA = [");
const dataEnd = creditsSource.indexOf("] as const;", dataStart);
assert.ok(dataStart >= 0 && dataEnd > dataStart, "CREDITS-01: le tableau CREDIT_GROUPS_DATA doit être identifiable");
const creditGroupsData = creditsSource.slice(dataStart, dataEnd);

for (const staleId of ["verification", "steamdb"]) {
  assert.doesNotMatch(
    creditGroupsData,
    new RegExp(`\\bid:\\s*["']${staleId}["']`),
    `CREDITS-01: CREDIT_GROUPS_DATA ne doit plus réintroduire l'item obsolète ${staleId}`,
  );
  cases++;
}

assert.match(
  creditGroupsData,
  /\bid:\s*["']steamwebapi["']/,
  "CREDITS-01: CREDIT_GROUPS_DATA doit conserver l'item Steam Web API actif",
);
cases++;

for (const [locale, catalogue] of [["en", en], ["fr", fr]] as const) {
  for (const staleKey of ["credits.item.verification.role", "credits.item.steamdb.role"]) {
    assert.ok(
      !Object.hasOwn(catalogue, staleKey),
      `CREDITS-01: le catalogue ${locale} ne doit plus contenir la clé i18n obsolète ${staleKey}`,
    );
    cases++;
  }
}

const luaVaultRoleRules = [
  ["en", en["credits.item.LuaVault.role"], /\b(?:generates|distributes)\b/i, /\b(?:Discord|community)\b/i],
  ["fr", fr["credits.item.LuaVault.role"], /\b(?:génère|distribue)\b/i, /\b(?:Discord|communauté)\b/i],
] as const;

for (const [locale, role, staleWords, communityMention] of luaVaultRoleRules) {
  assert.doesNotMatch(
    role,
    staleWords,
    `CREDITS-01: le rôle LuaVault ${locale} ne doit plus prétendre générer ou distribuer des fichiers`,
  );
  assert.match(
    role,
    communityMention,
    `CREDITS-01: le rôle LuaVault ${locale} doit présenter le Discord ou la communauté d'entraide`,
  );
  cases += 2;
}

export const __creditsStaleEntriesRan = true;
export const creditsStaleEntriesSuite = Promise.resolve();
export const creditsStaleEntriesCases = () => cases;

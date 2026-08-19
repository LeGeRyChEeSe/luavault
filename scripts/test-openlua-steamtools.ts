// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync } from "node:fs";
import { en } from "../src/lib/i18n/en";
import { fr } from "../src/lib/i18n/fr";

let cases = 0;

const steamtoolsSources = [
  "src-tauri/src/install.rs",
  "src/lib/i18n/en.ts",
  "src/lib/i18n/fr.ts",
] as const;
const expectedScriptUrl = "cdn.openlua.cloud/fix-st.ps1";

for (const sourcePath of steamtoolsSources) {
  const source = readFileSync(sourcePath, "utf8");
  assert.ok(
    !source.includes("cdn.LuaVault"),
    `OPENLUA-01: ${sourcePath} ne doit pas pointer vers cdn.LuaVault`,
  );
  assert.ok(
    source.includes(expectedScriptUrl),
    `OPENLUA-01: ${sourcePath} doit pointer vers ${expectedScriptUrl}`,
  );
  cases++;
}

const creditsSource = readFileSync("src/lib/credits.ts", "utf8");
assert.match(
  creditsSource,
  /\{\s*id:\s*"openlua",\s*name:\s*"openlua\.cloud",\s*url:\s*"https:\/\/openlua\.cloud",\s*licence:\s*undefined,\s*\}/,
  "OPENLUA-01: les crédits doivent contenir l'item openlua.cloud avec son URL exacte",
);
cases++;

for (const [locale, catalogue] of [["en", en], ["fr", fr]] as const) {
  assert.ok(
    Object.hasOwn(catalogue, "credits.item.openlua.role"),
    `OPENLUA-01: le catalogue ${locale} doit contenir credits.item.openlua.role`,
  );
  cases++;
}

export const __openluaSteamtoolsRan = true;
export const openluaSteamtoolsSuite = Promise.resolve();
export const openluaSteamtoolsCases = () => cases;

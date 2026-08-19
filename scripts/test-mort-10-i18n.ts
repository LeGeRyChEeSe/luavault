// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync } from "node:fs";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";

type Catalog = "en" | "fr";

const TARGETS = ["library.empty.none.hint", "settings.data.preview.hint"] as const;
const FORBIDDEN: Readonly<Record<Catalog, RegExp>> = {
  en: /search/i,
  fr: /recherche/i,
};

function valueFor(source: string, key: string, catalog: Catalog): string {
  const match = source.match(new RegExp(`^\\s*"${key.replace(/\./g, "\\.")}":\\s*"((?:\\\\.|[^"\\\\])*)",\\s*$`, "m"));
  if (match === null) {
    throw new Error(`MORT-10 ${catalog}.${key}: targeted catalog value is missing or is no longer a string literal`);
  }
  return match[1];
}

let cases = 0;
for (const catalog of ["en", "fr"] as const) {
  const source = readFileSync(`src/lib/i18n/${catalog}.ts`, "utf8");
  for (const key of TARGETS) {
    const value = valueFor(source, key, catalog);
    assert.ok(
      !FORBIDDEN[catalog].test(value),
      `MORT-10 ${catalog}.${key}: obsolete Search-tab guidance remains in ${JSON.stringify(value)}`,
    );
    cases++;
  }
}

console.log(`  ✓ MORT-10: ${cases} targeted local-import hints avoid obsolete Search-tab guidance`);
export const __mort10I18nRan = true;
export const mort10I18nSuite = Promise.resolve();
export const mort10I18nCases = () => cases;

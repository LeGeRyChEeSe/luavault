// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync } from "node:fs";
import { en } from "../src/lib/i18n/en";
import { fr } from "../src/lib/i18n/fr";

type DefaultCapability = {
  $schema: string;
  identifier: string;
  description: string;
  windows: unknown;
  permissions: unknown;
};

let cases = 0;

const capability = JSON.parse(
  readFileSync("src-tauri/capabilities/default.json", "utf8"),
) as DefaultCapability;

if (!Array.isArray(capability.windows)) {
  throw new Error("TAURIDOCS-01: default capability.windows must be an array");
}
const windows = capability.windows;
assert.ok(
  windows.includes("main"),
  "TAURIDOCS-01: default capability.windows must retain the main window",
);
cases++;
assert.ok(
  !windows.includes("verification"),
  "TAURIDOCS-01: default capability.windows must not grant permissions to the removed verification window",
);
cases++;
assert.deepEqual(
  windows,
  ["main"],
  "TAURIDOCS-01: main must be the only window covered by the default capability",
);
cases++;
assert.equal(
  capability.description,
  "Capability for the main window",
  "TAURIDOCS-01: default capability description must not mention the removed window",
);
cases++;
assert.equal(capability.identifier, "default", "TAURIDOCS-01: capability identifier must remain unchanged");
cases++;
assert.equal(
  capability.$schema,
  "../gen/schemas/desktop-schema.json",
  "TAURIDOCS-01: capability schema must remain unchanged",
);
cases++;
assert.deepEqual(
  capability.permissions,
  ["core:default", "opener:default", "dialog:default"],
  "TAURIDOCS-01: main must retain its existing default permissions",
);
cases++;

const tauridocsRoleRules = [
  ["en", en["credits.item.tauridocs.role"], /verification token/i],
  ["fr", fr["credits.item.tauridocs.role"], /jeton verification/i],
] as const;

for (const [locale, role, staleText] of tauridocsRoleRules) {
  assert.doesNotMatch(
    role,
    staleText,
    `TAURIDOCS-01: credits.item.tauridocs.role (${locale}) must not retain the removed verification-page wording`,
  );
  cases++;
}

console.log(`  \u2713 TAURIDOCS-01: ${cases} targeted capability and Tauri-docs credit guards pass`);
export const __tauridocsCapabilityRan = true;
export const tauridocsCapabilitySuite = Promise.resolve();
export const tauridocsCapabilityCases = () => cases;

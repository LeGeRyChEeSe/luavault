// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { existsSync, readFileSync } from "node:fs";

assert.ok(!existsSync("src/components/LicenceGate.svelte"), "la porte supprimée ne peut plus piéger le focus");
assert.match(readFileSync("src/App.svelte", "utf8"), /aria-modal="true"/, "l'aide conserve son dialogue accessible");
export const __a11yWiringRan = true;

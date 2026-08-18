// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { existsSync } from "node:fs";
import { patchAppIdFromFilename } from "../src/lib/patch-import";

assert.ok(!existsSync("src/components/LicenceGate.svelte"));
assert.ok(!existsSync("src/views/SearchView.svelte"));
assert.equal(patchAppIdFromFilename("Portal 2 (440).zip"), "440");
export const charteCases = () => 3;
export const __charteRan = true;

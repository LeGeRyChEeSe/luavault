// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
import { en } from "../src/lib/i18n/en";
import { fr } from "../src/lib/i18n/fr";

let cases = 0;
assert.equal(fr["nav.library"], "Bibliothèque"); cases++;
assert.equal(en["nav.library"], "Library"); cases++;

export const __i18nStateRan = true;
export const i18nStateSuite = Promise.resolve();
export const i18nStateCases = () => cases;

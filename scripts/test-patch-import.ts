// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync } from "node:fs";
import { patchAppIdFromFilename, patchFailureFor } from "../src/lib/patch-import";
import { stripCommentsAndStrings } from "./test-dlc-wiring";

type PatchFilenameCases = {
  accepted: [string, string][];
  rejected: string[];
};

const filenameCases: PatchFilenameCases = JSON.parse(
  readFileSync("shared/patch-filename-cases.json", "utf8"),
);

let cases = 0;
for (const filename of filenameCases.rejected) {
  assert.equal(patchAppIdFromFilename(filename), null, `IMPORT-01: nom refusé ${filename}`);
  cases++;
}
for (const [filename, expected] of filenameCases.accepted) {
  assert.equal(patchAppIdFromFilename(filename), expected, `IMPORT-01: nom accepté ${filename}`);
  cases++;
}

function delimitedBlock(source: string, openIndex: number, open: string, close: string, message: string): string {
  let depth = 0;
  for (let index = openIndex; index < source.length; index++) {
    if (source[index] === open) depth++;
    if (source[index] === close && --depth === 0) return source.slice(openIndex, index + 1);
  }
  throw new Error(message);
}

function importPatchPathsBody(source: string): string {
  const signature = "async function importPatchPaths(paths: string[])";
  const start = source.indexOf(signature);
  assert.ok(start >= 0, "IMPORT-01: importPatchPaths doit exister");
  const openBrace = source.indexOf("{", start + signature.length);
  assert.ok(openBrace >= 0, "IMPORT-01: le corps de importPatchPaths doit exister");
  const body = delimitedBlock(
    source,
    openBrace,
    "{",
    "}",
    "IMPORT-01: le corps de importPatchPaths doit se fermer",
  );
  return source.slice(start, openBrace + body.length);
}

for (const [error, expected, label] of [
  [new Error("boum"), { key: "library.patch.failed.backend", params: { error: "boum" } }, "Error non vide"],
  ["boum", { key: "library.patch.failed.backend", params: { error: "boum" } }, "chaîne non vide"],
  [new Error(""), { key: "library.patch.failed", params: { names: "Archive.zip" } }, "Error vide"],
  ["", { key: "library.patch.failed", params: { names: "Archive.zip" } }, "chaîne vide"],
  [{}, { key: "library.patch.failed", params: { names: "Archive.zip" } }, "objet, null et undefined"],
] as const) {
  assert.deepEqual(patchFailureFor(error, "Archive.zip"), expected, `IMPORT-01: ${label}`);
  if (label === "objet, null et undefined") {
    assert.deepEqual(patchFailureFor(null, "Archive.zip"), expected, "IMPORT-01: null");
    assert.deepEqual(patchFailureFor(undefined, "Archive.zip"), expected, "IMPORT-01: undefined");
  }
  cases++;
}

const importPatchPaths = stripCommentsAndStrings(
  importPatchPathsBody(readFileSync("src/views/LibraryView.svelte", "utf8")),
);
const catchMatch = /catch\s*\(\s*error\s*\)/.exec(importPatchPaths);
if (!catchMatch) {
  throw new Error("IMPORT-01: importPatchPaths doit lier l'erreur du backend dans catch (error)");
}
const catchOpenBrace = importPatchPaths.indexOf("{", catchMatch.index + catchMatch[0].length);
assert.ok(catchOpenBrace >= 0, "IMPORT-01: le bloc catch (error) doit exister");
const catchBody = delimitedBlock(
  importPatchPaths,
  catchOpenBrace,
  "{",
  "}",
  "IMPORT-01: le bloc catch (error) doit se fermer",
);
assert.ok(
  /\bpatchFailureFor\s*\(\s*error\s*,/.test(catchBody),
  "IMPORT-01: catch (error) doit déléguer la décision à patchFailureFor",
);
assert.ok(
  !/[?:]/.test(catchBody),
  "IMPORT-01: catch (error) ne doit pas reprendre la décision par ternaire",
);
cases++;

const toasts = readFileSync("src/components/Toasts.svelte", "utf8");
const toastTextSpan = /<span\s+class="([^"]*\bmin-w-0\b[^"]*\bbreak-words\b[^"]*)">\s*\{toast\.text\}\s*<\/span>/.exec(toasts);
if (!toastTextSpan) {
  throw new Error("IMPORT-01: le texte visuel du toast doit rester dans son span dédié");
}
assert.ok(
  toastTextSpan[1].split(/\s+/).includes("whitespace-pre-line"),
  "IMPORT-01: le texte visuel du toast doit préserver les lignes des échecs d'import",
);
cases++;

export const __patchImportRan = true;
export const patchImportSuite = Promise.resolve();
export const patchImportCases = () => cases;

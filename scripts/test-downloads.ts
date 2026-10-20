// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync } from "node:fs";
import type { DownloadFound } from "../src/lib/api";
import { BLOCK_KEYS, mergeFound, planDownload } from "../src/lib/downloads";
import { stripComments } from "./test-dlc-wiring";

let cases = 0;
function equal(actual: unknown, expected: unknown, message: string) {
  assert.deepEqual(actual, expected, message);
  cases++;
}

const file = (over: Partial<DownloadFound>): DownloadFound => ({
  path: "C:/Users/x/Downloads/1593030.lua",
  name: "1593030.lua",
  size: 100,
  kind: "lua",
  app_id: "1593030",
  ...over,
});
const library = new Map([["440", "Team Fortress 2"]]);

// A .lua is always addable: its AppID comes from its content, not from the library.
equal(planDownload(file({}), library), { importable: true, gameName: null }, "un .lua inconnu de la bibliothèque est ajoutable");
equal(
  planDownload(file({ app_id: "440" }), library),
  { importable: true, gameName: "Team Fortress 2" },
  "un .lua d'un jeu connu affiche le nom du jeu",
);

// A patch archive needs its game in the library — the backend's own rule.
equal(
  planDownload(file({ kind: "archive", name: "440_online_fix.zip", app_id: "440" }), library),
  { importable: true, gameName: "Team Fortress 2" },
  "une archive d'un jeu connu est ajoutable",
);
equal(
  planDownload(file({ kind: "archive", name: "999_online_fix.zip", app_id: "999" }), library),
  { importable: false, reason: "archive.unknown-game" },
  "une archive d'un jeu absent de la bibliothèque est refusée avec sa raison",
);
equal(
  planDownload(file({ kind: "archive", name: "patch.zip", app_id: null }), library),
  { importable: false, reason: "archive.no-app-id" },
  "une archive sans AppID est refusée avec sa raison",
);
equal(
  Object.keys(BLOCK_KEYS).sort(),
  ["archive.no-app-id", "archive.unknown-game"],
  "chaque raison de refus a sa clé de catalogue",
);

// The pending list holds a path once, and a rewritten file replaces its description.
const first = file({ path: "a", name: "a.lua" });
const second = file({ path: "b", name: "b.lua" });
equal(mergeFound([first], [second]).map((f) => f.path), ["a", "b"], "un nouveau fichier s'ajoute à la suite");
equal(
  mergeFound([first, second], [file({ path: "a", name: "a.lua", size: 999 })]).map((f) => [f.path, f.size]),
  [["a", 999], ["b", 100]],
  "un fichier réécrit remplace sa description sans changer de place",
);
equal(mergeFound([], []), [], "rien à fusionner, rien en attente");

// Closing the popup is never a yes: Escape and the backdrop both ignore.
const modal = stripComments(readFileSync("src/components/DownloadsModal.svelte", "utf8"));
assert.ok(/e\.target === e\.currentTarget && !busy\) onignore\(\)/.test(modal), "le fond de la fenêtre ignore");
cases++;
assert.ok(/e\.key === "Escape" && !busy\) onignore\(\)/.test(modal), "Échap ignore");
cases++;
assert.ok(!/Escape[^}]*onadd/.test(modal) && !/currentTarget[^}]*onadd/.test(modal), "ni Échap ni le fond n'ajoutent");
cases++;

// « Trouver un fichier » opens a page the BINARY chooses. No address, and no site name,
// may appear in the interface code that drives it: the frontend hands over nothing.
for (const source of ["src/views/LibraryView.svelte", "src/lib/downloads-state.svelte.ts", "src/components/DownloadsModal.svelte"]) {
  const code = stripComments(readFileSync(source, "utf8"));
  assert.ok(!/https?:\/\//i.test(code), `${source} ne contient aucune adresse`);
  cases++;
  assert.ok(!/openlua/i.test(code), `${source} ne nomme aucun site`);
  cases++;
}
const state = stripComments(readFileSync("src/lib/downloads-state.svelte.ts", "utf8"));
assert.ok(/openSourceSite\(\)/.test(state), "findFile appelle openSourceSite() sans argument");
cases++;

export const __downloadsRan = true;
export const downloadsSuite = Promise.resolve();
export const downloadsCases = () => cases;

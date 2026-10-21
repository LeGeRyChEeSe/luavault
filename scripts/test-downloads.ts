// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync } from "node:fs";
import type { DownloadFound } from "../src/lib/api";
import { BLOCK_KEYS, flavorKey, importOrder, mergeFound, planDownload } from "../src/lib/downloads";
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
  flavor: "lua",
  replaces: false,
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

// A patch for a game whose .lua is ticked in the same list can be added with it: the .lua goes first.
equal(
  planDownload(file({ kind: "archive", name: "999_bypass.rar", app_id: "999", flavor: "bypass" }), library, new Set(["999"])),
  { importable: true, gameName: null },
  "un bypass dont le .lua est coché à côté est ajoutable",
);
equal(
  planDownload(file({ kind: "archive", name: "999_bypass.rar", app_id: "999", flavor: "bypass" }), library, new Set(["1"])),
  { importable: false, reason: "archive.unknown-game" },
  "le .lua d'un autre jeu n'ouvre pas la porte",
);
equal(
  importOrder([
    file({ path: "p", kind: "archive", name: "1_bypass.rar" }),
    file({ path: "l", name: "1.lua" }),
    file({ path: "q", kind: "archive", name: "2_online_fix.zip", flavor: "patch" }),
  ]).map((f) => f.path),
  ["l", "p", "q"],
  "les .lua passent avant les archives, l'ordre relatif est gardé",
);

// What a file is called on screen: a .lua, an online patch or a DRM bypass.
equal(flavorKey(file({})), "downloads.kind.lua", "un .lua");
equal(flavorKey(file({ kind: "archive", flavor: "patch" })), "downloads.kind.patch", "un patch en ligne");
equal(flavorKey(file({ kind: "archive", flavor: "bypass" })), "downloads.kind.bypass", "un bypass DRM");

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
  // The raw text, not the comment-stripped one: stripping reads the `//` of `https://`
  // as the start of a comment and would cut the very address this looks for.
  const raw = readFileSync(source, "utf8");
  assert.ok(!/https?:/i.test(raw), `${source} ne contient aucune adresse`);
  cases++;
  assert.ok(!/openlua/i.test(raw), `${source} ne nomme aucun site`);
  cases++;
}
const state = stripComments(readFileSync("src/lib/downloads-state.svelte.ts", "utf8"));
assert.ok(/openSourceSite\(\)/.test(state), "findFile appelle openSourceSite() sans argument");
cases++;

// Answering the popup (Ajouter or Ignorer) takes back the Windows notification that
// listed the files: both paths must reach the command, or a toast outlives its question.
// Ignoring never loses a file: it only closes the question; the inventory still lists it.
assert.ok(/dismiss\(\)\s*\{[\s\S]*?this\.pending = \[\];/.test(state) && !/dismiss\(\)\s*\{[\s\S]{0,400}this\.inventory = \[\]/.test(state), "dismiss() vide l'attente mais jamais l'inventaire");
cases++;
assert.ok(/importOrder\(this\.shown\.filter/.test(state), "add() travaille sur la liste affichée, les .lua d'abord");
cases++;
assert.ok(/window\.addEventListener\("focus"/.test(state) && /refreshInventory\(\)/.test(state), "l'inventaire se rafraîchit au retour dans la fenêtre");
cases++;
const answerPaths = /dismiss\(\)\s*\{[\s\S]*?this\.pending = \[\];\s*this\.answered\(\)/.test(state) && /async add\([^)]*\)[^{]*\{[\s\S]*?this\.answered\(\)/.test(state);
assert.ok(answerPaths, "dismiss() et add() préviennent le backend que la question est réglée");
cases++;
assert.ok(/downloadsPendingClear\(\)/.test(state), "answered() appelle downloadsPendingClear");
cases++;

export const __downloadsRan = true;
export const downloadsSuite = Promise.resolve();
export const downloadsCases = () => cases;

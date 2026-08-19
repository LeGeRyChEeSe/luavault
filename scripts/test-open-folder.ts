// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync, readdirSync } from "node:fs";
import { openFolder, openSteamtoolsFolder } from "../src/lib/open-folder";
import { stripComments } from "./test-dlc-wiring";

let cases = 0;
const path = "C:\\Program Files (x86)\\Steam";

{
  const originalWindow = (globalThis as { window?: unknown }).window;
  let command: string | null = null;
  let args: unknown = null;
  (globalThis as { window?: unknown }).window = {
    __TAURI_INTERNALS__: {
      invoke: async (receivedCommand: string, receivedArgs: unknown) => {
        command = receivedCommand;
        args = receivedArgs;
      },
    },
  };

  try {
    await openFolder(path);
  } finally {
    if (originalWindow === undefined) {
      delete (globalThis as { window?: unknown }).window;
    } else {
      (globalThis as { window?: unknown }).window = originalWindow;
    }
  }

  assert.equal(
    command,
    "plugin:opener|reveal_item_in_dir",
    "STG-01-fix01: le défaut de openFolder doit être revealItemInDir, pas openPath",
  );
  assert.deepEqual(args, { paths: [path] }, "STG-01-fix01: le défaut revealItemInDir reçoit le dossier demandé");
  cases++;
}

{
  let revealed: string | null = null;
  await openFolder(path, async (value) => {
    revealed = value;
  });
  assert.equal(revealed, path, "STG-01: openFolder délègue au revealer explicitement fourni");
  cases++;
}

{
  let opened: string | null = null;
  const toasts: [string, string][] = [];
  await openSteamtoolsFolder(
    path,
    "Impossible d'ouvrir le dossier SteamTools.",
    (kind, message) => toasts.push([kind, message]),
    async (value) => {
      opened = value;
    },
  );
  assert.equal(opened, path, "STG-01: le clic SteamTools emploie openFolder");
  assert.deepEqual(toasts, [], "STG-01: aucune erreur n'est signalée après une ouverture réussie");
  cases++;
}

{
  const toasts: [string, string][] = [];
  await openSteamtoolsFolder(
    path,
    "Impossible d'ouvrir le dossier SteamTools.",
    (kind, message) => toasts.push([kind, message]),
    async () => {
      throw new Error("Explorer indisponible");
    },
  );
  assert.deepEqual(
    toasts,
    [["error", "Impossible d'ouvrir le dossier SteamTools."]],
    "STG-01: un rejet d'ouverture SteamTools devient un toast d'erreur visible",
  );
  cases++;
}

{
  const source = readFileSync("src/views/SettingsView.svelte", "utf8");
  const handlerStart = source.indexOf("function openSteamtoolsDir()");
  assert.ok(handlerStart >= 0, "STG-01: le gestionnaire du bouton SteamTools doit exister");
  const handlerEnd = source.indexOf("\n  // --------------------------------------------------------- hidden games", handlerStart);
  assert.ok(handlerEnd >= 0, "STG-01: le gestionnaire SteamTools doit être délimité");
  const handler = source.slice(handlerStart, handlerEnd);
  assert.match(handler, /\bopenSteamtoolsFolder\s*\(/, "STG-01: le gestionnaire délègue à la porte testée");
  assert.match(
    handler,
    /appState\.toast\s*\(\s*kind\s*,\s*message\s*\)/,
    "STG-01: le rejet de la porte testée reste relié au vrai toast applicatif",
  );
  const tip = 'tip={t("settings.folders.steamtools.tip")}';
  const buttonStart = source.lastIndexOf("<ActionButton", source.indexOf(tip));
  assert.ok(buttonStart >= 0, "STG-01: le bouton SteamTools doit rester identifiable par son infobulle");
  assert.match(
    source.slice(buttonStart, source.indexOf(tip)),
    /onclick=\{openSteamtoolsDir\}/,
    "STG-01: le bouton SteamTools appelle le gestionnaire qui délègue à openFolder",
  );
  cases++;
}

function sourceFiles(directory: string): string[] {
  return (readdirSync(directory, { withFileTypes: true }) as { name: string; isDirectory(): boolean }[]).flatMap((entry) => {
    const entryPath = `${directory}/${entry.name}`;
    return entry.isDirectory()
      ? sourceFiles(entryPath)
      : entryPath.endsWith(".svelte") || entryPath.endsWith(".ts")
        ? [entryPath]
        : [];
  });
}

function forbiddenOpenerImports(source: string): string[] {
  const importedNames: string[] = [];
  const imports = stripComments(source).matchAll(
    /^\s*import\s*(?:type\s*)?\{([^}]*)\}\s*from\s*["']@tauri-apps\/plugin-opener["']\s*;?/gm,
  );
  for (const imported of imports) {
    for (const specifier of imported[1].split(",")) {
      const name = specifier.trim().split(/\s+as\s+/)[0];
      if (name === "openPath" || name === "revealItemInDir") importedNames.push(name);
    }
  }
  return importedNames;
}

{
  const offenders = sourceFiles("src")
    .filter((file) => file !== "src/lib/open-folder.ts")
    .flatMap((file) => forbiddenOpenerImports(readFileSync(file, "utf8")).map((name) => `${file} (${name})`));
  assert.deepEqual(
    offenders,
    [],
    "STG-01-fix01: chaque ouverture de dossier doit passer par src/lib/open-folder.ts; import direct fautif",
  );
  cases++;
}

export const __openFolderRan = true;
export const openFolderSuite = Promise.resolve();
export const openFolderCases = () => cases;

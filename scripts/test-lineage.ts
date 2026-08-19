// @ts-nocheck
import { readFileSync, readdirSync, statSync } from "fs";
import { join } from "path";

const forbidden = [
  "Turnstile",
  "X-Client-Data",
  "AutoSteamTools",
  "autosteamtools",
  "astrelease",
  ".astbak",
  "ASTBCK",
  "AST-HMAC",
  "ast-veil",
  "ast-spotlight",
  "AST_API_BASE",
  "AST_UPDATE_BASE",
  "fix_available",
];

const ignorePatterns = [
  "node_modules",
  "src-tauri/target",
  "dist",
  ".git",
  ".orchestration",
  ".claude",
  ".pi",
  "AGENTS.md",
  "CLAUDE.md",
  "QWEN.md",
  "PLAN_IMPLEMENTATION.md",
  "IDEES_AMELIORATIONS.md",
];

function getFiles(dir: string, filesList: string[] = []) {
  const files = readdirSync(dir);
  for (const file of files) {
    const fullPath = join(dir, file);
    if (ignorePatterns.some(p => fullPath.replace(/\\/g, "/").includes(p))) {
      continue;
    }
    if (fullPath.replace(/\\/g, "/").endsWith("scripts/test-lineage.ts")) continue;
    if (fullPath.endsWith(".lock") || fullPath.endsWith("package-lock.json")) continue;
    
    if (statSync(fullPath).isDirectory()) {
      getFiles(fullPath, filesList);
    } else {
      filesList.push(fullPath);
    }
  }
  return filesList;
}

const files = getFiles(".");

let failed = false;

function fail(message: string) {
  console.error(`\x1b[31mL'audit de lignage a échoué :\x1b[0m ${message}`);
  failed = true;
}

const discordInvite = readFileSync("src/lib/credits.ts", "utf8");
if (!discordInvite.includes('DISCORD_INVITE = "https://discord.gg/vSczZGT7aQ"')) {
  fail("le lien Discord doit être https://discord.gg/vSczZGT7aQ dans src/lib/credits.ts.");
}
if (discordInvite.includes('"https://LuaVault/"')) {
  fail("le lien de l'entrée LuaVault dans credits.ts ne doit plus pointer vers le placeholder https://LuaVault/.");
}

const sidebar = readFileSync("src/App.svelte", "utf8");
if (!sidebar.includes(">LV<")) {
  fail("le badge de la barre latérale doit afficher LV dans src/App.svelte.");
}

const iconGenerator = readFileSync("scripts/make-icon.ps1", "utf8");
if (!iconGenerator.includes("'LV'")) {
  fail("le générateur d'icône doit dessiner LV dans scripts/make-icon.ps1.");
}

const onboarding = readFileSync("src/components/Onboarding.svelte", "utf8");
if (!/\bLV\b/.test(onboarding)) {
  fail("le badge d'onboarding doit afficher LV dans src/components/Onboarding.svelte.");
}

for (const file of ["README.md", "src/views/CreditsView.svelte"]) {
  const content = readFileSync(file, "utf8").toLocaleLowerCase();
  for (const phrase of ["projet éducatif", "secret", "dlc"]) {
    if (content.includes(phrase)) {
      fail(`la formulation interdite \"${phrase}\" a été trouvée dans ${file}.`);
    }
  }
}

for (const file of files) {
  try {
    const content = readFileSync(file, "utf8");
    for (const word of forbidden) {
      if (content.includes(word)) {
        fail(`Le mot clé interdit "${word}" a été trouvé dans ${file}.`);
      }
    }
  } catch (e) {
    // Ignore unreadable or binary files if they throw
  }
}

if (failed) {
  process.exit(1);
} else {
  console.log("\x1b[32mOK: Audit de lignage (aucune trace privée détectée)\x1b[0m");
}

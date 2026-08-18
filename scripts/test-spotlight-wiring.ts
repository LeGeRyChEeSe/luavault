/**
 * Structural tripwires for the restored game spotlight.
 *
 * The project has no DOM runner for this modal, so these checks pin the
 * production wiring that makes a card click observable to the user.
 */

// @ts-expect-error — Node types are deliberately not a frontend dependency.
import { readFileSync as readFileSyncRaw } from "node:fs";
import { displayCorrection, spotlightTitle } from "../src/lib/spotlight";
import { stripComments, stripCommentsAndStrings } from "./test-dlc-wiring";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

function assertOk(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

function blockBody(source: string, start: number): string | null {
  const open = source.indexOf("{", start);
  if (open < 0) return null;
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    if (source[i] === "}" && --depth === 0) return source.slice(open + 1, i);
  }
  return null;
}

const app = stripComments(readFileSync("src/App.svelte", { encoding: "utf8" }));
const spotlightRaw = readFileSync("src/components/GameSpotlight.svelte", { encoding: "utf8" });
const spotlight = stripCommentsAndStrings(spotlightRaw);
const spotlightCode = stripComments(spotlightRaw);
const cardClass = spotlightRaw.indexOf("lv-spotlight-card");
const cardStart = spotlightRaw.lastIndexOf("<div", cardClass);
let cardEnd = spotlightRaw.indexOf(">", cardClass);
while (cardEnd >= 0 && spotlightRaw[cardEnd - 1] === "=") {
  cardEnd = spotlightRaw.indexOf(">", cardEnd + 1);
}
const cardTag = cardStart >= 0 && cardEnd > cardStart ? spotlightRaw.slice(cardStart, cardEnd + 1) : "";

assertOk(
  /import\s+GameSpotlight\s+from\s+["']\.\/components\/GameSpotlight\.svelte["']/.test(app) &&
    /<GameSpotlight\s*\/>/.test(app),
  "SP1: App.svelte doit monter GameSpotlight pour rendre le clic sur une carte visible",
);

assertOk(
  cardTag.includes("use:focusTrap") &&
    cardTag.includes('role="dialog"') &&
    cardTag.includes('tabindex="-1"') &&
    cardTag.includes('aria-modal="true"') &&
    cardTag.includes('aria-labelledby="spotlight-title"') &&
    /aria-hidden=\{openShot\s*!==\s*null\s*\?\s*"true"\s*:\s*undefined\}/.test(cardTag),
  "SP2: la carte spotlight conserve son dialogue accessible et son focus trap sur le même nœud",
);

assertOk(
  /\$effect\(\(\)\s*=>\s*\{\s*return\s*\(\)\s*=>\s*clearTimeout\(closeTimer\);\s*\}\);/.test(spotlightCode) &&
    /const\s+cur\s*=\s*current;\s*shotIndex\s*=\s*null;\s*if\s*\(!cur\)\s*return;/.test(spotlightCode) &&
    /event\.key\s*===\s*"Escape"\s*\)\s*\{\s*event\.preventDefault\(\);\s*event\.stopPropagation\(\);\s*shotIndex\s*=\s*null;/.test(spotlightCode) &&
    /event\.key\s*===\s*"ArrowRight"\s*\)\s*\{\s*event\.preventDefault\(\);\s*event\.stopPropagation\(\);\s*stepShot\(1\);/.test(spotlightCode) &&
    /event\.key\s*===\s*"ArrowLeft"\s*\)\s*\{\s*event\.preventDefault\(\);\s*event\.stopPropagation\(\);\s*stepShot\(-1\);/.test(spotlightCode) &&
    /onclick=\{\(e\)\s*=>\s*\{\s*e\.stopPropagation\(\);\s*shotIndex\s*=\s*null;\s*\}\}/.test(spotlightCode),
  "SP5: la visionneuse isole ses raccourcis, se réinitialise entre jeux et nettoie son timer",
);

for (const binding of [
  "downloadLua",
  "getLuaDlcReport",
  "getLuaLocalDlcIds",
  "cacheDlcReport",
  "getCachedDlcReport",
]) {
  assertOk(
    !new RegExp(`\\b${binding}\\b`).test(spotlight),
    `SP3: GameSpotlight ne doit pas appeler l'ancien binding ${binding}`,
  );
}

assertOk(
  /getSteamDetails\(\s*cur\.appId\s*,\s*lang\s*\)/.test(spotlight) &&
    /details\.screenshots/.test(spotlight) &&
    /details\.changelog/.test(spotlight) &&
    /<GameActions\s+\{status\}\s+onAfterAction=\{afterAction\}\s*\/>/.test(spotlight),
  "SP4: les métadonnées Steam, galerie, changelog et actions locales restent câblés",
);

assertOk(
  spotlightTitle("AppID 1593030", "Half-Life 2") === "Half-Life 2" &&
    spotlightTitle("AppID 1593030", undefined) === "AppID 1593030" &&
    spotlightTitle("AppID 1593030", null) === "AppID 1593030",
  "SP6: spotlightTitle doit préférer le nom Steam et conserver le fallback local sans détails",
);

assertOk(
  displayCorrection({ name: "AppID 1593030", icon: null }, { name: "Half-Life 2", header_image: null })?.name === "Half-Life 2" &&
    displayCorrection({ name: "Half-Life 2", icon: "https://old.jpg" }, { name: "Half-Life 2", header_image: "https://new.jpg" })?.icon === "https://new.jpg" &&
    displayCorrection({ name: "Half-Life 2", icon: "https://same.jpg" }, { name: "Half-Life 2", header_image: "https://same.jpg" }) === null &&
    displayCorrection({ name: "AppID 1593030", icon: null }, null) === null,
  "SP8: displayCorrection ne persiste que les métadonnées Steam nouvelles et conserve le silence sans détails",
);

const detailsFetch = spotlight.indexOf("getSteamDetails(cur.appId, lang)");
const detailsThen = detailsFetch < 0 ? -1 : spotlight.indexOf(".then((result) =>", detailsFetch);
const detailsThenBody = detailsThen < 0 ? null : blockBody(spotlight, detailsThen);
const assignment = detailsThenBody?.indexOf("details = result") ?? -1;
const correction = detailsThenBody?.indexOf("const correction = displayCorrection(cur, result)") ?? -1;
const condition = detailsThenBody?.indexOf("if (correction)") ?? -1;
const persist = detailsThenBody?.indexOf("setLibraryDisplay(cur.appId, correction.name, correction.icon)") ?? -1;
const persistCalls = detailsThenBody === null
  ? 0
  : (detailsThenBody.match(/\bsetLibraryDisplay\s*\(/g) ?? []).length;
assertOk(
  detailsThenBody !== null && assignment >= 0 && correction > assignment && condition > correction && persist > condition,
  "SP9: la correction de bibliothèque suit details = result, reste dans .then() et dépend de displayCorrection",
);
assertOk(
  persistCalls === 1,
  "SP9: exclusivité de l'écriture : le .then() des détails Steam contient exactement un appel à setLibraryDisplay",
);

const refresh = detailsThenBody?.indexOf("appState.refreshLibrary()") ?? -1;
assertOk(
  detailsThenBody !== null && refresh > persist,
  "SP10: appState.refreshLibrary() doit suivre setLibraryDisplay dans le .then() des détails Steam",
);

const titleMarkup = /<h2\b[^>]*>\s*\{([\s\S]*?)\}\s*<\/h2>/.exec(spotlight);
assertOk(
  titleMarkup?.[1].trim() === "spotlightTitle(game.name, details?.name)",
  "SP7: le titre doit déléguer à spotlightTitle(game.name, details?.name), sans game.name nu ni ternaire local",
);

assertOk(
  /details\?\.metacritic\s*!=\s*null/.test(spotlight),
  "SP11: le bloc Metacritic doit comparer explicitement à null, pas tester une vérité JS (0 est un score valide)",
);

assertOk(
  /t\(\s*["']spotlight\.metacritic["']\s*,\s*\{\s*score:\s*details\.metacritic\s*\}\s*\)/.test(spotlightCode),
  "SP12: la pastille Metacritic doit passer le score par t(\"spotlight.metacritic\", { score: … })",
);

console.log("  ✓ tripwires structurels GameSpotlight (montage, a11y, isolation, retrait API, contenu conservé)");

/**
 * Structural guard rails for the artwork cache (LOT-14).
 *
 * Same contract as test-playtime-wiring.ts: these are NOT behaviour tests.
 * The Rust invariants of the cache (hashed names, capped reads, atomic
 * writes, LRU purge, failures never cached) are pinned by the unit tests in
 * artwork.rs — each one goes red when the invariant it protects is removed.
 * The guards here pin the WIRING the unit tests cannot see: the CDN <img>
 * tags replaced by the cached/fallback component, the onerror fallback,
 * the asset-protocol plumbing (scope, CSP), the commands the frontend
 * calls, and the CDN client's scheme/redirect gates (A7).
 *
 * Every pattern runs on a STRIPPED copy of the source
 * (stripCommentsAndStrings / stripComments from test-dlc-wiring.ts), so a
 * commented-out call or a tooltip quoting the markup never satisfies a
 * guard. Guards that need a string literal (the invoke command names, the
 * CSP directives) run on the stripComments copy — strings kept, comments
 * gone — exactly the P4 precedent.
 *
 * Deliberately loose on shape: each assertion goes red on a
 * behaviour-breaking revert, not on a rename or a reformat.
 */

// @ts-expect-error — `node:fs` has no types here: @types/node is not a
// project dependency. See the identical note in test-playtime-wiring.ts.
import { readFileSync as readFileSyncRaw } from "node:fs";
import { stripComments, stripCommentsAndStrings } from "./test-dlc-wiring";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

function assertOk(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

// ── Read and strip the sources ────────────────────────────────
const artworkComponent = stripCommentsAndStrings(
  readFileSync("src/components/Artwork.svelte", { encoding: "utf8" }),
);
// Strings kept: the fallback icon name IS a string literal.
const artworkComponentStr = stripComments(
  readFileSync("src/components/Artwork.svelte", { encoding: "utf8" }),
);
const artworkLib = stripCommentsAndStrings(
  readFileSync("src/lib/artwork.ts", { encoding: "utf8" }),
);
// Strings kept: a `data:` URI can only ever live inside a string literal,
// so the guard that hunts them must NOT run on the strings-stripped copy.
const artworkLibStr = stripComments(
  readFileSync("src/lib/artwork.ts", { encoding: "utf8" }),
);
const gameCard = stripCommentsAndStrings(
  readFileSync("src/views/GameCard.svelte", { encoding: "utf8" }),
);
const spotlight = stripCommentsAndStrings(
  readFileSync("src/components/GameSpotlight.svelte", { encoding: "utf8" }),
);
const settingsView = stripCommentsAndStrings(
  readFileSync("src/views/SettingsView.svelte", { encoding: "utf8" }),
);
// Strings kept: the invoke command names are string literals.
const apiStr = stripComments(readFileSync("src/lib/api.ts", { encoding: "utf8" }));
// tauri.conf.json read RAW: JSON has no comments, so there is no dead code
// to strip — and stripComments would eat the CSP after the first `//` of
// http://asset.localhost, blinding the very guard it should serve.
const confRaw = readFileSync("src-tauri/tauri.conf.json", { encoding: "utf8" });
const libRs = stripCommentsAndStrings(
  readFileSync("src-tauri/src/lib.rs", { encoding: "utf8" }),
);
const artworkRs = stripCommentsAndStrings(
  readFileSync("src-tauri/src/artwork.rs", { encoding: "utf8" }),
);
// Cargo.toml's comments are #-style, which neither shared stripper knows:
// strip them here so a comment quoting the feature can't keep A6-4 green.
const cargoToml = readFileSync("src-tauri/Cargo.toml", { encoding: "utf8" }).replace(
  /#[^\n]*/g,
  "",
);

// ── A1 — the Artwork component: onerror fallback + the neutral tile ──
// Correct: restyling the image, reordering the branches.
// Broken: removing onerror — a dead asset file would then show the
// browser's broken frame instead of the tile.
assertOk(
  /<img[^<>]*onerror=/.test(artworkComponent),
  "A1-1: l'<img> d'Artwork.svelte porte un onerror (repli sur la tuile si le fichier asset est illisible)",
);
// The fallback look is THE neutral tile — Icon name="gamepad" — not a
// second invented appearance. On the stripComments copy: the name is a
// string literal, so strings must survive.
assertOk(
  /<Icon[^<>]*name="gamepad"/.test(artworkComponentStr),
  'A1-2: le repli d\'Artwork.svelte est la tuile neutre existante (<Icon name="gamepad">)',
);

// ── A2 — every CDN image goes through the component ──────────────
// Pins the WIRING expression, not the co-presence of two identifiers:
// <Artwork> must receive the very URL that used to feed <img src>.
// Correct: reformatting the tag, adding attributes.
// Broken: reverting one image to a bare <img src={…}> with no fallback.
assertOk(
  /<Artwork[^<>]*url=\{\s*status\.icon\s*\}/.test(gameCard),
  "A2-1: GameCard rend l'icône via <Artwork url={status.icon}>",
);
assertOk(
  /<Artwork[^<>]*url=\{\s*details\?\.header_image\s*\?\?\s*game\.icon\s*\}/.test(spotlight),
  "A2-2: GameSpotlight rend la jaquette via <Artwork url={details?.header_image ?? game.icon}>",
);
// Loose on which field of the shot is passed: the strip shows `shot.thumbnail`
// since screenshots became a {thumbnail, full} pair, and the guard must not
// redden on that — what it protects is that the strip goes through Artwork
// (cache + fallback tile), not the shape of the expression inside.
assertOk(
  /<Artwork[^<>]*url=\{\s*shot(\.\w+)?\s*\}/.test(spotlight),
  "A2-3: GameSpotlight rend les captures d'écran via <Artwork url={shot…}>",
);
assertOk(
  /<Artwork[^<>]*url=\{\s*game\.icon\s*\}/.test(settingsView),
  "A2-4: SettingsView rend les icônes des jeux masqués via <Artwork url={game.icon}>",
);
// And no CDN image survives as a bare <img src> in those views: the
// fallback must not be bypassable by re-adding one.
for (const [name, source] of [
  ["GameCard.svelte", gameCard],
  ["GameSpotlight.svelte", spotlight],
  ["SettingsView.svelte", settingsView],
] as const) {
  assertOk(
    !/<img[\s\S]*?>/.test(source),
    `A2-8: ${name} ne contient plus de <img> directe — toutes les images CDN passent par Artwork`,
  );
}

// ── A3 — resolution: cache first, no network offline, no data: ───
// Correct: renaming the helper, adding logging.
// Broken: fetching before the cache lookup, dropping the offline gate,
// or moving images back into IPC payloads (data: URIs).
// Pins the EXPRESSION ORDER, not the co-presence of two identifiers: the
// old indexOf() comparison read the import line and never the body, so
// inverting the calls left it green. Here: `await artworkCached(...)`, then
// the offline gate, then `await artworkFetch(...)` — in that order.
assertOk(
  /await\s+artworkCached\([\s\S]*?if\s*\(\s*!\s*appState\.online\s*\)\s*return\s+null\s*;[\s\S]*?await\s+artworkFetch\(/.test(
    artworkLib,
  ),
  "A3-1: artwork.ts consulte le cache (await artworkCached), coupe le réseau hors ligne, et n'appelle artworkFetch qu'ensuite",
);
assertOk(
  /if\s*\(\s*!\s*appState\.online\s*\)/.test(artworkLib),
  "A3-2: artwork.ts n'émet aucune requête hors ligne (garde sur appState.online)",
);
assertOk(
  /convertFileSrc\(/.test(artworkLib),
  "A3-3: artwork.ts sert les fichiers via le protocole asset (convertFileSrc), pas par IPC",
);
// On the stripComments copy: a real `data:` URI lives inside a string
// literal, which stripCommentsAndStrings has ALREADY erased — the old
// guard could never go red for what it claimed to protect (the file's own
// header states the rule four lines up).
assertOk(
  !artworkLibStr.includes("data:image"),
  "A3-4: aucune image en data: — le cache passe par le protocole asset, jamais par l'IPC",
);

// ── A4 — the frontend commands exist with the right names ────────
// On the stripComments copy: the invoke names are string literals.
assertOk(
  /artworkCached[\s\S]{0,120}?"artwork_cached"/.test(apiStr),
  'A4-1: api.ts expose artworkCached → invoke("artwork_cached")',
);
assertOk(
  /artworkFetch[\s\S]{0,120}?"artwork_fetch"/.test(apiStr),
  'A4-2: api.ts expose artworkFetch → invoke("artwork_fetch")',
);
assertOk(
  /artworkCacheInfo[\s\S]{0,120}?"artwork_cache_info"/.test(apiStr),
  'A4-3: api.ts expose artworkCacheInfo → invoke("artwork_cache_info")',
);
assertOk(
  /artworkCacheClear[\s\S]{0,120}?"artwork_cache_clear"/.test(apiStr),
  'A4-4: api.ts expose artworkCacheClear → invoke("artwork_cache_clear")',
);

// ── A5 — the settings show the size and clear through ConfirmButton ──
// Correct: restyling the section.
// Broken: removing the size display, or the destructive action losing
// its two-step confirmation.
assertOk(
  /artworkCache\.bytes/.test(settingsView) && /formatBytes\(/.test(settingsView),
  "A5-1: SettingsView affiche la taille du cache (artworkCache.bytes via formatBytes)",
);
assertOk(
  /<ConfirmButton[^<>]*onconfirm=\{\s*clearArtworkCache\s*\}/.test(settingsView),
  "A5-2: le vidage du cache passe par ConfirmButton (action destructive confirmée)",
);
assertOk(
  /artworkCacheClear\(/.test(settingsView),
  "A5-3: SettingsView appelle artworkCacheClear",
);

// ── A6 — the asset protocol is actually wired ────────────────────
// Correct: reordering the CSP directives inside img-src.
// Broken: widening nothing else but dropping asset: (every cached image
// 404s), disabling the protocol, or granting the scope statically.
assertOk(
  /img-src[^";]*asset:\s*http:\/\/asset\.localhost/.test(confRaw),
  "A6-1: la CSP autorise le protocole asset pour les images (img-src … asset: http://asset.localhost)",
);
assertOk(
  /"assetProtocol"[\s\S]{0,80}?"enable"\s*:\s*true/.test(confRaw),
  "A6-2: tauri.conf.json active le protocole asset (assetProtocol.enable)",
);
// Pins the FOLDER and the non-recursive flag, not just the call: the old
// pattern stopped at the opening parenthesis, so granting the webview a
// recursive read of C:\ left it green. Exactly one folder, not recursive —
// the webview renders third-party content.
assertOk(
  /asset_protocol_scope\(\)[\s\S]{0,160}?allow_directory\(\s*&artwork_dir\s*,\s*false\s*\)/.test(
    libRs,
  ),
  "A6-3: lib.rs accorde le scope asset à l'exécution sur LE dossier de cache seulement, non récursif (allow_directory(&artwork_dir, false))",
);
assertOk(
  cargoToml.includes("protocol-asset"),
  "A6-4: Cargo.toml active la fonctionnalité protocol-asset (liée à assetProtocol.enable par tauri-build)",
);
// LOT-14 touched img-src ONLY. Every other directive must stay VERBATIM —
// widening script-src or connect-src to the asset origin would hand it
// code execution / request rights, not image loading. A6-1 alone only ever
// saw what was ADDED to img-src; this pins everything that must NOT move.
{
  const cspMatch = confRaw.match(/"csp"\s*:\s*"([^"]*)"/);
  assertOk(cspMatch !== null, "A6-5: la CSP est déclarée dans tauri.conf.json");
  const csp = cspMatch === null ? "" : cspMatch[1];
  const directives: Record<string, string> = {};
  for (const d of csp.split("; ")) {
    const sep = d.indexOf(" ");
    if (sep > 0) directives[d.slice(0, sep)] = d;
  }
  assertOk(
    directives["default-src"] === "default-src 'self'",
    "A6-5: la CSP n'a pas bougé hors img-src (default-src)",
  );
  assertOk(
    directives["script-src"] === "script-src 'self'",
    "A6-5: la CSP n'a pas bougé hors img-src (script-src)",
  );
  assertOk(
    directives["style-src"] === "style-src 'self' 'unsafe-inline'",
    "A6-5: la CSP n'a pas bougé hors img-src (style-src)",
  );
  assertOk(
    directives["connect-src"] ===
      "connect-src ipc: http://ipc.localhost https://api.github.com https://raw.githubusercontent.com",
    "A6-5: la CSP n'a pas bougé hors img-src (connect-src)",
  );
  assertOk(
    Object.keys(directives).sort().join(",") ===
      "connect-src,default-src,img-src,script-src,style-src",
    "A6-5: aucune directive CSP ajoutée (seul img-src a été touché par LOT-14)",
  );
  // img-src legitimately changed: its pre-LOT-14 sources stay, the asset
  // source joins. Inner order may move (see the A6 header).
  assertOk(
    (directives["img-src"] ?? "").includes("'self' https: data:") &&
      /asset:\s*http:\/\/asset\.localhost/.test(directives["img-src"] ?? ""),
    "A6-5: img-src garde ses sources d'origine ET la source asset",
  );
}

// ── A7 — the CDN client: https only, redirects never followed ────
// The Rust unit tests exercise ensure_https and fabricated statuses; these
// guards pin the WIRING they cannot see: that download() actually calls the
// gate, and that the client is built with Policy::none. Removing either
// silently reopens the SSRF — a redirect walking the https gate around into
// http:// or a private address.
assertOk(
  /fn download\([\s\S]{0,400}?ensure_https\(url\)/.test(artworkRs),
  "A7-1: artwork.rs appelle ensure_https avant d'émettre la requête (la garde https est branchée sur download)",
);
assertOk(
  /redirect::Policy::none\(\)/.test(artworkRs),
  "A7-2: le client artwork ne suit AUCUNE redirection (redirect::Policy::none) — un saut vers http:// ou une adresse privée est refusé, pas suivi",
);

// ── The two strip passes behave the way these guards rely on ─────
// The guards above stand on stripCommentsAndStrings / stripComments:
// verify the exact constructs they depend on, on both passes.
{
  const s = stripCommentsAndStrings;
  assertOk(
    s("<Artwork\n  url={status.icon}\n/>").includes("url={status.icon}"),
    "strip: une expression d'attribut Svelte survit",
  );
  assertOk(
    !s("<!-- <Artwork url={status.icon} /> -->").includes("Artwork"),
    "strip: un <Artwork> en commentaire HTML ne satisfait aucune garde",
  );
  assertOk(
    !s('// artworkFetch("x")\nartworkCached("x")').includes("artworkFetch"),
    "strip: un appel commenté disparaît",
  );
  const c = stripComments;
  assertOk(
    c('invoke<string>("artwork_fetch", { url })').includes('"artwork_fetch"'),
    "stripComments: les chaînes (noms de commandes) survivent",
  );
  assertOk(
    !c('<!-- invoke<string>("artwork_fetch") -->').includes("artwork_fetch"),
    "stripComments: un invoke en commentaire HTML disparaît",
  );
  console.log("  ✓ stripCommentsAndStrings / stripComments (syntaxes utilisées par ces gardes)");
}

console.log(
  "  ✓ tripwires structurels Artwork (Artwork.svelte + vues + api.ts + protocole asset + client CDN : A1–A7)",
);

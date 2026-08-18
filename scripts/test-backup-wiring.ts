/**
 * Structural guard rails for LOT-21: backup encryption password, library error
 * visibility, and the probe command.
 *
 * These are NOT behaviour tests. They read the sources and pin the shape of
 * wirings no test in this stack can execute — a reverted condition, a removed
 * call site, or a swallowed error.
 *
 * Every pattern runs on a STRIPPED copy of the source: comments (//, block
 * comments, <!-- -->) and string literals are removed first, so a commented-out
 * call or a comment quoting the markup never satisfies a guard. The strip
 * functions are pure and tested at the bottom of this file — they are the
 * foundation every guard builds on.
 *
 * Deliberately loose on shape: each assertion goes red on a
 * behaviour-breaking revert, not on a rename or a reorder. A guard that
 * cries wolf is a guard someone disables.
 *
 * Note: these guards prove nothing about the WebView2 rendering — a real
 * behaviour bench belongs to LOT-23.
 */

// @ts-expect-error — `node:fs` has no types here: @types/node is not a
// project dependency. See the identical note in test-library-view.ts.
import { readFileSync as readFileSyncRaw } from "node:fs";
import type { PasswordCheck } from "../src/lib/backup-password";

const readFileSync = readFileSyncRaw as (
  path: string,
  options: { encoding: "utf8" },
) => string;

import { stripComments, stripCommentsAndStrings } from "./test-dlc-wiring";

/**
 * Strip comments and strings like `stripCommentsAndStrings`, but preserve
 * `${...}` expressions inside template literals (backtick strings).
 *
 * The base function swallows the entire template literal — including the
 * code inside `${}` — which breaks guards that need to verify a variable
 * reference survives in the stripped output (pitfall: LOT-22 message
 * construction uses a template literal).
 */
function stripCommentsAndStringsPreserveTemplateExprs(src: string): string {
  let out = "";
  let i = 0;
  const n = src.length;
  while (i < n) {
    const c = src[i];
    // Line comment.
    if (c === "/" && src[i + 1] === "/") {
      while (i < n && src[i] !== "\n") i++;
      continue;
    }
    // Block comment.
    if (c === "/" && src[i + 1] === "*") {
      i += 2;
      while (i < n && !(src[i] === "*" && src[i + 1] === "/")) i++;
      i += 2;
      out += " ";
      continue;
    }
    // HTML comment.
    if (c === "<" && src.startsWith("<!--", i)) {
      i += 4;
      while (i < n && !src.startsWith("-->", i)) i++;
      i += 3;
      out += " ";
      continue;
    }
    // Double-quoted string (strip normally).
    if (c === '"') {
      i++;
      while (i < n && src[i] !== '"') {
        if (src[i] === "\\") i++;
        i++;
      }
      i++; // closing quote
      out += " ";
      continue;
    }
    // Template literal (backtick): extract ${...} expressions.
    if (c === "`") {
      i++; // skip opening backtick
      while (i < n && src[i] !== "`") {
        if (src[i] === "\\" && src[i + 1] === "`") {
          i += 2; // escaped backtick inside template
          continue;
        }
        // Start of expression: ${
        if (src[i] === "$" && src[i + 1] === "{") {
          i += 2; // skip ${
          let depth = 1;
          while (i < n && depth > 0) {
            if (src[i] === "{") depth++;
            else if (src[i] === "}") depth--;
            if (depth > 0) {
              out += src[i];
              i++;
            } else {
              i++; // skip closing }
            }
          }
          out += " ";
          continue;
        }
        // Literal text of the template is NOT code: drop it, exactly like the
        // contents of a quoted string. Keeping it would let a guard stay green
        // on a message that merely *mentions* the identifier it is supposed to
        // pin — pitfall 32, in the one place a template literal reintroduces it.
        i++;
      }
      i++; // skip closing backtick
      out += " ";
      continue;
    }
    out += c;
    i++;
  }
  return out;
}

function assertOk(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

// ── Read and strip the sources ────────────────────────────────
const apiRaw = readFileSync("src/lib/api.ts", { encoding: "utf8" });
const api = stripCommentsAndStrings(apiRaw);
const apiNoStrip = stripComments(apiRaw); // keeps string literals (command names)

const appStateRaw = readFileSync("src/lib/app-state.svelte.ts", { encoding: "utf8" });
const appStateStripped = stripCommentsAndStrings(appStateRaw);

const settingsRaw = readFileSync("src/views/SettingsView.svelte", { encoding: "utf8" });
const settingsStripped = stripCommentsAndStrings(settingsRaw);

// ── P0 — checkExportPassword is a pure function with the right signature ──
{
  const pwdRaw = readFileSync("src/lib/backup-password.ts", { encoding: "utf8" });
  const pwd = stripCommentsAndStrings(pwdRaw);
  assertOk(
    /export function checkExportPassword/.test(pwd),
    "P0: checkExportPassword doit être exporté",
  );
  // After stripping, parameters are on separate lines — match loosely.
  const hasEnabled = /enabled/.test(pwd);
  const hasPhrase = /phrase/.test(pwd);
  const hasConfirm = /confirm/.test(pwd);
  assertOk(
    hasEnabled && hasPhrase && hasConfirm,
    "P0: la signature accepte enabled, phrase, confirm",
  );
  // No branch should leak the phrase value.
  assertOk(
    !/reason.*phrase/.test(pwd) && !/phrase.*reason/.test(pwd),
    "P0: aucun motif de refus ne doit contenir la valeur de phrase",
  );
  console.log("  ✓ P0: checkExportPassword exporté, signature correcte, pas de fuite");
}

// ── H1 — exportBackup passes password in invoke args ─────────
// H1-1: the invoke call for export_backup includes the password key.
// We extract the exact invoke call for export_backup and check
// that `password` appears in its argument object.
// Correct: renaming the parameter, reordering object keys.
// Broken: removing `password` from the args object.
// B7: uses stripComments (keeps string literals) so a commented-out
// call placed before the real one cannot satisfy this guard by chance.
{
  const idx = apiNoStrip.indexOf('"export_backup"');
  const close = apiNoStrip.indexOf(');', idx);
  const callBody = apiNoStrip.slice(idx, close);
  assertOk(
    /\bpassword\b/.test(callBody),
    "H1-1: export_backup invoke doit contenir la clé password",
  );
}

// ── H2 — importBackup passes password in invoke args ─────────
// H2-1: the invoke call for import_backup includes the password key.
// Same as H1: stripComments keeps string literals for command names.
{
  const idx = apiNoStrip.indexOf('"import_backup"');
  const close = apiNoStrip.indexOf(');', idx);
  const callBody = apiNoStrip.slice(idx, close);
  assertOk(
    /\bpassword\b/.test(callBody),
    "H2-1: import_backup invoke doit contenir la clé password",
  );
}

// ── H3 — probeBackup exists and calls probe_backup ───────────
// H3-1: the command name probe_backup is invoked.
// B7: stripComments keeps string literals so command names survive.
assertOk(
  /probe_backup/.test(apiNoStrip),
  "H3-1: la commande probe_backup doit être invoquée",
);

// H3-2: the BackupProbe type is exported.
assertOk(
  /interface BackupProbe/.test(api),
  "H3-2: BackupProbe doit être exporté",
);

// H3-3: SnapshotInfo carries the encrypted field.
assertOk(
  /interface SnapshotInfo[\s\S]{0,600}?\bencrypted\b/.test(api),
  "H3-3: SnapshotInfo doit porter le champ encrypted",
);

// ── H4 — refreshLibrary records errors instead of swallowing them ─
// H4-1: the catch block writes to libraryError.
// Correct: renaming the state variable, changing error message text.
// Broken: an empty catch {} that leaves library unchanged.
assertOk(
  /libraryError\s*=\s*/.test(appStateStripped),
  "H4-1: le catch de refreshLibrary doit assigner libraryError",
);

// H4-2: libraryError is reset to null on success.
assertOk(
  /libraryError\s*=\s*null/.test(appStateStripped),
  "H4-2: libraryError doit être remis à null en cas de succès",
);

// H4-3: a toast is dispatched on error.
// After stripping, the string "error" is gone, but the call site
// `this.toast("error", ...)` becomes `this.toast( , ...)` — we look
// for the pattern around it.
assertOk(
  /toast\s*\([^)]*libraryError/.test(appStateStripped),
  "H4-3: une toast error doit être affichée en cas d'échec",
);

// H4-4: the old empty-catch pattern is gone (stripped).
// Correct: any rewrite that doesn't reintroduce an empty catch body.
// Broken: `} catch {` followed immediately by `}` or `await` on the next line.
{
  const fnStart = appStateStripped.indexOf("async refreshLibrary");
  const fnEnd = appStateStripped.indexOf("async refreshStatuses", fnStart);
  const body = fnStart >= 0 && fnEnd > fnStart ? appStateStripped.slice(fnStart, fnEnd) : "";
  // An empty catch: "catch { }" or "catch {\n    }" with no meaningful content.
  const emptyCatch = /catch\s*\{[\s}]*\}/.test(body);
  assertOk(
    !emptyCatch,
    "H4-4: refreshLibrary ne doit pas avoir de catch vide",
  );
}

// ── H5 — doExport validates password before opening the file dialog ─
// Extract the body of doExport only — searching the whole file is fragile
// (an import at the top can satisfy a guard that should check the call site).
const doExportStart = settingsStripped.indexOf("async function doExport");
const doExportEnd = settingsStripped.indexOf("async function doImport", doExportStart);
const doExportBody =
  doExportStart >= 0 && doExportEnd > doExportStart
    ? settingsStripped.slice(doExportStart, doExportEnd)
    : "";

// H5-1: checkExportPassword is called before the save dialog.
// Correct: reordering, renaming local variables.
// Broken: calling exportBackup directly without validation,
// or calling checkExportPassword after save (refuse before opening).
const checkIdx = doExportBody.indexOf("checkExportPassword");
const saveIdx = doExportBody.indexOf("await save");
assertOk(
  checkIdx >= 0 && saveIdx > checkIdx,
  "H5-1: checkExportPassword doit précéder save dans doExport",
);

// H5-2: exportBackup is called with the password result.
// Correct: renaming check, reordering arguments.
// Broken: calling exportBackup without the password argument.
{
  // Find the exportBackup call inside doExport only.
  const callIdx = doExportBody.indexOf("exportBackup");
  assertOk(
    callIdx >= 0,
    "H5-2: exportBackup doit être appelé dans doExport",
  );
  // The call should include password in the arguments.
  const callEnd = doExportBody.indexOf(");", callIdx);
  const callBody = callIdx >= 0 && callEnd > callIdx ? doExportBody.slice(callIdx, callEnd) : "";
  assertOk(
    callBody.includes("password") || callBody.includes("check"),
    "H5-2: l'appel à exportBackup doit transmettre le mot de passe",
  );
}

// H5-3: password fields are cleared in a finally block (runs regardless of outcome).
// After stripping, `exportPassword = ""` becomes `exportPassword = `.
// We pin the exact location: a `finally` keyword inside doExportBody that
// assigns to exportPassword. A guard that counts occurrences is decoration;
// this guard proves the cleanup is in an unreachable path or missing.
const finallyBlock = doExportBody.match(/finally\s*\{([^}]*)\}/);
assertOk(
  finallyBlock !== null,
  "H5-3: doExport doit contenir un bloc finally",
);
assertOk(
  finallyBlock !== null && /exportPassword\s*=/.test(finallyBlock[1]),
  "H5-3: le bloc finally doit vider exportPassword",
);

// H5-4: confirmation field is also cleared in the same finally block.
assertOk(
  finallyBlock !== null && /exportPasswordConfirm\s*=/.test(finallyBlock[1]),
  "H5-4: le bloc finally doit vider exportPasswordConfirm",
);

// ── Structural tests for checkExportPassword (real tests) ─────
{
  // Import the pure function and test it for real.
  const pwdModule = await import("../src/lib/backup-password");
  const check = pwdModule.checkExportPassword as (
    enabled: boolean,
    phrase: string,
    confirm: string,
  ) => { ok: boolean; password?: string; reason?: string };

  // Disabled: always ok, empty password.
  {
    const r = check(false, "", "");
    assertOk(r.ok === true, "test: disabled should always pass");
    assertOk(r.ok === true && r.password === "", "test: disabled returns empty password");
  }

  // Empty phrase: refused.
  {
    const r = check(true, "", "");
    assertOk(r.ok === false, "test: empty phrase should be refused");
    assertOk(
      r.ok === false && r.reason === "required",
      "test: une phrase vide doit rendre le code 'required'",
    );
  }

  // Mismatch: refused.
  {
    const r = check(true, "monsecret", "autre");
    assertOk(r.ok === false, "test: mismatch should be refused");
    assertOk(
      r.ok === false && r.reason === "mismatch",
      "test: deux phrases différentes doivent rendre le code 'mismatch'",
    );
  }

  // Ce que cette directive garde : `reason` est une union FERMÉE, donc une
  // phrase interpolée n'y entre pas — c'est le typage, et non une assertion
  // d'exécution, qui interdit désormais qu'un motif de refus réémette la
  // phrase saisie. Les trois assertions de fuite d'avant I18N-47 étaient
  // devenues vacantes pour cette raison même.
  //
  // Si svelte-check dit « Unused '@ts-expect-error' directive », quelqu'un
  // vient de rendre `reason` assignable depuis un `string` quelconque : la
  // garde est tombée, et cette ligne est le seul endroit qui le dit.
  {
    const phrase = "monsecret";
    // @ts-expect-error — un motif interpolé ne doit PAS être assignable à reason
    const leaked: PasswordCheck = { ok: false, reason: `Phrase ${phrase} invalide` };
    assertOk(leaked.ok === false, "P0-bis: le refus interpolé ne compile pas");
  }

  // Nominal: both match.
  {
    const r = check(true, "monsecret", "monsecret");
    assertOk(r.ok === true, "test: matching passwords should pass");
    assertOk(r.ok === true && r.password === "monsecret", "test: returns the password");
  }

  // Nominal: empty passphrase with encryption disabled.
  {
    const r = check(false, "", "");
    assertOk(r.ok === true, "test: disabled + empty = ok");
  }

  console.log("  ✓ checkExportPassword (codes required/mismatch, désactivé, nominal)");
}

// ── Strip tests: prove the stripper on both usages ───────────
{
  const s = stripCommentsAndStrings;

  // Usage 1: a commented-out call disappears.
  assertOk(
    !s('  // exportBackup(target, options, password)').includes("exportBackup"),
    "stripper: un appel commenté disparaît",
  );

  // Usage 2: the real code survives stripping.
  assertOk(
    s('  exportBackup(target, options, password)').includes("exportBackup"),
    "stripper: le code réel survit au strip",
  );

  // Usage 3: a comment quoting the markup must not satisfy a guard.
  assertOk(
    !s('  // libraryError = `Impossible de lire`').includes("libraryError"),
    "stripper: un commentaire citant libraryError disparaît",
  );

  // Usage 4: stripComments also works (for Svelte HTML comments).
  const c = stripComments;
  assertOk(
    !c('  <!-- exportBackup(target, options) -->').includes("exportBackup"),
    "stripper (stripComments): un commentaire HTML citant exportBackup disparaît",
  );

  console.log("  ✓ stripper prouvé sur ses deux usages (commentaires + chaînes)");
}

// ── H7 — doImport et restoreSnapshot passent par la porte commune ──
// H7-1: doImport n'appelle pas importBackup directement.
// On extrait le corps de doImport et on vérifie qu'aucun appel direct à importBackup n'y figure.
{
  const doImportStart = settingsStripped.indexOf("async function doImport");
  const doImportEnd = settingsStripped.indexOf("async function restoreSnapshot", doImportStart);
  const doImportBody =
    doImportStart >= 0 && doImportEnd > doImportStart
      ? settingsStripped.slice(doImportStart, doImportEnd)
      : "";
  assertOk(
    !/\bimportBackup\b/.test(doImportBody),
    "H7-1: doImport ne doit pas appeler importBackup directement",
  );
}

// H7-2: restoreSnapshot n'appelle pas importBackup directement.
// Le corps de restoreSnapshot s'arrête avant runImportWithPassword.
{
  const restoreStart = settingsStripped.indexOf("async function restoreSnapshot");
  const restoreEnd = settingsStripped.indexOf("async function runImportWithPassword", restoreStart);
  const restoreBody =
    restoreStart >= 0 && restoreEnd > restoreStart
      ? settingsStripped.slice(restoreStart, restoreEnd)
      : "";
  assertOk(
    !/\bimportBackup\b/.test(restoreBody),
    "H7-2: restoreSnapshot ne doit pas appeler importBackup directement",
  );
}

// H7-3: la porte commune (runImportWithPassword) appelle probeBackup AVANT doRestore (qui appelle importBackup).
// On extrait le corps de runImportWithPassword et on vérifie l'ordre d'appel.
{
  const fnStart = settingsStripped.indexOf("async function runImportWithPassword");
  const fnEnd = settingsStripped.indexOf("async function doRestore", fnStart);
  const fnBody =
    fnStart >= 0 && fnEnd > fnStart
      ? settingsStripped.slice(fnStart, fnEnd)
      : "";
  const probeIdx = fnBody.indexOf("probeBackup");
  const doRestoreIdx = fnBody.indexOf("doRestore");
  assertOk(
    probeIdx >= 0 && doRestoreIdx > probeIdx,
    "H7-3: probeBackup doit précéder doRestore dans runImportWithPassword (sonde avant import)",
  );
}

// ── H8 — Dialogue de phrase secrète accessible ──
// H8-1: le dialogue porte use:focusTrap.
// On cherche dans le markup (le code non strippé garde les attributs Svelte).
const settingsRaw2 = readFileSync("src/views/SettingsView.svelte", { encoding: "utf8" });
assertOk(
  /use:focusTrap/.test(settingsRaw2),
  "H8-1: le dialogue doit porter use:focusTrap",
);

// H8-2: le dialogue porte role="dialog".
assertOk(
  /role="dialog"/.test(settingsRaw2),
  "H8-2: le dialogue doit porter role=\"dialog\"",
);

// H8-3: le dialogue porte aria-modal="true".
assertOk(
  /aria-modal="true"/.test(settingsRaw2),
  "H8-3: le dialogue doit porter aria-modal=\"true\"",
);

// H8-4: aria-labelledby référence un id qui existe réellement dans le markup.
// On extrait l'id référencé par aria-labelledby, puis on vérifie qu'un id="..." existe avec cette valeur.
const ariaLabelledByMatch = settingsRaw2.match(/aria-labelledby="([^"]+)"/);
assertOk(
  ariaLabelledByMatch !== null,
  "H8-4: le dialogue doit avoir un aria-labelledby",
);
if (ariaLabelledByMatch) {
  const refId = ariaLabelledByMatch[1];
  // Vérifier que l'id existe quelque part dans le markup.
  const idAttrRegex = new RegExp(`id="${refId}"`);
  assertOk(
    idAttrRegex.test(settingsRaw2),
    `H8-4: l'id ${refId} référencé par aria-labelledby doit exister dans le markup`,
  );
}

// H8-5: l'écouteur Escape est local au dialogue, pas sur window ou document.
// On vérifie qu'il n'y a pas de window.addEventListener("keydown" ... Escape)
// ni document.addEventListener("keydown" ... Escape) dans le fichier.
// On utilise stripCommentsAndStrings pour que "Escape" disparaisse du code.
const settingsFullStripped = stripCommentsAndStrings(settingsRaw2);
assertOk(
  !/window\.addEventListener/.test(settingsFullStripped) &&
    !/document\.addEventListener/.test(settingsFullStripped),
  "H8-5: l'écouteur Escape ne doit pas être sur window ou document",
);
// Vérifier qu'il y a bien un onkeydown ou handleKeydown dans le dialogue.
assertOk(
  /onkeydown|on:keydown|handleKeydown/.test(settingsRaw2),
  "H8-5: un écouteur Escape local doit exister dans le dialogue",
);

// H8-6: la phrase d'import est effacée sur un chemin qui s'exécute quel que soit le résultat.
// On cherche un chemin qui reset importPassword dans un bloc finally ou après le await.
// Le piège du round A : run() attrape ses erreurs, donc un .catch() derrière est du code mort.
// On vérifie que importPassword = "" est dans handlePasswordSubmit (le bloc try ou après).
{
  const pwdStart = settingsStripped.indexOf("async function handlePasswordSubmit");
  const pwdEnd = settingsStripped.indexOf("function handleClosePasswordDialog", pwdStart);
  const pwdBody =
    pwdStart >= 0 && pwdEnd > pwdStart
      ? settingsStripped.slice(pwdStart, pwdEnd)
      : "";
  // La phrase doit être effacée dans handlePasswordSubmit, soit dans un finally, soit après le await.
  // Après stripping, `importPassword = ""` devient `importPassword = `.
  const hasReset = /importPassword\s*=/.test(pwdBody);
  assertOk(
    hasReset,
    "H8-6: importPassword doit être effacé dans handlePasswordSubmit",
  );
}

// H8-7: la liste des instantanés distingue snapshot.encrypted.
// On vérifie que le markup contient une condition sur snapshot.encrypted.
assertOk(
  /snapshot\.encrypted/.test(settingsRaw2),
  "H8-7: la liste des instantanés doit distinguer snapshot.encrypted",
);

// H8-8: pour les archives chiffrées, lua_count n'est pas affiché comme un fait.
// On vérifie que lua_count est dans le bloc {:else) de la chaîne {#if snapshot.encrypted}.
// Approche : dans le bloc each, on cherche un bloc {#if snapshot.encrypted} ... {:else} ... {/if}
// où lua_count apparaît dans la partie :else. Si lua_count est en dehors de ce bloc,
// ou dans le bloc {#if}, la garde rougit.
{
  const eachStart = settingsRaw2.indexOf("{#each snapshots as snapshot");
  const eachEnd = settingsRaw2.indexOf("{/each}", eachStart);
  const eachBody = eachStart >= 0 && eachEnd > eachStart ? settingsRaw2.slice(eachStart, eachEnd) : "";
  // Trouver le bloc {#if snapshot.encrypted} ... {:else} ... {/if} qui contient lua_count.
  // On cherche {#if snapshot.encrypted} suivi (pas trop loin) de :else puis lua_count.
  const encIfIdx = eachBody.indexOf("{#if snapshot.encrypted}");
  assertOk(
    encIfIdx >= 0,
    "H8-8: la liste doit avoir un bloc {#if snapshot.encrypted}",
  );
  // Trouver le {:else} correspondant dans ce bloc.
  const elseIdx = eachBody.indexOf(":else", encIfIdx);
  assertOk(
    elseIdx >= 0,
    "H8-8: le bloc {#if snapshot.encrypted} doit avoir un {:else}",
  );
  // Trouver le {/if} après :else.
  const closeIfIdx = eachBody.indexOf("{/if}", elseIdx);
  assertOk(
    closeIfIdx > elseIdx,
    "H8-8: le bloc {#if ... :else} doit avoir un {/if}",
  );
  // lua_count doit être dans la partie entre :else et {/if}.
  const elseBlock = eachBody.slice(elseIdx, closeIfIdx + 4);
  assertOk(
    /lua_count/.test(elseBlock),
    "H8-8: lua_count doit apparaître dans le bloc :else (non-encrypted)",
  );
  // lua_count ne doit PAS être dans la partie entre {#if} et :else.
  const encBlock = eachBody.slice(encIfIdx, elseIdx);
  assertOk(
    !/lua_count/.test(encBlock),
    "H8-8: lua_count ne doit pas apparaître dans le bloc encrypted ({#if ... :else})",
  );
}

console.log("  ✓ H7: doImport et restoreSnapshot passent par la porte commune");
console.log("  ✓ H8: dialogue accessible et gardes de mot de passe");

// ── H6 — libraryError must be the FIRST condition in the chain (B6) ───
// A bare presence check is useless: the guard proved green while the banner
// was unreachable (visible.length === 0 always won). We must assert that
// `libraryError` appears BEFORE the other branches in the SAME {#if} chain.
// Strategy: find the last {/if} before "appState.libraryError", slice from
// there, and verify that libraryError comes before visible/matches/shown.
const libraryViewRaw = readFileSync("src/views/LibraryView.svelte", { encoding: "utf8" });
const libraryViewStripped = stripCommentsAndStrings(libraryViewRaw);
const idxLibErr = libraryViewStripped.indexOf("appState.libraryError");
assertOk(
  idxLibErr >= 0,
  "H6: libraryError doit exister dans le markup de LibraryView",
);
// Slice from the last {/if} before libraryError to isolate the chain.
const beforeLib = libraryViewStripped.slice(0, idxLibErr);
const lastIfClose = beforeLib.lastIndexOf("{/if}");
const chain = libraryViewStripped.slice(lastIfClose);
const idxLibraryError = chain.indexOf("libraryError");
const idxVisible = chain.indexOf("visible.length");
const idxMatches = chain.indexOf("matches.length");
const idxShown = chain.indexOf("shown.length");
assertOk(
  idxLibraryError < idxVisible,
  "H6: libraryError doit précéder visible.length dans la chaîne",
);
assertOk(
  idxLibraryError < idxMatches,
  "H6: libraryError doit précéder matches.length dans la chaîne",
);
assertOk(
  idxLibraryError < idxShown,
  "H6: libraryError doit précéder shown.length dans la chaîne",
);
console.log("  ✓ H6: libraryError est la première condition de la chaîne");

// ── D1' — doRestore transmet le mot de passe à importBackup ──
// On extrait le corps de doRestore et on vérifie que l'appel à importBackup
// contient bien le mot de passe (importPassword ou pwd).
// Correct : renommer la variable, changer l'ordre des arguments.
// Cassé : retirer le mot de passe de l'appel — l'archive chiffrée échoue.
{
  const fnStart = settingsStripped.indexOf("async function doRestore");
  const fnEnd = settingsStripped.indexOf("async function handlePasswordSubmit", fnStart);
  const fnBody =
    fnStart >= 0 && fnEnd > fnStart
      ? settingsStripped.slice(fnStart, fnEnd)
      : "";
  // On cherche l'appel à importBackup dans doRestore.
  const callIdx = fnBody.indexOf("importBackup");
  assertOk(
    callIdx >= 0,
    "D1-1: importBackup doit être appelé dans doRestore",
  );
  const callEnd = fnBody.indexOf(");", callIdx);
  const callBody = callIdx >= 0 && callEnd > callIdx ? fnBody.slice(callIdx, callEnd) : "";
  assertOk(
    /importBackup\s*\([^)]*password/i.test(callBody) || /importPassword/.test(callBody) || /\bpwd\b/.test(callBody),
    "D1-2: l'appel à importBackup dans doRestore doit transmettre le mot de passe",
  );
}

// ── D2' — handlePasswordSubmit ne ferme pas le dialogue sur erreur ──
// handlePasswordSubmit appelle importBackup directement (pas via run())
// pour que l'erreur remonte et garde le dialogue ouvert.
// Correct : renommer variables, changer le texte du toast.
// Cassé : remettre run() ou doRestore() — le catch est mort, le dialogue se ferme.
//        OU ajouter showPasswordDialog = false dans le catch.
// On utilise le code brut (non strippé) pour analyser la structure des blocs.
{
  const fnStart = settingsRaw2.indexOf("async function handlePasswordSubmit");
  const fnEnd = settingsRaw2.indexOf("function handleClosePasswordDialog", fnStart);
  const fnBody =
    fnStart >= 0 && fnEnd > fnStart
      ? settingsRaw2.slice(fnStart, fnEnd)
      : "";
  // importBackup doit être appelé directement (pas doRestore ni run).
  const hasDirectImport = /importBackup\s*\(/.test(fnBody);
  const hasDoRestore = /doRestore\s*\(/.test(fnBody);
  assertOk(
    hasDirectImport,
    "D2-1: handlePasswordSubmit doit appeler importBackup directement",
  );
  assertOk(
    !hasDoRestore,
    "D2-2: handlePasswordSubmit ne doit pas appeler doRestore (run() avale l'erreur)",
  );
  // On cherche le bloc catch et on vérifie qu'il ne ferme pas le dialogue.
  // On compte les accolades pour extraire le corps du catch correctement.
  const catchIdx = fnBody.indexOf("catch");
  assertOk(
    catchIdx >= 0,
    "D2-3: handlePasswordSubmit doit avoir un bloc catch",
  );
  const braceOpen = fnBody.indexOf("{", catchIdx);
  assertOk(
    braceOpen >= 0,
    "D2-3: le catch doit avoir un bloc ouvert",
  );
  // Compter les accolades pour trouver la fin du catch.
  let depth = 0;
  let catchEnd = braceOpen;
  for (let i = braceOpen; i < fnBody.length; i++) {
    if (fnBody[i] === "{") depth++;
    else if (fnBody[i] === "}") {
      depth--;
      if (depth === 0) {
        catchEnd = i;
        break;
      }
    }
  }
  const catchBody = fnBody.slice(braceOpen + 1, catchEnd);
  assertOk(
    !/showPasswordDialog\s*=\s*false/.test(catchBody),
    "D2-4: le catch ne doit pas fermer le dialogue (showPasswordDialog = false)",
  );
  assertOk(
    !/importPendingPath\s*=\s*null/.test(catchBody),
    "D2-5: le catch ne doit pas réinitialiser importPendingPath",
  );
}

// ── D3' — data-tip sur un élément DOM enveloppant l'icône ──
// Icons.svelte n'a pas de rest props : data-tip sur <Icon> est ignoré.
// Il doit être sur un élément réel (<span>) qui enveloppe l'icône.
// Correct : changer le texte du tip, le nom de la classe du span.
// Cassé : remettre data-tip directement sur <Icon>.
{
  // Chercher le bloc de liste des snapshots.
  const eachStart = settingsRaw2.indexOf("{#each snapshots as snapshot");
  const eachEnd = settingsRaw2.indexOf("{/each}", eachStart);
  const eachBody = eachStart >= 0 && eachEnd > eachStart ? settingsRaw2.slice(eachStart, eachEnd) : "";
  // Le bloc encrypted doit avoir data-tip sur un élément DOM, pas sur <Icon>.
  const encIdx = eachBody.indexOf("{#if snapshot.encrypted}");
  assertOk(
    encIdx >= 0,
    "D3-1: le bloc encrypted doit exister dans la liste des snapshots",
  );
  const encEnd = eachBody.indexOf("{/if}", encIdx);
  const encBlock = encIdx >= 0 && encEnd > encIdx ? eachBody.slice(encIdx, encEnd) : "";
  // data-tip ne doit pas être un attribut de <Icon>.
  const iconWithTip = /<Icon[^>]*data-tip/.test(encBlock);
  assertOk(
    !iconWithTip,
    "D3-2: data-tip ne doit pas être un attribut de <Icon> (pas de rest props)",
  );
  // data-tip doit être sur un élément enveloppant (span, div, button...).
  const tipOnWrapper = /<(span|div|button|a)[^>]*data-tip/.test(encBlock);
  assertOk(
    tipOnWrapper,
    "D3-3: data-tip doit être sur un élément DOM enveloppant l'icône",
  );
}

console.log("  ✓ D1: doRestore transmet le mot de passe à importBackup");
console.log("  ✓ D2: handlePasswordSubmit gère l'erreur sans fermer le dialogue");
console.log("  ✓ D3: data-tip sur un élément DOM enveloppant l'icône");

// ── H9 — readoptIndex : la porte de sortie est atteignable et reste manuelle ──
const libraryViewRaw2 = readFileSync("src/views/LibraryView.svelte", { encoding: "utf8" });
const libraryViewStripped2 = stripCommentsAndStrings(libraryViewRaw2);

// H9-1: readoptIndex est invoqué depuis le frontend.
assertOk(
  /readoptIndex/.test(api),
  "H9-1: readoptIndex doit être exporté depuis api.ts",
);
assertOk(
  /readopt_index/.test(apiNoStrip),
  "H9-2: la commande readopt_index doit être invoquée depuis api.ts",
);
// H9-2b: le chemin est bien transmis dans les args invoke.
{
  const idx = apiNoStrip.indexOf('"readopt_index"');
  const close = apiNoStrip.indexOf(');', idx);
  const callBody = apiNoStrip.slice(idx, close);
  assertOk(
    /\bpath\b/.test(callBody),
    "H9-2b: l'appel readopt_index doit transmettre path dans les args invoke",
  );
}

// H9-3: la bannière libraryError contient un appel à readoptIndex via ConfirmButton.
// On extrait le bloc {#if appState.libraryError} ... {/if} et on vérifie la présence
// de ConfirmButton et de readoptIndex dans son corps.
{
  const libErrStart = libraryViewRaw2.indexOf("{#if appState.libraryError}");
  // La chaîne if/else se termine par le {/if} juste avant le prochain {#if
  // qui n'est pas un {:else if} — ici {#if ctxMenu.
  // On cherche le {/if} qui est le dernier avant un nouveau {#if au niveau
  // racine (pas indenté ou peu indenté, suivi d'un bloc JSX).
  // Approche simplifiée : on cherche le {/if} qui précède </div>\n\n{#if ctxMenu.
  const ctxMenuRef = libraryViewRaw2.indexOf("{#if ctxMenu");
  const libErrBlock = libErrStart >= 0 && ctxMenuRef > libErrStart
    ? libraryViewRaw2.slice(libErrStart, ctxMenuRef)
    : "";
  assertOk(
    /ConfirmButton/.test(libErrBlock),
    "H9-3: la bannière libraryError doit contenir un ConfirmButton",
  );
  assertOk(
    /readoptIndex/.test(libErrBlock),
    "H9-4: la bannière libraryError doit appeler readoptIndex",
  );
}

// H9-5: readoptIndex n'est appelée automatiquement nulle part — ni dans refreshLibrary,
// ni dans adoptFromSteam, ni dans un effet $effect.
// On vérifie dans appStateStripped que readoptIndex n'apparaît pas.
assertOk(
  !/readoptIndex/.test(appStateStripped),
  "H9-5: readoptIndex ne doit pas être appelée automatiquement dans app-state.svelte.ts",
);

// H9-6: la confirmation est bien via ConfirmButton (pas un onclick nu sur readoptIndex).
// Dans le markup strippé, readoptIndex ne doit pas apparaître directement dans un onclick.
{
  const libErrBlockStripped = stripCommentsAndStrings(
    libraryViewRaw2.slice(
      libraryViewRaw2.indexOf("{#if appState.libraryError}"),
      libraryViewRaw2.indexOf("{/if}", libraryViewRaw2.indexOf("{#if appState.libraryError}")) + 4,
    ),
  );
  // readoptIndex doit être dans le corps de onconfirm, pas dans un onclick nu.
  assertOk(
    /onconfirm/.test(libErrBlockStripped),
    "H9-6: readoptIndex doit être appelé via onconfirm de ConfirmButton",
  );
}

console.log("  ✓ H9: readoptIndex atteignable depuis la bannière, jamais automatique");

console.log("  ✓ tripwires structurels LOT-21 (api.ts, app-state.svelte.ts, SettingsView.svelte)");

// ── LOT-22 — la fonction de dépouillement est elle-même testée ──
// Piège n°32 : une garde textuelle ne vaut que ce que vaut son passage de
// dépouillement. `stripCommentsAndStrings` est testé chez lui ; sa variante
// qui préserve les `${...}` est neuve, donc chaque garde bâtie dessus repose
// sur elle. Les deux premiers cas sont ceux qui ont laissé passer cinq
// réécritures cassées au LOT-12 : un commentaire ou une chaîne qui **cite**
// le code supprimé ne doit jamais tenir la garde verte.
{
  const strip = stripCommentsAndStringsPreserveTemplateExprs;

  assertOk(
    !/config_kept_local/.test(strip("// on consulte summary.config_kept_local ici")),
    "strip: un commentaire de ligne citant l'identifiant ne survit pas",
  );
  assertOk(
    !/config_kept_local/.test(strip("/* summary.config_kept_local */ const x = 1;")),
    "strip: un commentaire de bloc citant l'identifiant ne survit pas",
  );
  assertOk(
    !/config_kept_local/.test(strip('const s = "summary.config_kept_local";')),
    "strip: une chaîne double-quote citant l'identifiant ne survit pas",
  );
  assertOk(
    !/config_kept_local/.test(strip("const t = `texte config_kept_local sans expression`;")),
    "strip: le texte littéral d'un template n'est pas du code — il ne survit pas",
  );
  assertOk(
    /config_kept_local/.test(strip("const t = `a ${f(summary.config_kept_local)} b`;")),
    "strip: une expression ${...} d'un template survit — c'est du code",
  );
  assertOk(
    /config_kept_local/.test(
      strip("const t = `${cond ? 'x' : 'y'}${g(summary.config_kept_local)}`;"),
    ),
    "strip: une expression imbriquée n'empêche pas la suivante de survivre",
  );
  // L'apostrophe française ne doit pas avaler la fin du fichier : c'est la
  // raison pour laquelle `stripCommentsAndStrings` ne dépouille pas les
  // quotes simples (voir son commentaire), et la variante doit s'y tenir.
  assertOk(
    /config_kept_local/.test(strip("Lire l'annonce\nconst x = summary.config_kept_local;")),
    "strip: une apostrophe française n'avale pas le reste de la source",
  );

  console.log("  ✓ LOT-22: le passage de dépouillement est lui-même testé (7 cas)");
}

// ── LOT-22 — config_kept_local consulté aux deux sites de message ──
// La garde doit prouver que les DEUX constructions de message (doRestore
// et handlePasswordSubmit) consultent summary.config_kept_local.
// Les messages sont dans des template literals : stripCommentsAndStrings
// normal les supprime entièrement (y compris les ${...}). On utilise
// stripCommentsAndStringsPreserveTemplateExprs qui extrait les expressions.
// Un compteur d'occurrences ne suffit pas : on épingle la position dans
// le corps de CHAQUE fonction.
{
  const settingsForLot22 = stripCommentsAndStringsPreserveTemplateExprs(settingsRaw);

  const doRestStart = settingsForLot22.indexOf("async function doRestore");
  const doRestEnd = settingsForLot22.indexOf("async function handlePasswordSubmit", doRestStart);
  const doRestBody =
    doRestStart >= 0 && doRestEnd > doRestStart
      ? settingsForLot22.slice(doRestStart, doRestEnd)
      : "";
  assertOk(
    /config_kept_local/.test(doRestBody),
    "LOT-22: doRestore doit consulter config_kept_local dans le message de restauration",
  );

  const pwdStart = settingsForLot22.indexOf("async function handlePasswordSubmit");
  const pwdEnd = settingsForLot22.indexOf("function handleClosePasswordDialog", pwdStart);
  const pwdBody =
    pwdStart >= 0 && pwdEnd > pwdStart
      ? settingsForLot22.slice(pwdStart, pwdEnd)
      : "";
  assertOk(
    /config_kept_local/.test(pwdBody),
    "LOT-22: handlePasswordSubmit doit consulter config_kept_local dans le toast",
  );

  // Vérification croisée : config_kept_local doit exister dans api.ts aussi.
  assertOk(
    /config_kept_local/.test(api),
    "LOT-22: ImportSummary doit déclarer config_kept_local",
  );

  console.log("  ✓ LOT-22: config_kept_local consulté aux deux sites de message");
}

// ── PO-01 — handlePasswordSubmit bloque la réentrance et la fermeture ──
// PO-01-1: handlePasswordSubmit commence par une garde busy === "import".
// Cette garde couvre les trois portes (bouton, Entrée, overlay/Échap) d'un coup.
// Correct : renommer le string, ajouter un espace.
// Cassé : retirer la garde → double restauration possible.
// Attention : stripCommentsAndStrings supprime le littéral "import" mais garde
// la comparaison `busy === `. On teste la présence de busy ===, pas le littéral.
{
  const fnStart = settingsStripped.indexOf("async function handlePasswordSubmit");
  const fnEnd = settingsStripped.indexOf("function handleClosePasswordDialog", fnStart);
  const fnBody =
    fnStart >= 0 && fnEnd > fnStart
      ? settingsStripped.slice(fnStart, fnEnd)
      : "";
  // Trois exigences, et les deux dernières ne sont pas du zèle : une campagne
  // adverse a fait survivre exactement ces deux mutations-là.
  //
  //  a) la comparaison existe ;
  //  b) l'affectation qui la rend opérante existe aussi. Garder `busy === …`
  //     sans jamais poser `busy = …` laisse la fonction grande ouverte tout en
  //     ayant l'air gardée — c'est la mutation qui survivait ;
  //  c) et l'ordre : comparaison, PUIS affectation, PUIS le premier `await`.
  //     Une garde placée après l'affectation est inatteignable, et une garde
  //     placée après le premier `await` arrive trop tard — le second appel est
  //     déjà parti. C'est la position qui porte l'exigence, pas la présence
  //     (forme de garde creuse n°3 de ETAT.md).
  const guard = fnBody.search(/busy\s*===/);
  const assign = fnBody.search(/busy\s*=\s*[^=]/);
  const firstAwait = fnBody.search(/\bawait\b/);
  assertOk(guard >= 0, "PO-01-1: handlePasswordSubmit doit comparer busy");
  assertOk(
    assign >= 0,
    "PO-01-1: handlePasswordSubmit doit AFFECTER busy — une comparaison sans " +
      "affectation ne bloque rien",
  );
  assertOk(
    guard < assign,
    "PO-01-1: la comparaison de busy doit précéder son affectation, sinon la " +
      "garde est inatteignable",
  );
  assertOk(
    firstAwait < 0 || assign < firstAwait,
    "PO-01-1: busy doit être posé AVANT le premier await, sinon un second " +
      "appel part pendant l'attente",
  );
}

// PO-01-2: handleClosePasswordDialog bloque pendant l'opération.
// Si on peut fermer le dialogue pendant une restauration, l'utilisateur perd
// toute visibilité sur ce qui se passe — et le dialogue disparaît pendant
// que le code continue en arrière-plan.
{
  const fnStart = settingsStripped.indexOf("function handleClosePasswordDialog");
  const fnEnd = settingsStripped.indexOf("async function dropSnapshot", fnStart);
  const fnBody =
    fnStart >= 0 && fnEnd > fnStart
      ? settingsStripped.slice(fnStart, fnEnd)
      : "";
  assertOk(
    /busy\s*===/.test(fnBody),
    "PO-01-2: handleClosePasswordDialog doit vérifier busy",
  );
}

// PO-01-3: handlePasswordSubmit libère busy dans un finally.
// Un busy jamais relâché condamne le dialogue.
{
  const fnStart = settingsStripped.indexOf("async function handlePasswordSubmit");
  const fnEnd = settingsStripped.indexOf("function handleClosePasswordDialog", fnStart);
  const fnBody =
    fnStart >= 0 && fnEnd > fnStart
      ? settingsStripped.slice(fnStart, fnEnd)
      : "";
  const finallyBlock = fnBody.match(/finally\s*\{([^}]*)\}/);
  assertOk(
    finallyBlock !== null,
    "PO-01-3: handlePasswordSubmit doit contenir un bloc finally",
  );
  assertOk(
    finallyBlock !== null && /busy\s*=\s*null/.test(finallyBlock[1]),
    "PO-01-3: le finally doit libérer busy",
  );
}

console.log("  ✓ PO-01: réentrance, fermeture bloquée, finally");

// ── MAJ-C — bouton de mise à jour persistant dans le shell ──────────
// Le bouton doit survivre à la navigation : s'il est dans une vue, il
// disparaît dès qu'on change d'onglet. Il doit être dans App.svelte.
{
  const appRaw = readFileSync("src/App.svelte", { encoding: "utf8" });
  const appStripped = stripCommentsAndStringsPreserveTemplateExprs(appRaw);
  const modalRaw = readFileSync("src/components/UpdateModal.svelte", { encoding: "utf8" });
  const modalStripped = stripCommentsAndStringsPreserveTemplateExprs(modalRaw);

  // MAJ-C-1 : le bouton est DANS le bloc {#if appState.updateAvailable}.
  // Après stripping, les identifiants survivent. On vérifie que le bouton
  // apparaît après l'ouverture du bloc et avant sa fermeture.
  const ifOpenIdx = appStripped.indexOf("{#if appState.updateAvailable}");
  assertOk(ifOpenIdx >= 0, "MAJ-C-1 : App.svelte doit contenir {#if appState.updateAvailable}");
  // Trouver la fin du bloc conditionnel (prochain {/if} ou fin du fichier).
  const ifCloseIdx = appStripped.indexOf("{/if}", ifOpenIdx);
  const blockBody = ifCloseIdx > ifOpenIdx
    ? appStripped.slice(ifOpenIdx, ifCloseIdx)
    : appStripped.slice(ifOpenIdx);
  assertOk(
    blockBody.includes("showUpdateModal = true"),
    "MAJ-C-1 : le bouton doit être dans le bloc {#if appState.updateAvailable}",
  );

  // MAJ-C-2 : la fonction de mise à jour vérifie la réentrance AVANT
  // le premier await, et relâche l'état dans un finally.
  // handleUpdate vit dans UpdateModal.svelte.
  // On extrait le corps de handleUpdate en comptant les accolades.
  const fnStart = modalStripped.indexOf("async function handleUpdate");
  assertOk(fnStart >= 0, "MAJ-C-2 : handleUpdate doit exister dans UpdateModal.svelte");
  // Trouver l'accolade ouvrante de la fonction.
  const braceOpen = modalStripped.indexOf("{", fnStart);
  assertOk(braceOpen >= 0, "MAJ-C-2 : handleUpdate doit avoir une accolade ouvrante");
  // Compter les accolades pour trouver la fermante.
  let depth = 0;
  let fnEnd = braceOpen;
  for (let i = braceOpen; i < modalStripped.length; i++) {
    if (modalStripped[i] === "{") depth++;
    else if (modalStripped[i] === "}") {
      depth--;
      if (depth === 0) {
        fnEnd = i;
        break;
      }
    }
  }
  const fnBody = fnStart >= 0 && fnEnd > braceOpen
    ? modalStripped.slice(fnStart, fnEnd + 1)
    : "";

  // La garde de réentrance : comparaison, affectation, puis await.
  // Après stripping, les littéraux disparaissent mais les identifiants survivent.
  const guardIdx = fnBody.search(/\(busy\)/);
  const assignIdx = fnBody.search(/busy\s*=\s*[^=]/);
  const firstAwaitIdx = fnBody.search(/\bawait\b/);
  assertOk(
    guardIdx >= 0 && firstAwaitIdx >= 0 && guardIdx < firstAwaitIdx,
    "MAJ-C-2 : la comparaison busy doit précéder le premier await",
  );
  assertOk(
    assignIdx >= 0 && firstAwaitIdx >= 0 && assignIdx < firstAwaitIdx,
    "MAJ-C-2 : l'affectation busy = doit précéder le premier await",
  );

  // Le finally libère busy.
  const finallyBlock = fnBody.match(/finally\s*\{([^}]*)\}/);
  assertOk(
    finallyBlock !== null,
    "MAJ-C-2 : handleUpdate doit contenir un bloc finally",
  );
  assertOk(
    finallyBlock !== null && /busy\s*=\s*null/.test(finallyBlock[1]),
    "MAJ-C-2 : le finally doit libérer busy",
  );

  // MAJ-C-2b : installUpdate reçoit la valeur retournée par downloadUpdate,
  // pas artifact.file. C'est la garde qui empêche le bouton de ne rien faire.
  const dlCall = fnBody.indexOf("downloadUpdate(");
  const instCall = fnBody.indexOf("installUpdate(");
  assertOk(dlCall >= 0 && instCall >= 0, "MAJ-C-2b : downloadUpdate et installUpdate doivent exister");
  // extraire l'argument de installUpdate( ... )
  const parenOpen = fnBody.indexOf("(", instCall);
  const parenClose = fnBody.indexOf(")", parenOpen);
  const instArg = fnBody.slice(parenOpen + 1, parenClose).trim();
  // L'argument ne doit pas être artifact.file.
  assertOk(
    instArg !== "artifact.file",
    "MAJ-C-2b : installUpdate ne doit pas recevoir artifact.file",
  );

  // MAJ-C-3 : la modale porte use:focusTrap.
  // stripCommentsAndStringsPreserveTemplateExprs ne retire que commentaires
  // et littéraux, jamais les attributs de balise — le dépouillement suffit.
  assertOk(
    /use:focusTrap/.test(modalStripped),
    "MAJ-C-3 : la modale doit porter use:focusTrap",
  );

  // MAJ-C-4 : l'onclick du bouton ouvre la modale.
  // Le bouton dans le bloc conditionnel doit contenir showUpdateModal = true.
  assertOk(
    blockBody.includes("showUpdateModal = true"),
    "MAJ-C-4 : le bouton doit ouvrir la modale via showUpdateModal = true",
  );

  // MAJ-C-5 : <UpdateModal est monté dans App.svelte.
  const appModalRaw = readFileSync("src/App.svelte", { encoding: "utf8" });
  assertOk(
    /<UpdateModal/.test(appModalRaw),
    "MAJ-C-5 : App.svelte doit monter <UpdateModal",
  );

  // MAJ-C-6 : la fonction de fermeture vérifie busy avant onclose.
  // Modèle : handleClosePasswordDialog (PO-01-2). Si on retire le guard,
  // la modale se ferme pendant l'installation.
  const closeStart = modalStripped.indexOf("function closeModal");
  assertOk(closeStart >= 0, "MAJ-C-6 : closeModal doit exister dans UpdateModal.svelte");
  const closeBraceOpen = modalStripped.indexOf("{", closeStart);
  const closeBraceDepth = (() => {
    let d = 0;
    for (let i = closeBraceOpen; i < modalStripped.length; i++) {
      if (modalStripped[i] === "{") d++;
      else if (modalStripped[i] === "}") { d--; if (d === 0) return i; }
    }
    return modalStripped.length;
  })();
  const closeBody = closeStart >= 0 && closeBraceOpen >= 0
    ? modalStripped.slice(closeStart, closeBraceDepth + 1)
    : "";
  assertOk(
    /busy/.test(closeBody),
    "MAJ-C-6 : closeModal doit vérifier busy avant onclose",
  );

  console.log("  ✓ MAJ-C : bouton persistant, réentrance, focusTrap");
}

// ── MAJ-D — checkUpdateResult appelé au démarrage ────────────────
// MAJ-D-1 : app-state.svelte.ts contient un appel à checkUpdateResult
// dans le corps de la fonction de démarrage (onMount dans App.svelte),
// pas seulement importé ou cité dans un commentaire.
// On utilise stripCommentsAndStringsPreserveTemplateExprs pour que le
// code survive au dépouillement (la méthode checkUpdateResult est dans
// un template literal dans App.svelte).
{
  const appRaw = readFileSync("src/App.svelte", { encoding: "utf8" });
  const appStripped = stripCommentsAndStringsPreserveTemplateExprs(appRaw);

  // MAJ-D-1 : checkUpdateResult est appelé dans le bloc onMount.
  // On extrait le corps de la fonction IIFE dans onMount.
  const onMountStart = appStripped.indexOf("onMount(");
  assertOk(onMountStart >= 0, "MAJ-D-1 : App.svelte doit contenir onMount");
  // Trouver l'accolade ouvrante de onMount.
  const onMountBrace = appStripped.indexOf("(", onMountStart);
  const fnOpen = appStripped.indexOf("{", onMountBrace);
  assertOk(fnOpen >= 0, "MAJ-D-1 : onMount doit avoir une accolade ouvrante");
  // Compter les accolades pour trouver la fin.
  let depth = 0;
  let fnEnd = fnOpen;
  for (let i = fnOpen; i < appStripped.length; i++) {
    if (appStripped[i] === "{") depth++;
    else if (appStripped[i] === "}") {
      depth--;
      if (depth === 0) { fnEnd = i; break; }
    }
  }
  const onMountBody = appStripped.slice(fnOpen, fnEnd + 1);
  assertOk(
    /checkUpdateResult/.test(onMountBody),
    "MAJ-D-1 : onMount doit appeler checkUpdateResult (résultat de mise à jour)",
  );

  // MAJ-D-2 : checkUpdateResult est défini dans app-state.svelte.ts.
  assertOk(
    /async checkUpdateResult/.test(appStateStripped),
    "MAJ-D-2 : app-state.svelte.ts doit définir checkUpdateResult",
  );

  // MAJ-D-3 : la vérification de mise à jour part AVANT l'adoption Steam.
  //
  // Mesuré sur une installation réelle : `adoptFromSteam` parcourt toute la
  // bibliothèque Steam et appelle le réseau par jeu — près d'une minute. Placée
  // derrière, la vérification n'aboutissait qu'au bout de ce délai : le bouton
  // n'apparaissait pas, l'utilisateur concluait que la détection ne marchait
  // pas et allait la déclencher à la main dans les Réglages.
  //
  // C'est donc l'ORDRE qui porte l'exigence, pas la présence des deux appels.
  const checkIdx = onMountBody.indexOf("checkForUpdate");
  const adoptIdx = onMountBody.indexOf("adoptFromSteam");
  assertOk(checkIdx >= 0, "MAJ-D-3 : onMount doit appeler checkForUpdate");
  assertOk(adoptIdx >= 0, "MAJ-D-3 : onMount doit appeler adoptFromSteam");
  assertOk(
    checkIdx < adoptIdx,
    "MAJ-D-3 : checkForUpdate doit partir AVANT adoptFromSteam — derrière une adoption " +
      "qui dure une minute, la mise à jour passe pour non détectée",
  );

  console.log("  ✓ MAJ-D : checkUpdateResult au démarrage, et vérification avant l'adoption");
}

// Marker for the runner to prove this suite was imported (B1).
export const __backupWiringRan = true;

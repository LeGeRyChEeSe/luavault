/**
 * Shared source-stripping helpers used by every "*-wiring.ts" structural
 * guard file in this directory.
 *
 * The DLC reconciliation feature these helpers used to police
 * (LOT-10 fix02: GameSpotlight.svelte's DLC section, the remote /dlc/
 * fetch, the session cache) was removed with the public edition — there is
 * no LuaVault-operated network service to reconcile against. Only the
 * parser helpers below remain; every other "*-wiring.ts" file imports them.
 *
 * Runs from the project root (validate.ps1 and `npm run test:ts` both
 * reach it through test-virtual-scroll.ts).
 */

function assertOk(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

// ── Strip comments and strings ────────────────────────────────
// A single left-to-right pass removes //,  /* *&#47;, <!-- -->, and the
// double-quote and backtick string literals. Escaped quotes inside a
// string do not end it. Single-quoted strings are deliberately NOT
// stripped: Svelte template text carries French apostrophes (l'annonce,
// d'un) that a naive scanner would treat as openers, swallowing the rest
// of the file. The codebase uses double quotes in code, so no guard
// depends on single-quote stripping. The result preserves newlines and
// replaces removed spans with a single space, so line-oriented patterns
// still work.
export function stripCommentsAndStrings(src: string): string {
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
    // Double-quoted string or template literal.
    if (c === '"' || c === "`") {
      const q = c;
      i++;
      while (i < n && src[i] !== q) {
        if (src[i] === "\\") i++;
        i++;
      }
      i++; // closing quote
      out += " ";
      continue;
    }
    out += c;
    i++;
  }
  return out;
}

// ── Strip comments only ───────────────────────────────────────
// The comment branches of stripCommentsAndStrings, WITHOUT the string
// branch — for guards that must keep seeing string literals (an
// <option>'s value IS the wiring) while still refusing commented-out
// markup (pitfall 32). Known limitation, accepted: a "//" or "<!--"
// inside a string literal eats the rest of that line — the guards built
// on this target one well-known line, not arbitrary code.
export function stripComments(src: string): string {
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
    out += c;
    i++;
  }
  return out;
}

// ── Tests for stripCommentsAndStrings itself ──────────────────
// The function is the foundation: if it is wrong, every guard above is.
{
  const s = stripCommentsAndStrings;
  assertOk(s("a // comment\nb").includes("b"), "strip: le code après un // survit");
  assertOk(!s("a // refreshDlc(x)\nb").includes("refreshDlc"), "strip: un appel commenté disparaît");
  assertOk(!s("a /* block */ b").includes("block"), "strip: un bloc /* */ disparaît");
  assertOk(!s("a <!-- html --> b").includes("html"), "strip: un commentaire HTML disparaît");
  assertOk(!s('a "str" b').includes("str"), "strip: une chaîne double disparaît");
  assertOk(!s("a `str` b").includes("str"), "strip: un template literal disparaît");
  assertOk(!s('a "esc\\"aped" b').includes("esc"), "strip: un échappement \\\" ne termine pas la chaîne");
  // Single quotes are NOT stripped: French apostrophes in Svelte template
  // text (l'annonce, d'un) would swallow the rest of the file.
  assertOk(s("a 'str' b").includes("'str'"), "strip: les guillemets simples survivent (apostrophes françaises)");
  assertOk(s("a /* x */ b").includes("a") && s("a /* x */ b").includes("b"), "strip: le code autour d'un bloc survit");
  // A comment quoting the markup must not satisfy a guard.
  assertOk(
    !s("<!-- {#if showDlc} generated_at (UTC) {#each lua_dlc_app_ids} -->").includes("showDlc"),
    "strip: un commentaire HTML citant le markup ne satisfait aucun garde-fou",
  );
  console.log("  ✓ stripCommentsAndStrings (commentaires //,  /* */ , <!-- -->, chaînes, échappements)");

  // stripComments: same comment handling, strings KEPT — this is what P4
  // needs (the option's value is a string), and the difference that makes
  // a commented-out option fall.
  const c = stripComments;
  assertOk(c("a <!-- commented --> b").includes("a") && !c("a <!-- commented --> b").includes("commented"), "stripComments: un commentaire HTML disparaît");
  assertOk(
    !c('<!-- <option value="playtime">Temps de jeu</option> -->').includes("option"),
    "stripComments: une option mise en commentaire HTML disparaît",
  );
  assertOk(c('a <option value="playtime"> b').includes('<option value="playtime">'), "stripComments: les chaînes survivent");
  assertOk(!c("a // line comment\nb").includes("line"), "stripComments: un commentaire // disparaît");
  assertOk(c("a // line comment\nb").includes("b"), "stripComments: le code après un // survit");
  assertOk(c("a /* block */ b").includes("a") && c("a /* block */ b").includes("b") && !c("a /* block */ b").includes("block"), "stripComments: un bloc /* */ disparaît");
  assertOk(c("a 'l'annonce' b").includes("l'annonce"), "stripComments: les apostrophes françaises survivent");
  console.log("  ✓ stripComments (commentaires retirés, chaînes conservées)");
}

console.log("  ✓ helpers de dépouillement de source (stripComments / stripCommentsAndStrings)");

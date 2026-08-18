// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readdirSync, readFileSync, statSync } from "node:fs";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { join } from "node:path";

const ACCENT = /[éèàçêâûîôëïüÉÈÀÇÊÂÛÎÔËÏÜ]/;
const ROOT = "src";

/**
 * This guard detects literals containing an accented character. It cannot
 * detect French text without an accent (for example "dans", "sans", "avec",
 * or compound terms such as the ToolsView folder label). Such text is covered
 * only by a dedicated graphical demonstration when one exists. Do not add a
 * French-word dictionary here: it would be fragile and create technical-name
 * false positives.
 *
 * Every exception is deliberately local and documented. Do not add broad
 * filename or directory patterns here: a UI literal must be translated rather
 * than hidden from this guard. There are no legitimate accented literals today.
 */
const EXCEPTIONS: Readonly<Record<string, readonly number[]>> = {
  "src/lib/i18n.svelte.ts": [15], // Locale endonym is a stable selector label, not translated UI copy.
  // Example shape for a future raw-data exception:
  // "src/lib/example.ts": [42], // Exact line is a raw external payload, not UI text.
};

type Finding = { path: string; line: number; value: string };

function lineAt(source: string, index: number): number {
  return source.slice(0, index).split("\n").length;
}

type ScanState = {
  lastNonWhitespace: string;
  previousNonWhitespace: string;
  lastCodeWord: string;
};

function regexStartsHere(state: ScanState): boolean {
  return (
    state.lastNonWhitespace === "" ||
    "(,=:[!&|?{};*%~^<>".includes(state.lastNonWhitespace) ||
    ((state.lastNonWhitespace === "+" || state.lastNonWhitespace === "-") &&
      state.previousNonWhitespace !== state.lastNonWhitespace) ||
    state.lastCodeWord === "return" ||
    state.lastCodeWord === "typeof"
  );
}

function rememberCode(state: ScanState, char: string): void {
  if (/\s/.test(char)) return;
  state.previousNonWhitespace = state.lastNonWhitespace;
  state.lastNonWhitespace = char;
  state.lastCodeWord = /[A-Za-z_$0-9]/.test(char) ? state.lastCodeWord + char : "";
}

function endOfRegex(source: string, start: number, end: number): number {
  let i = start + 1;
  let inCharacterClass = false;
  while (i < end) {
    const char = source[i++];
    if (char === "\\") {
      i++;
    } else if (char === "[") {
      inCharacterClass = true;
    } else if (char === "]") {
      inCharacterClass = false;
    } else if (char === "/" && !inCharacterClass) {
      while (i < end && /[a-z]/i.test(source[i])) i++;
      return i;
    }
  }
  return end;
}

function endOfQuoted(source: string, start: number, end: number, quote: string): number {
  let i = start + 1;
  while (i < end) {
    const char = source[i++];
    if (char === "\\") i++;
    else if (char === quote) return i;
  }
  return end;
}

/** A template nested in an expression cannot close its outer expression. */
function endOfTemplate(source: string, start: number, end: number): number {
  let i = start + 1;
  while (i < end) {
    const char = source[i++];
    if (char === "\\") i++;
    else if (char === "`") return i;
  }
  return end;
}

function endOfTemplateExpression(source: string, start: number, end: number): number {
  let i = start;
  let depth = 1;
  const state: ScanState = { lastNonWhitespace: "", previousNonWhitespace: "", lastCodeWord: "" };
  while (i < end && depth > 0) {
    const char = source[i];
    if (char === "/" && source[i + 1] === "/") {
      i += 2;
      while (i < end && source[i] !== "\n") i++;
      continue;
    }
    if (char === "/" && source[i + 1] === "*") {
      i += 2;
      while (i < end && !(source[i] === "*" && source[i + 1] === "/")) i++;
      i += 2;
      continue;
    }
    if (char === "/" && regexStartsHere(state)) {
      i = endOfRegex(source, i, end);
      continue;
    }
    if (char === "'" || char === '"') {
      i = endOfQuoted(source, i, end, char);
      continue;
    }
    if (char === "`") {
      i = endOfTemplate(source, i, end);
      continue;
    }
    if (char === "{") depth++;
    if (char === "}") depth--;
    rememberCode(state, char);
    i++;
  }
  return i;
}

function scanQuoted(source: string, start: number, end: number, path: string, findings: Finding[]): void {
  let i = start;
  const state: ScanState = { lastNonWhitespace: "", previousNonWhitespace: "", lastCodeWord: "" };
  while (i < end) {
    const c = source[i];
    if (c === "/" && source[i + 1] === "/") {
      i += 2;
      while (i < end && source[i] !== "\n") i++;
      continue;
    }
    if (c === "/" && source[i + 1] === "*") {
      i += 2;
      while (i < end && !(source[i] === "*" && source[i + 1] === "/")) i++;
      i += 2;
      continue;
    }
    if (c === "/" && regexStartsHere(state)) {
      i = endOfRegex(source, i, end);
      continue;
    }
    if (c === "`") {
      // Template text is checked as a literal; each ${…} is then scanned as
      // code so a quote or nested template cannot hide the expression that follows.
      const literalStart = i;
      let value = "";
      i++;
      while (i < end) {
        const char = source[i];
        if (char === "\\") {
          value += source[i + 1] ?? "";
          i += 2;
        } else if (char === "`") {
          if (ACCENT.test(value)) findings.push({ path, line: lineAt(source, literalStart), value });
          i++;
          break;
        } else if (char === "$" && source[i + 1] === "{") {
          if (ACCENT.test(value)) findings.push({ path, line: lineAt(source, literalStart), value });
          const expressionStart = i + 2;
          const expressionEnd = endOfTemplateExpression(source, expressionStart, end);
          scanQuoted(source, expressionStart, expressionEnd - 1, path, findings);
          value = "";
          i = expressionEnd;
        } else {
          value += char;
          i++;
        }
      }
      rememberCode(state, "`");
      continue;
    }
    if (c !== "'" && c !== '"') {
      rememberCode(state, c);
      i++;
      continue;
    }

    const quote = c;
    const literalStart = i;
    let value = "";
    i++;
    while (i < end && source[i] !== quote) {
      if (source[i] === "\\") {
        value += source[i + 1] ?? "";
        i += 2;
      } else {
        value += source[i++];
      }
    }
    if (ACCENT.test(value)) findings.push({ path, line: lineAt(source, literalStart), value });
    i++;
    rememberCode(state, quote);
  }
}

function scanSvelte(source: string, path: string, findings: Finding[]): void {
  const scripts: { start: number; end: number }[] = [];
  const re = /<script(?:\s[^>]*)?>/gi;
  for (let match = re.exec(source); match; match = re.exec(source)) {
    const start = re.lastIndex;
    const close = source.indexOf("</script>", start);
    if (close < 0) break;
    scripts.push({ start, end: close });
    scanQuoted(source, start, close, path, findings);
    re.lastIndex = close + "</script>".length;
  }

  let markup = source;
  for (const script of scripts) {
    markup = `${markup.slice(0, script.start)}${" ".repeat(script.end - script.start)}${markup.slice(script.end)}`;
  }
  // Tags can contain quoted attribute literals; text nodes are the other
  // visible-literal surface. HTML comments are removed before either.
  markup = markup.replace(/<!--[\s\S]*?-->/g, (comment) => " ".repeat(comment.length));
  for (const tag of markup.matchAll(/<[^>]*>/g)) {
    scanQuoted(source, tag.index ?? 0, (tag.index ?? 0) + tag[0].length, path, findings);
  }
  const text = markup.replace(/<[^>]*>/g, (tag) => " ".repeat(tag.length));
  for (const match of text.matchAll(new RegExp(ACCENT.source, "g"))) {
    findings.push({ path, line: lineAt(source, match.index ?? 0), value: match[0] });
  }
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`I18N-55 valeur: ${message}`);
}

{
  const findings: Finding[] = [];
  const source = "const matcher = /don't/;\nconst label = 'Échec';";
  scanQuoted(source, 0, source.length, "value.ts", findings);
  assert(
    findings.some((finding) => finding.value === "Échec"),
    "une regex /don't/ ne doit pas avaler la chaîne accentuée qui la suit",
  );
}
{
  const findings: Finding[] = [];
  const source = "const label = `neutral ${'Échec'}`;";
  scanQuoted(source, 0, source.length, "value.ts", findings);
  assert(
    findings.some((finding) => finding.value === "Échec"),
    "une expression ${…} de template doit être analysée séparément du texte littéral",
  );
}

function sourceFiles(dir: string): string[] {
  const files: string[] = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      if (path.replace(/\\/g, "/") !== "src/lib/i18n") files.push(...sourceFiles(path));
    } else if (path.endsWith(".ts") || path.endsWith(".svelte")) {
      files.push(path);
    }
  }
  return files;
}

const files = sourceFiles(ROOT);
const findings: Finding[] = [];
for (const path of files) {
  const source = readFileSync(path, "utf8");
  const normalized = path.replace(/\\/g, "/");
  if (path.endsWith(".svelte")) scanSvelte(source, normalized, findings);
  else scanQuoted(source, 0, source.length, normalized, findings);
}

const unexpected = findings.filter((finding) => !EXCEPTIONS[finding.path]?.includes(finding.line));
if (unexpected.length) {
  throw new Error(
    `I18N-55: literal(s) accented outside src/lib/i18n must use t():\n${unexpected
      .map((finding) => `  ${finding.path}:${finding.line} → ${JSON.stringify(finding.value)}`)
      .join("\n")}`,
  );
}

console.log(`  ✓ I18N-55: ${files.length} sources sans littéral français non routé`);
export const __i18nLiteralsRan = true;
export const i18nLiteralsSuite = Promise.resolve();
export const i18nLiteralsCases = () => 3;

// @ts-expect-error — Node types are deliberately not a frontend dependency.
import { existsSync, readFileSync, readdirSync } from "node:fs";
// @ts-expect-error — Node types are deliberately not a frontend dependency.
import { dirname, join, relative, resolve } from "node:path";
// @ts-expect-error — Node types are deliberately not a frontend dependency.
import { fileURLToPath } from "node:url";

let cases = 0;

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

/**
 * Mask comments and every JavaScript string form while keeping offsets and
 * newlines intact. `stripCommentsAndStrings` in test-dlc-wiring deliberately
 * keeps single quotes (Svelte prose contains French apostrophes) and replaces
 * a removed span with one space. That is right for its Svelte guards, but not
 * here: E2E imports use single-quoted paths and we must read those paths back
 * at exactly the same offsets in the original TypeScript source.
 */
export function stripCommentsAndStringsPreservingLength(source: string): string {
  let masked = "";
  let i = 0;
  let lastNonWhitespace = "";
  let previousNonWhitespace = "";
  let lastCodeWord = "";
  let openQuote: string | null = null;

  const blank = (char: string): void => {
    masked += char === "\n" || char === "\r" ? char : " ";
  };
  const consumeQuoted = (quote: string): void => {
    openQuote = quote;
    blank(source[i++]);
    while (i < source.length) {
      const char = source[i];
      blank(char);
      i++;
      if (char === "\\" && i < source.length) {
        blank(source[i++]);
      } else if (char === quote) {
        openQuote = null;
        break;
      }
    }
  };
  const regexStartsHere = (): boolean =>
    lastNonWhitespace === "" ||
    "(,=:[!&|?{};*%~^<>".includes(lastNonWhitespace) ||
    ((lastNonWhitespace === "+" || lastNonWhitespace === "-") && previousNonWhitespace !== lastNonWhitespace) ||
    lastCodeWord === "return" ||
    lastCodeWord === "typeof";
  const consumeRegex = (): void => {
    let inCharacterClass = false;
    blank(source[i++]);
    while (i < source.length) {
      const char = source[i];
      blank(char);
      i++;
      if (char === "\\" && i < source.length) {
        blank(source[i++]);
      } else if (char === "[") {
        inCharacterClass = true;
      } else if (char === "]") {
        inCharacterClass = false;
      } else if (char === "/" && !inCharacterClass) {
        break;
      }
    }
  };

  while (i < source.length) {
    const char = source[i];
    if (char === "/" && source[i + 1] === "/") {
      blank(source[i++]);
      blank(source[i++]);
      while (i < source.length && source[i] !== "\n") blank(source[i++]);
      continue;
    }
    if (char === "/" && source[i + 1] === "*") {
      blank(source[i++]);
      blank(source[i++]);
      while (i < source.length && !(source[i] === "*" && source[i + 1] === "/")) blank(source[i++]);
      if (i < source.length) blank(source[i++]);
      if (i < source.length) blank(source[i++]);
      continue;
    }
    if (char === "/" && regexStartsHere()) {
      consumeRegex();
      continue;
    }
    if (char === "'" || char === '"' || char === "`") {
      consumeQuoted(char);
      continue;
    }
    masked += char;
    if (!/\s/.test(char)) {
      previousNonWhitespace = lastNonWhitespace;
      lastNonWhitespace = char;
      if (/[A-Za-z_$0-9]/.test(char)) {
        lastCodeWord += char;
      } else {
        lastCodeWord = "";
      }
    }
    i++;
  }
  if (openQuote !== null) {
    throw new Error(`BANC-01: chaîne ou template non fermé (${openQuote})`);
  }
  return masked;
}

type RelativeDependency = { binding: string | null; path: string };

function firstRelativePath(source: string): string | null {
  const match = /["'](?<path>\.{1,2}\/[^"]*?)["']/.exec(source);
  return match?.groups?.path ?? null;
}

function bindingIn(clause: string): string | null {
  return clause.match(/^(?:type\s+)?([A-Za-z_$][\w$]*)\s*(?:,|$)/)?.[1] ?? null;
}

function relativeImportsIn(source: string): RelativeDependency[] {
  const masked = stripCommentsAndStringsPreservingLength(source);
  const declarations = [...masked.matchAll(/(?:^|\n)[\t ]*(?:import|export)\b/g)].map(
    (match) => match.index! + match[0].lastIndexOf(match[0].trim()),
  );
  const imports: RelativeDependency[] = [];

  for (let index = 0; index < declarations.length; index++) {
    const start = declarations[index];
    const next = declarations[index + 1] ?? source.length;
    const semicolon = masked.indexOf(";", start);
    const end = semicolon >= 0 && semicolon < next ? semicolon + 1 : next;
    const raw = source.slice(start, end);
    const clean = masked.slice(start, end);

    if (/^import\s*\(/.test(clean)) {
      const path = firstRelativePath(raw);
      if (path) imports.push({ binding: null, path });
      continue;
    }

    const staticDeclaration = /^(?<kind>import|export)\s+(?:(?<clause>[\s\S]*?)\s+from\s+)?["'](?<path>\.{1,2}\/[^"]*?)["']/.exec(raw);
    if (!staticDeclaration || !/^(?:import|export)\b/.test(clean)) continue;

    const clause = staticDeclaration.groups?.clause?.trim() ?? "";
    const binding = staticDeclaration.groups?.kind === "import" && clause ? bindingIn(clause) : null;
    imports.push({ binding, path: staticDeclaration.groups?.path ?? "" });
  }

  for (const dynamic of masked.matchAll(/\bimport\s*\(/g)) {
    const start = dynamic.index!;
    const raw = source.slice(start, source.indexOf(")", start) + 1);
    const path = firstRelativePath(raw);
    if (path) imports.push({ binding: null, path });
  }
  return imports;
}

/**
 * Extract the relative dependencies and explicitly registered graphical suites
 * from e2e/run.ts. This is deliberately pure so its parsing decisions are
 * tested by value below; the disk guard only delegates to it.
 */
export function suitesDeclaredIn(source: string): {
  imports: RelativeDependency[];
  registered: string[];
} {
  const masked = stripCommentsAndStringsPreservingLength(source);
  const suiteAnchor = /\bconst\s+SUITES\s*:\s*Suite\[\]\s*=\s*\[/.exec(masked);
  if (!suiteAnchor || suiteAnchor.index === undefined) {
    throw new Error("BANC-01: ancre const SUITES: Suite[] = [...] introuvable dans e2e/run.ts");
  }

  const arrayStart = suiteAnchor.index + suiteAnchor[0].length;
  let depth = 1;
  let arrayEnd = arrayStart;
  while (arrayEnd < masked.length && depth > 0) {
    if (masked[arrayEnd] === "[") depth++;
    if (masked[arrayEnd] === "]") depth--;
    arrayEnd++;
  }
  if (depth !== 0) {
    throw new Error("BANC-01: fermeture de SUITES introuvable dans e2e/run.ts");
  }

  return {
    imports: relativeImportsIn(source),
    registered: masked.slice(arrayStart, arrayEnd - 1).match(/[A-Za-z_$][\w$]*/g) ?? [],
  };
}

function assertValue(condition: unknown, message: string): void {
  assert(condition, `BANC-01 valeur: ${message}`);
  cases++;
}

function assertThrows(run: () => void, message: string): void {
  let thrown = false;
  try {
    run();
  } catch {
    thrown = true;
  }
  assertValue(thrown, message);
}

function firstImportPathFrom(source: string): string | null {
  try {
    return suitesDeclaredIn(source).imports[0]?.path ?? null;
  } catch {
    return null;
  }
}

const VALUE_SUITE = "\nconst SUITES: Suite[] = [alpha];\n";

{
  const declared = suitesDeclaredIn(`import alpha from './tests/alpha.e2e.ts';${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.binding === "alpha", "import par défaut extrait");
}
{
  const declared = suitesDeclaredIn(`import { beta } from './tests/beta.e2e.ts';${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.binding === null, "import à accolades extrait sans binding par défaut");
}
{
  const declared = suitesDeclaredIn(`import * as gamma from './tests/gamma.e2e.ts';${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.binding === null, "import namespace extrait sans binding par défaut");
}
{
  const declared = suitesDeclaredIn(`import delta, { helper } from './tests/delta.e2e.ts';${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.binding === "delta", "import mixte extrait avec son binding par défaut");
}
{
  const declared = suitesDeclaredIn(`import type { Epsilon } from './tests/epsilon.e2e.ts';${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.binding === null, "import type à accolades extrait sans binding par défaut");
}
{
  const declared = suitesDeclaredIn(`import zeta,\n  { helper } from './tests/zeta.e2e.ts';${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.path === "./tests/zeta.e2e.ts", "import sur deux lignes extrait");
}
{
  const declared = suitesDeclaredIn(`import eta from './tests/eta.e2e.ts'${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.binding === "eta", "import sans point-virgule extrait");
}
{
  const declared = suitesDeclaredIn(
    "import alpha from './tests/alpha.e2e.ts';\nimport beta from './tests/beta.e2e.ts';\nconst SUITES: Suite[] = [alpha];",
  );
  assertValue(
    declared.registered.length === 1 && declared.registered[0] === "alpha",
    "SUITES amputée reste visible comme telle",
  );
}
assertThrows(
  () => suitesDeclaredIn("import alpha from './tests/alpha.e2e.ts';"),
  "SUITES absente doit faire échouer l'analyse",
);
{
  const masked = stripCommentsAndStringsPreservingLength("before /* hidden import */ after");
  assertValue(!masked.includes("hidden"), "le dépouilleur retire les commentaires de bloc par valeur");
}
{
  const declared = suitesDeclaredIn(
    "import { performance } from 'node:perf_hooks';\nimport integrity from './tests/integrity.e2e.ts';\nconst SUITES: Suite[] = [];",
  );
  assertValue(
    declared.imports.length === 1 && declared.imports[0]?.binding === "integrity",
    "un import node: placé avant une suite relative ne l'avale pas",
  );
}
{
  const declared = suitesDeclaredIn(
    "import integrity from './tests/integrity.e2e.ts';\nconst SUITES: Suite[] = [/* integrity, */ alpha];",
  );
  assertValue(!declared.registered.includes("integrity"), "une suite commentée ne satisfait pas R2 ou R3");
}
{
  const declared = suitesDeclaredIn(
    "/* import gate from './tests/gate.e2e.ts'; */\nconst text = `import mod from './pas-un-vrai-fichier.ts';`;\nconst SUITES: Suite[] = [];",
  );
  assertValue(declared.imports.length === 0, "un import de commentaire ou de template literal est ignoré");
}
{
  const declared = suitesDeclaredIn(
    "export { helper } from './barrel.ts';\nimport './side-effect.ts';\nawait import('./dynamic.ts');\nconst SUITES: Suite[] = [];",
  );
  assertValue(
    declared.imports.map((entry) => entry.path).join(",") === "./barrel.ts,./side-effect.ts,./dynamic.ts",
    "export from, import sans clause et import dynamique sont tous extraits",
  );
}
{
  const declared = suitesDeclaredIn(`const re = /don't/;\nimport alpha from './tests/alpha.e2e.ts';${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.path === "./tests/alpha.e2e.ts", "une regex avec apostrophe ne masque pas l'import suivant");
}
{
  const declared = suitesDeclaredIn(`const re = /["']/;\nimport alpha from './tests/alpha.e2e.ts';${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.path === "./tests/alpha.e2e.ts", "une regex avec guillemet ne masque pas l'import suivant");
}
{
  assertValue(
    firstImportPathFrom(`const a = txt.match(/don't/);\nimport alpha from './tests/alpha.e2e.ts';${VALUE_SUITE}`) === "./tests/alpha.e2e.ts",
    "la reconnaissance regex après parenthèse avec apostrophe conserve l'import suivant",
  );
}
{
  assertValue(
    firstImportPathFrom(`const a = txt.match(/["']/);\nimport alpha from './tests/alpha.e2e.ts';${VALUE_SUITE}`) === "./tests/alpha.e2e.ts",
    "la reconnaissance regex après parenthèse avec guillemet conserve l'import suivant",
  );
}
{
  assertValue(
    firstImportPathFrom(`const re = /\\//;\nimport alpha from './tests/alpha.e2e.ts';${VALUE_SUITE}`) === "./tests/alpha.e2e.ts",
    "un slash échappé dans une regex conserve l'import suivant",
  );
}
assertThrows(
  () => stripCommentsAndStringsPreservingLength("const label = 'moitie"),
  "une chaîne impaire doit faire échouer le filet de fermeture",
);
{
  const masked = stripCommentsAndStringsPreservingLength("let tick = 0; tick++;\nconst half = tick++ / 2;\nconst label = 'moitie';");
  assertValue(masked.includes("const label = ") && !masked.includes("moitie"), "tick++ / 2 reste une division et la chaîne suivante est masquée");
}
{
  const masked = stripCommentsAndStringsPreservingLength("const r = a / b; const s = 'x';");
  assertValue(masked.includes("const s = ") && !masked.includes("x"), "une division ne devient pas une regex et la chaîne suivante est masquée");
}
{
  const declared = suitesDeclaredIn(`const re = /[a-z/]'\/;\nimport alpha from './tests/alpha.e2e.ts';${VALUE_SUITE}`);
  assertValue(declared.imports[0]?.path === "./tests/alpha.e2e.ts", "un slash dans une classe regex ne ferme pas le littéral");
}
const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const E2E_DIR = join(ROOT, "e2e");
const RUNNER_PATH = join(E2E_DIR, "run.ts");

function everyTypeScriptFile(directory: string): string[] {
  const entries = readdirSync(directory, { withFileTypes: true }) as unknown as {
    name: string;
    isDirectory(): boolean;
    isFile(): boolean;
  }[];
  return entries.flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return everyTypeScriptFile(path);
    return entry.isFile() && entry.name.endsWith(".ts") ? [path] : [];
  });
}

function projectPath(path: string): string {
  return relative(ROOT, path).replaceAll("\\", "/");
}

function assertExistingImports(importer: string, imports: { path: string }[]): void {
  for (const imported of imports) {
    const pathOnDisk = resolve(dirname(importer), imported.path);
    assert(
      existsSync(pathOnDisk),
      `BANC-01 R1: le fichier absent ${projectPath(pathOnDisk)} est importé par ${projectPath(importer)}`,
    );
  }
}

export const e2eImportsSuite = (async () => {
  const runner = readFileSync(RUNNER_PATH, "utf8");
  const declared = suitesDeclaredIn(runner);

  assert(declared.imports.length > 0, "BANC-01 R4: aucun import relatif extrait de e2e/run.ts");
  for (const file of everyTypeScriptFile(E2E_DIR)) {
    const source = file === RUNNER_PATH ? runner : readFileSync(file, "utf8");
    const imports = file === RUNNER_PATH ? declared.imports : relativeImportsIn(source);
    assertExistingImports(file, imports);
  }
  for (const imported of declared.imports.filter((entry) => entry.path.startsWith("./tests/") && entry.binding)) {
    assert(
      declared.registered.includes(imported.binding!),
      `BANC-01 R2: la suite ${imported.binding} est importée mais jamais enregistrée, donc jamais exécutée`,
    );
  }
  for (const suite of declared.registered) {
    assert(
      declared.imports.some((entry) => entry.binding === suite),
      `BANC-01 R3: la suite ${suite} est enregistrée mais son import est absent`,
    );
  }

  cases++;
})();

export const __e2eImportsRan = true;
export const e2eImportsCases = () => cases;

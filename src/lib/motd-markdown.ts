/**
 * Le dialecte markdown du message du jour, réduit à un arbre.
 *
 * Fonction pure, sans import — testée par valeur dans
 * `scripts/test-motd-markdown.ts`, pas par une garde textuelle (piège n°54).
 *
 * Pourquoi un analyseur maison plutôt qu'une bibliothèque : la charte interdit
 * toute dépendance npm ajoutée sans décision, et surtout `{@html}` est banni de
 * tout `.svelte` du dépôt (garde N6 de test-news-wiring.ts). Un rendu markdown
 * classique produit du HTML ; celui-ci produit un ARBRE que `MotdMarkdown.svelte`
 * rend nœud par nœud avec de vrais éléments Svelte. Le texte tiers ne devient
 * jamais du balisage, quelle que soit sa forme.
 *
 * Le dialecte, volontairement petit — c'est l'auteur qui écrit, pas le public :
 *
 *   # Titre / ## Sous-titre / ### Intertitre
 *   - puce  ou  * puce            1. numéro
 *   > citation                    ---  (filet)
 *   **gras**  *italique* ou _italique_  `code`
 *   [texte](https://…)            — https uniquement, tout autre schéma reste du texte
 *   {rose}texte{/}                — ton nommé : mint peach rose lilac sky
 *                                   alias : green orange red purple blue yellow
 *   {#ff8800}texte{/}             — couleur hexadécimale à six chiffres
 *
 * Un saut de ligne simple à l'intérieur d'un paragraphe est un retour à la ligne
 * (pas un espace comme en CommonMark) : l'auteur d'une notice écrit comme dans
 * un message, pas comme dans un article. Une ligne vide sépare deux blocs.
 *
 * Marqueur non refermé ou ton inconnu : le texte est rendu TEL QUEL, marqueur
 * compris. Le défaut se voit à l'écran au lieu de disparaître — même politique
 * que `richSegments`.
 */

export type Inline =
  | { kind: "text"; text: string }
  | { kind: "break" }
  | { kind: "code"; text: string }
  | { kind: "strong"; children: Inline[] }
  | { kind: "em"; children: Inline[] }
  | { kind: "link"; href: string; children: Inline[] }
  | { kind: "color"; color: MotdColor; children: Inline[] };

export type MotdColor = { tone: MotdTone } | { hex: string };

export type MotdTone = "mint" | "peach" | "rose" | "lilac" | "sky";

export type Block =
  | { kind: "heading"; level: 1 | 2 | 3; children: Inline[] }
  | { kind: "paragraph"; children: Inline[] }
  | { kind: "list"; ordered: boolean; items: Inline[][] }
  | { kind: "quote"; children: Inline[] }
  | { kind: "rule" };

/**
 * Les tons acceptés. Les alias existent parce que l'auteur pense « rouge »
 * quand il écrit une panne, et que la charte ne connaît que `rose` ; les deux
 * doivent mener au même ton, celui qui porte déjà ce sens dans l'application.
 */
const TONES: Record<string, MotdTone> = {
  mint: "mint",
  peach: "peach",
  rose: "rose",
  lilac: "lilac",
  sky: "sky",
  green: "mint",
  orange: "peach",
  yellow: "peach",
  red: "rose",
  purple: "lilac",
  blue: "sky",
};

const HEX = /^#[0-9a-fA-F]{6}$/;

/** `{rose}` / `{#ff8800}` → couleur, ou null si le nom n'est pas admis. */
export function parseColorName(name: string): MotdColor | null {
  if (HEX.test(name)) return { hex: name.toLowerCase() };
  const tone = TONES[name.toLowerCase()];
  return tone ? { tone } : null;
}

/** Seuls les liens `https://` sont des liens ; le reste est du texte. */
export function isSafeHref(href: string): boolean {
  return /^https:\/\/[^\s<>"']+$/i.test(href);
}

// ─────────────────────────────────────────────────────────────── inline

const OPEN_COLOR = /^\{(#[0-9a-fA-F]{6}|[a-zA-Z]+)\}/;
const CLOSE_COLOR = "{/}";

/**
 * Cherche la fermeture d'une couleur ouverte à `from` (juste après `{ton}`),
 * en tenant compte des couleurs imbriquées. Retourne l'index de `{/}` ou -1.
 */
function findColorClose(src: string, from: number): number {
  let depth = 0;
  let i = from;
  while (i < src.length) {
    if (src.startsWith(CLOSE_COLOR, i)) {
      if (depth === 0) return i;
      depth--;
      i += CLOSE_COLOR.length;
      continue;
    }
    const open = OPEN_COLOR.exec(src.slice(i));
    if (open && parseColorName(open[1]) !== null) {
      depth++;
      i += open[0].length;
      continue;
    }
    i++;
  }
  return -1;
}

/** Fermeture d'un délimiteur simple (`**`, `*`, `_`, `` ` ``) hors position 0. */
function findClose(src: string, from: number, delim: string): number {
  const at = src.indexOf(delim, from);
  // Vide (`****`) n'est pas une emphase : il n'y a rien à mettre en valeur.
  return at > from ? at : -1;
}

/** Réunit les textes contigus — un arbre avec `text`,`text` est un défaut d'analyse. */
function pushText(out: Inline[], text: string): void {
  if (text === "") return;
  const last = out[out.length - 1];
  if (last && last.kind === "text") {
    last.text += text;
  } else {
    out.push({ kind: "text", text });
  }
}

export function parseInline(src: string): Inline[] {
  const out: Inline[] = [];
  let i = 0;
  let plain = "";

  const flush = () => {
    pushText(out, plain);
    plain = "";
  };

  while (i < src.length) {
    const ch = src[i];

    // Retour à la ligne dur : un saut de ligne simple dans un bloc.
    if (ch === "\n") {
      flush();
      out.push({ kind: "break" });
      i++;
      continue;
    }

    // Échappement : `\*` rend un astérisque littéral.
    if (ch === "\\" && i + 1 < src.length && "*_`[]{}\\#".includes(src[i + 1])) {
      plain += src[i + 1];
      i += 2;
      continue;
    }

    // Code : aucune analyse à l'intérieur.
    if (ch === "`") {
      const end = findClose(src, i + 1, "`");
      if (end !== -1) {
        flush();
        out.push({ kind: "code", text: src.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }

    // Gras. Devant `***`, la fermeture est la FIN de la série : ainsi
    // `**gras *et italique***` referme l'italique avant le gras.
    if (src.startsWith("**", i)) {
      let end = findClose(src, i + 2, "**");
      while (end !== -1 && src[end + 2] === "*") end++;
      if (end !== -1) {
        flush();
        out.push({ kind: "strong", children: parseInline(src.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }

    // Italique : `*` ou `_`, fermé par le même caractère. Le `_` n'ouvre
    // qu'en début de mot — `snake_case_name` reste intact.
    if (ch === "*" || (ch === "_" && (i === 0 || /\s|[([{]/.test(src[i - 1])))) {
      const end = findClose(src, i + 1, ch);
      if (end !== -1 && !/\s/.test(src[i + 1]) && (ch === "*" || end + 1 >= src.length || /[\s.,;:!?)\]}]/.test(src[end + 1]))) {
        flush();
        out.push({ kind: "em", children: parseInline(src.slice(i + 1, end)) });
        i = end + 1;
        continue;
      }
    }

    // Lien : [texte](https://…) — les crochets peuvent contenir de l'emphase,
    // pas d'autre lien.
    if (ch === "[") {
      const closeBracket = src.indexOf("](", i + 1);
      if (closeBracket !== -1) {
        const closeParen = src.indexOf(")", closeBracket + 2);
        const label = src.slice(i + 1, closeBracket);
        if (closeParen !== -1 && label.length > 0 && !label.includes("[")) {
          const href = src.slice(closeBracket + 2, closeParen).trim();
          if (isSafeHref(href)) {
            flush();
            out.push({ kind: "link", href, children: parseInline(label) });
            i = closeParen + 1;
            continue;
          }
        }
      }
    }

    // Couleur : {ton}…{/}.
    if (ch === "{") {
      const open = OPEN_COLOR.exec(src.slice(i));
      if (open) {
        const color = parseColorName(open[1]);
        if (color !== null) {
          const start = i + open[0].length;
          const end = findColorClose(src, start);
          if (end !== -1 && end > start) {
            flush();
            out.push({ kind: "color", color, children: parseInline(src.slice(start, end)) });
            i = end + CLOSE_COLOR.length;
            continue;
          }
        }
      }
    }

    plain += ch;
    i++;
  }
  flush();
  return out;
}

// ─────────────────────────────────────────────────────────────── blocks

const HEADING = /^(#{1,3})\s+(.*\S)\s*$/;
const BULLET = /^[-*]\s+(.*)$/;
const NUMBERED = /^\d+[.)]\s+(.*)$/;
const QUOTE = /^>\s?(.*)$/;
const RULE = /^(-{3,}|\*{3,}|_{3,})\s*$/;

export function parseMotd(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let paragraph: string[] = [];

  const flushParagraph = () => {
    if (paragraph.length === 0) return;
    blocks.push({ kind: "paragraph", children: parseInline(paragraph.join("\n")) });
    paragraph = [];
  };

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    if (trimmed === "") {
      flushParagraph();
      i++;
      continue;
    }

    // Le filet passe AVANT la puce : `---` commence par un tiret.
    if (RULE.test(trimmed)) {
      flushParagraph();
      blocks.push({ kind: "rule" });
      i++;
      continue;
    }

    const heading = HEADING.exec(trimmed);
    if (heading) {
      flushParagraph();
      blocks.push({
        kind: "heading",
        level: heading[1].length as 1 | 2 | 3,
        children: parseInline(heading[2]),
      });
      i++;
      continue;
    }

    const bullet = BULLET.exec(trimmed);
    const numbered = NUMBERED.exec(trimmed);
    if (bullet || numbered) {
      flushParagraph();
      const ordered = numbered !== null && bullet === null;
      const items: Inline[][] = [];
      while (i < lines.length) {
        const t = lines[i].trim();
        const m = ordered ? NUMBERED.exec(t) : BULLET.exec(t);
        if (!m) break;
        // Une ligne indentée qui suit une puce la continue.
        let text = m[1];
        while (i + 1 < lines.length && /^\s{2,}\S/.test(lines[i + 1]) && !BULLET.test(lines[i + 1].trim()) && !NUMBERED.test(lines[i + 1].trim())) {
          text += "\n" + lines[i + 1].trim();
          i++;
        }
        items.push(parseInline(text));
        i++;
      }
      blocks.push({ kind: "list", ordered, items });
      continue;
    }

    if (QUOTE.test(trimmed)) {
      flushParagraph();
      const parts: string[] = [];
      while (i < lines.length) {
        const m = QUOTE.exec(lines[i].trim());
        if (!m) break;
        parts.push(m[1]);
        i++;
      }
      blocks.push({ kind: "quote", children: parseInline(parts.join("\n")) });
      continue;
    }

    paragraph.push(trimmed);
    i++;
  }
  flushParagraph();
  return blocks;
}

/** Le texte brut d'un arbre — pour les tests et les libellés d'accessibilité. */
export function plainText(nodes: Inline[]): string {
  return nodes
    .map((n) => {
      switch (n.kind) {
        case "text":
        case "code":
          return n.text;
        case "break":
          return "\n";
        default:
          return plainText(n.children);
      }
    })
    .join("");
}

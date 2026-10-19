/**
 * MOTD — tests par valeur de `src/lib/motd-markdown.ts` et `src/lib/motd.ts`.
 *
 * Aucune garde textuelle ici : les deux modules sont purs, donc ce sont leurs
 * RÉSULTATS qui sont épinglés (piège n°54). La forme est celle de
 * `test-search-state.ts` : promesse exportée, compteur incrémenté à la fin de
 * chaque cas, et sentinelle `beforeExit` — un arrêt silencieux ne doit jamais
 * se lire comme un succès (piège n°50).
 *
 * `motd.ts` importe un TYPE depuis `api.ts` ; un `import type` disparaît à la
 * compilation, donc le module se charge sans Tauri ni shim.
 */

import { parseMotd, parseInline, parseColorName, isSafeHref, plainText } from "../src/lib/motd-markdown";
import type { Block, Inline } from "../src/lib/motd-markdown";
import { actionLabel, actionNeedsConfirm, confirmKeyOf, isForced, motdSeverity, presentationOf, resolveMotdText, shouldOpenMotdAtStartup, showsMainButton, steamDowngradeOffer } from "../src/lib/motd";
import type { MotdAction, MotdActionKind, MotdActionState, MotdMessage, Workaround } from "../src/lib/api";

function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a as object);
  const kb = Object.keys(b as object);
  if (ka.length !== kb.length) return false;
  return ka.every((k) => deepEqual((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
}

const assert = {
  ok(cond: unknown, msg?: string): void {
    if (!cond) throw new Error(msg ?? `valeur attendue vraie, obtenu ${cond}`);
  },
  equal(actual: unknown, expected: unknown, msg?: string): void {
    if (actual !== expected) throw new Error(msg ?? `attendu ${JSON.stringify(expected)}, obtenu ${JSON.stringify(actual)}`);
  },
  deep(actual: unknown, expected: unknown, msg?: string): void {
    if (!deepEqual(actual, expected)) {
      throw new Error(`${msg ?? "structures différentes"}\n  attendu : ${JSON.stringify(expected)}\n  obtenu  : ${JSON.stringify(actual)}`);
    }
  },
};

let cases = 0;
const EXPECTED_CASES = 40;
function label(name: string): void {
  cases++;
  console.log(`  ✓ ${name}`);
}
export const motdCases = () => cases;

const text = (t: string): Inline => ({ kind: "text", text: t });
const para = (...children: Inline[]): Block => ({ kind: "paragraph", children });

const message = (id: string): MotdMessage => ({
  id,
  published_at: "2026-09-16T10:00:00Z",
  expires_at: "2026-09-19T10:00:00Z",
  severity: "info",
  title: { fr: "Titre" },
  body: { fr: "Corps" },
  actions: [],
  persistent: false,
});

export const motdSuite: Promise<void> = (async () => {
  try {
    console.log("\n── MOTD : markdown ──");

    // ── inline ──

    assert.deep(parseInline("bonjour"), [text("bonjour")]);
    label("texte brut → un seul nœud texte");

    assert.deep(parseInline("le **serveur** est"), [
      text("le "),
      { kind: "strong", children: [text("serveur")] },
      text(" est"),
    ]);
    label("**gras** découpé comme richSegments");

    assert.deep(parseInline("a *b* c _d_ e"), [
      text("a "),
      { kind: "em", children: [text("b")] },
      text(" c "),
      { kind: "em", children: [text("d")] },
      text(" e"),
    ]);
    label("*italique* et _italique_");

    assert.deep(parseInline("snake_case_name reste"), [text("snake_case_name reste")]);
    label("un _ au milieu d'un mot n'ouvre rien");

    assert.deep(parseInline("un `code` ici"), [text("un "), { kind: "code", text: "code" }, text(" ici")]);
    label("`code` en ligne");

    assert.deep(parseInline("x `**pas gras**` y"), [text("x "), { kind: "code", text: "**pas gras**" }, text(" y")]);
    label("aucune analyse à l'intérieur du code");

    assert.deep(parseInline("**gras *et italique***"), [
      { kind: "strong", children: [text("gras "), { kind: "em", children: [text("et italique")] }] },
    ]);
    label("emphases imbriquées");

    assert.deep(parseInline("mal **fermé"), [text("mal **fermé")]);
    label("marqueur non refermé rendu tel quel, marqueur compris");

    assert.deep(parseInline("vide **** ici"), [text("vide **** ici")]);
    label("emphase vide : pas une emphase");

    assert.deep(parseInline("\\*pas italique\\*"), [text("*pas italique*")]);
    label("échappement \\* rend un astérisque littéral");

    // ── liens ──

    assert.deep(parseInline("voir [le statut](https://status.example.com/x) svp"), [
      text("voir "),
      { kind: "link", href: "https://status.example.com/x", children: [text("le statut")] },
      text(" svp"),
    ]);
    label("[texte](https://…) devient un lien");

    assert.deep(parseInline("[x](http://evil.example)"), [text("[x](http://evil.example)")]);
    assert.deep(parseInline("[x](javascript:alert(1))"), [text("[x](javascript:alert(1))")]);
    assert.deep(parseInline("[x](file:///C:/Windows)"), [text("[x](file:///C:/Windows)")]);
    label("http:, javascript:, file: — jamais des liens, toujours du texte");

    assert.ok(isSafeHref("https://a.b/c?d=e#f"));
    assert.ok(!isSafeHref("https://a.b/c d"));
    assert.ok(!isSafeHref('https://a.b/"onclick'));
    assert.ok(!isSafeHref("HTTP://a.b"));
    label("isSafeHref : https seul, sans espace ni guillemet");

    // ── couleurs ──

    assert.deep(parseColorName("rose"), { tone: "rose" });
    assert.deep(parseColorName("red"), { tone: "rose" });
    assert.deep(parseColorName("Green"), { tone: "mint" });
    assert.deep(parseColorName("#FF8800"), { hex: "#ff8800" });
    assert.equal(parseColorName("magenta"), null);
    assert.equal(parseColorName("#fff"), null);
    assert.equal(parseColorName("#gg0000"), null);
    label("parseColorName : tons, alias, hex à six chiffres ; le reste refusé");

    assert.deep(parseInline("état : {red}en panne{/} depuis"), [
      text("état : "),
      { kind: "color", color: { tone: "rose" }, children: [text("en panne")] },
      text(" depuis"),
    ]);
    label("{red}…{/} colore le segment");

    assert.deep(parseInline("{#00aaff}bleu{/}"), [
      { kind: "color", color: { hex: "#00aaff" }, children: [text("bleu")] },
    ]);
    label("{#hex}…{/} couleur libre");

    assert.deep(parseInline("{magenta}x{/}"), [text("{magenta}x{/}")]);
    assert.deep(parseInline("{red}jamais fermé"), [text("{red}jamais fermé")]);
    assert.deep(parseInline("{red}{/}"), [text("{red}{/}")]);
    label("ton inconnu, non refermé ou vide : texte tel quel");

    assert.deep(parseInline("{red}a {blue}b{/} c{/}"), [
      {
        kind: "color",
        color: { tone: "rose" },
        children: [text("a "), { kind: "color", color: { tone: "sky" }, children: [text("b")] }, text(" c")],
      },
    ]);
    label("couleurs imbriquées : chaque {/} ferme la sienne");

    assert.deep(parseInline("{red}**gras rouge**{/}"), [
      { kind: "color", color: { tone: "rose" }, children: [{ kind: "strong", children: [text("gras rouge")] }] },
    ]);
    label("emphase à l'intérieur d'une couleur");

    assert.deep(parseInline("{count} jeux"), [text("{count} jeux")]);
    label("une accolade qui n'est pas un ton reste du texte");

    // ── blocs ──

    assert.deep(parseMotd("un\ndeux"), [para(text("un"), { kind: "break" }, text("deux"))]);
    label("saut de ligne simple = retour à la ligne dans le paragraphe");

    assert.deep(parseMotd("un\n\ndeux"), [para(text("un")), para(text("deux"))]);
    label("ligne vide = nouveau paragraphe");

    assert.deep(parseMotd("# Titre\n## Sous\n### Inter\n#### pas un titre"), [
      { kind: "heading", level: 1, children: [text("Titre")] },
      { kind: "heading", level: 2, children: [text("Sous")] },
      { kind: "heading", level: 3, children: [text("Inter")] },
      para(text("#### pas un titre")),
    ]);
    label("titres # ## ### ; #### reste un paragraphe");

    assert.deep(parseMotd("- a\n* b\n  suite\n- c"), [
      { kind: "list", ordered: false, items: [[text("a")], [text("b"), { kind: "break" }, text("suite")], [text("c")]] },
    ]);
    label("liste à puces, ligne indentée = continuation");

    assert.deep(parseMotd("1. a\n2. b\n\ntexte"), [
      { kind: "list", ordered: true, items: [[text("a")], [text("b")]] },
      para(text("texte")),
    ]);
    label("liste numérotée puis paragraphe");

    assert.deep(parseMotd("> l1\n> l2\n---\nfin"), [
      { kind: "quote", children: [text("l1"), { kind: "break" }, text("l2")] },
      { kind: "rule" },
      para(text("fin")),
    ]);
    label("citation sur deux lignes, filet, paragraphe");

    assert.deep(parseMotd("a\r\nb\r\n\r\nc"), [para(text("a"), { kind: "break" }, text("b")), para(text("c"))]);
    label("CRLF normalisé");

    assert.deep(parseMotd(""), []);
    assert.deep(parseMotd("\n\n  \n"), []);
    label("source vide → aucun bloc");

    assert.equal(plainText(parseInline("a **b** {red}c{/} [d](https://e.f)")), "a b c d");
    label("plainText retire tout balisage");

    // ── motd.ts ──

    assert.equal(motdSeverity("warning"), "warning");
    assert.equal(motdSeverity("critical"), "critical");
    assert.equal(motdSeverity("info"), "info");
    assert.equal(motdSeverity("urgent"), "info");
    assert.equal(motdSeverity(null), "info");
    assert.equal(motdSeverity(undefined), "info");
    label("motdSeverity : inconnu → info");

    assert.equal(resolveMotdText({ fr: "Bonjour", en: "Hello" }, "fr"), "Bonjour");
    assert.equal(resolveMotdText({ fr: "Bonjour", en: "Hello" }, "en"), "Hello");
    assert.equal(resolveMotdText({ fr: "Bonjour", en: "Hello" }, "de"), "Hello");
    assert.equal(resolveMotdText({ fr: "Bonjour" }, "en"), "Bonjour");
    assert.equal(resolveMotdText({ es: "Hola" }, "fr"), "Hola");
    assert.equal(resolveMotdText({ fr: "  ", en: "Hello" }, "fr"), "Hello");
    assert.equal(resolveMotdText({}, "fr"), "");
    assert.equal(resolveMotdText(null, "fr"), "");
    label("resolveMotdText : locale, puis en, puis fr, puis n'importe laquelle");

    assert.equal(shouldOpenMotdAtStartup(null, null, null), false);
    assert.equal(shouldOpenMotdAtStartup(message("m1"), null, null), true);
    assert.equal(shouldOpenMotdAtStartup(message("m1"), "m1", null), false);
    assert.equal(shouldOpenMotdAtStartup(message("m2"), "m1", null), true);
    assert.equal(shouldOpenMotdAtStartup(message("m1"), null, "m1"), false);
    assert.equal(shouldOpenMotdAtStartup(message("m2"), null, "m1"), true);
    label("shouldOpenMotdAtStartup : jamais l'id oublié, jamais deux fois par session");

    // ── boutons d'action ──

    const KINDS: MotdActionKind[] = ["steam_downgrade", "steam_undo", "restart_steam", "repair_tools", "open_url", "steamtools_source"];
    assert.deep(
      KINDS.map((k) => actionNeedsConfirm(k)),
      [true, true, true, true, false, true],
    );
    label("actionNeedsConfirm : tout ce qui ferme, relance ou réécrit demande un second clic, pas l'ouverture d'une page");

    // La phrase du second clic ne dit « Steam sera fermé » que pour ce qui le ferme vraiment.
    assert.deep(
      KINDS.map((k) => confirmKeyOf(k)),
      ["motd.action.confirm", "motd.action.confirm", "motd.action.confirm", "motd.action.confirm", "motd.action.confirm_light", "motd.action.confirm_light"],
    );
    label("confirmKeyOf : seuls les gestes qui ferment Steam promettent de le fermer");

    const act = (id: string, kind: MotdActionKind, labelMap: Record<string, string> = {}): MotdAction => ({ id, kind, label: labelMap });
    assert.equal(actionLabel(act("a", "restart_steam", { fr: "Relancer", en: "Restart" }), "en"), "Restart");
    assert.equal(actionLabel(act("a", "restart_steam", { fr: "Relancer" }), "en"), "Relancer");
    assert.equal(actionLabel(act("a", "restart_steam"), "fr"), "", "vide : l'interface met le libellé par défaut du type");
    label("actionLabel : langue, repli, ou vide pour le libellé par défaut");

    const withActions = (...a: MotdAction[]): MotdMessage => ({ ...message("m1"), actions: a });
    assert.equal(steamDowngradeOffer(null), null);
    assert.equal(steamDowngradeOffer(message("m1")), null);
    assert.equal(steamDowngradeOffer(withActions(act("r", "restart_steam"), act("o", "open_url"))), null);
    assert.equal(steamDowngradeOffer(withActions(act("r", "restart_steam"), act("d", "steam_downgrade")))?.id, "d");
    label("steamDowngradeOffer : la carte Outils n'existe que si le message en cours porte le correctif");

    // ── message persistant et diagnostiqué ──

    const need = (needed: boolean): Workaround => ({ action_id: "fix", needed, pinned: !needed, installed_version: "1791918634" });
    const persistent: MotdMessage = { ...message("m1"), persistent: true };
    assert.equal(isForced(persistent, need(true)), true);
    assert.equal(isForced(persistent, need(false)), false, "correctif déjà appliqué ou inutile : message ordinaire");
    assert.equal(isForced(persistent, null), false, "aucun correctif proposé");
    assert.equal(isForced({ ...message("m1"), persistent: false }, need(true)), false, "non persistant : jamais forcé");
    assert.equal(isForced(null, need(true)), false);
    label("isForced : seulement un message persistant dont le correctif reste nécessaire ici");

    // « Ne plus afficher » est retiré tant que le message est forcé, et seulement alors.
    assert.equal(shouldOpenMotdAtStartup(persistent, "m1", null, true), true, "forcé : l'id écarté ne le fait pas taire");
    assert.equal(shouldOpenMotdAtStartup(persistent, "m1", null, false), false, "plus nécessaire : le choix passé tient");
    assert.equal(shouldOpenMotdAtStartup(persistent, "m1", "m1", true), false, "mais jamais deux fois par session");
    assert.equal(shouldOpenMotdAtStartup(persistent, null, null, true), true);
    assert.equal(shouldOpenMotdAtStartup(null, "m1", null, true), false);
    label("shouldOpenMotdAtStartup : forcé, l'id écarté ne compte plus, mais une seule ouverture par session");

    // Un bouton qui ne sert qu'une fois dit, après coup, qu'il a servi : valable pour tout type d'action.
    const st = (state: string) => ({ state: state as MotdActionState, ran_at: null });
    assert.equal(presentationOf(st("todo")), "button");
    assert.equal(presentationOf(st("repeatable")), "button", "un geste (relancer, ouvrir) reste toujours disponible");
    assert.equal(presentationOf(st("done")), "done");
    assert.equal(presentationOf(st("not_needed")), "not_needed");
    assert.equal(presentationOf(st("recorded")), "recorded");
    assert.equal(presentationOf(st("lost")), "lost");
    assert.equal(presentationOf(undefined), "button", "pas de réponse : jamais de bouton caché");
    assert.equal(presentationOf(null), "button");
    assert.equal(presentationOf(st("something_new")), "button", "un état inconnu d'un vieux client ne cache rien");
    label("presentationOf : l'état de chaque bouton décide de son affichage, l'inconnu retombe sur le bouton");

    assert.equal(showsMainButton("button"), true);
    assert.equal(showsMainButton("lost"), true, "plus en place : on peut le refaire");
    for (const p of ["done", "not_needed", "recorded"] as const) {
      assert.equal(showsMainButton(p), false, `${p} : pas de second appui sur le bouton principal`);
    }
    label("showsMainButton : seuls « à faire » et « n'est plus en place » montrent le bouton");

    console.log(`── MOTD : ${cases} cas ──`);
  } catch (e) {
    console.error("FATAL:", e);
    throw e;
  }
})();

export const __motdRan = true;

const proc = (globalThis as { process?: { exitCode?: number; on?: (e: string, cb: () => void) => void } }).process;
proc?.on?.("beforeExit", () => {
  if (cases !== EXPECTED_CASES) {
    console.error(
      `FATAL: test-motd-markdown s'est arrêtée après ${cases} cas sur ${EXPECTED_CASES} — arrêt silencieux, pas un succès.`,
    );
    if (proc) proc.exitCode = 1;
  }
});

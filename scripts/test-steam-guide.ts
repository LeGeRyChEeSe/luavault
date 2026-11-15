/**
 * The guide drawn in Steam. Run with: npx --yes tsx scripts/test-steam-guide.ts
 * Replayed by scripts/validate.ps1 (step "Frontend: guide dans Steam").
 *
 * 1. The engine (`src/lib/steam-guide/engine.ts`) played by value: whole runs, every detour.
 * 2. The three lists of targets agree: the engine's, Rust's (`steam_guide::TARGETS`, which
 *    refuses any other), the page script's (`TARGETS`, which resolves them).
 * 3. Every key the engine can show exists in both catalogues, within the guide's lengths, with
 *    no em dash.
 * 4. The page script `scripts/steam-guide.js` is EXECUTED against a hand-rolled DOM: the view
 *    that draws, the ring on the target, texts set as text, the words it sends back.
 *
 * No node:assert (see test-virtual-scroll.ts): a throw makes tsx exit 1.
 */
// @ts-expect-error — `node:fs` has no types here: @types/node is not a
// project dependency. See the identical note in test-card-tabs.ts.
import { readFileSync as readFileSyncRaw } from "node:fs";
const readFileSync = readFileSyncRaw as (path: URL, options: "utf8") => string;
import {
  actionKey,
  ACTIONS,
  CHOICES,
  choicesOf,
  currentPlace,
  indexOf,
  infoKey,
  INITIAL,
  needMet,
  reduce,
  STEPS,
  TARGET_VIEW,
  TARGETS,
  viewsOf,
  type Event,
  type Notice,
  type State,
  type StepId,
} from "../src/lib/steam-guide/engine";
import { fr } from "../src/lib/i18n/fr";
import { en } from "../src/lib/i18n/en";

let passed = 0;
function ok(cond: unknown, what: string): void {
  if (!cond) throw new Error("ÉCHEC : " + what);
  passed++;
}
function eq<T>(a: T, b: T, what: string): void {
  if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`ÉCHEC : ${what}\n  reçu    ${JSON.stringify(a)}\n  attendu ${JSON.stringify(b)}`);
  passed++;
}

const root = new URL("..", import.meta.url);
const read = (p: string) => readFileSync(new URL(p, root), "utf8");

// ── 1. the engine ─────────────────────────────────────────────────────
function play(events: Event[], from: State = INITIAL) {
  let s = from;
  const effects: unknown[] = [];
  for (const ev of events) {
    const out = reduce(s, ev);
    s = out.state;
    effects.push(...out.effects);
  }
  return { s, effects, step: STEPS[s.index].id as StepId };
}
const win = (place: object): Event => ({ kind: "place", from: "window", place: place as never });
const web = (place: object): Event => ({ kind: "place", from: "web", place: place as never });
const storeApp = (appId: string, owned = false, known = false) => web({ place: "store-app", appId, owned, known });
const status = (appId: string, over: Partial<{ installing: boolean; installed: boolean; patches: boolean; luaInSteam: boolean }> = {}): Event => ({
  kind: "status",
  facts: { appId, stage: "needs_steam_install", installing: false, installed: false, patches: false, luaInSteam: false, ...over },
});
const job = (appId: string, tone: string, install = true, library = false, site = false): Event => ({ kind: "job", appId, tone, install, library, site });

{
  // The whole run, as a person makes it.
  let r = play([win({ place: "window" })]);
  eq(r.step, "first-welcome", "le guide s'ouvre sur l'accueil");
  eq(choicesOf(STEPS[r.s.index], r.s), [{ id: "start", primary: true }, { id: "later", primary: false }], "C'est parti en surbrillance");
  r = play([{ kind: "choose", choice: "start" }], r.s);
  eq(r.step, "first-pill", "puis la pastille");
  r = play([{ kind: "next" }], r.s);
  eq(r.step, "first-store", "puis le magasin");
  r = play([{ kind: "choose", choice: "open-store" }], r.s);
  eq(r.effects, [{ kind: "open", place: "store" }], "Ouvrir pour moi ouvre le magasin");
  r = play([win({ place: "web-shown" }), web({ place: "store" })], r.s);
  eq(r.step, "first-search", "le magasin ouvert : chercher");
  r = play([storeApp("264710")], r.s);
  eq(r.effects, [{ kind: "lookup-free", appId: "264710" }], "une page de jeu : gratuit ou pas, demandé à Steam");
  eq(r.step, "first-search", "pas encore jugé");
  r = play([{ kind: "free", appId: "264710", free: false }], r.s);
  eq(r.step, "first-button", "un jeu payant : le bouton");
  eq(viewsOf(STEPS[r.s.index], r.s), { where: "web", bubble: "web", target: "store-find" }, "dessiné dans le magasin");
  r = play([{ kind: "fetch", appIds: ["264710"] }], r.s);
  eq(r.step, "first-check", "le clic : la vérification");
  eq(r.s.appId, "264710", "le jeu suivi");
  r = play([job("264710", "busy", false, false, true)], r.s);
  eq(r.step, "first-check", "le site ouvert : on attend les fichiers");
  r = play([job("264710", "ok", true)], r.s);
  eq(r.step, "first-panel", "fini : le panneau");
  eq(r.effects, [{ kind: "status", appId: "264710" }], "et l'état du jeu est lu (un correctif dû ?)");
  r = play([status("264710", { patches: true })], r.s);
  eq(infoKey(STEPS[r.s.index], r.s), "steamguide.step.first-panel.patches", "le correctif dû est dit, une fois");
  r = play([{ kind: "next" }], r.s);
  eq(r.step, "first-install", "puis Installer");
  r = play([{ kind: "hit", target: "store-install" }], r.s);
  eq(r.step, "first-wait", "le clic sur Installer : l'attente");
  r = play([{ kind: "tick" }], r.s);
  eq(r.effects, [{ kind: "status", appId: "264710" }], "l'attente relit l'état au battement");
  r = play([status("264710", { installing: true })], r.s);
  eq(r.step, "first-wait", "toujours en téléchargement");
  r = play([status("264710", { installed: true, patches: true })], r.s);
  eq(r.step, "first-open-library", "installé : la bibliothèque (le correctif pas encore posé)");
  r = play([{ kind: "choose", choice: "open-library" }], r.s);
  eq(r.effects, [{ kind: "open", place: "library-app:264710" }], "Ouvrir pour moi ouvre la page du jeu");
  r = play([win({ place: "library-app", appId: "264710" })], r.s);
  eq(r.step, "first-ready", "la barre Jouer");
  eq(viewsOf(STEPS[r.s.index], r.s), { where: "window", bubble: "window", target: "play-block" }, "dans la fenêtre");
  eq(infoKey(STEPS[r.s.index], r.s), "steamguide.step.first-ready.patches", "orange tant que le correctif attend");
  r = play([{ kind: "next" }, { kind: "next" }, { kind: "next" }, { kind: "next" }], r.s);
  eq(r.step, "more-end", "le reste en bref, jusqu'à la fin");
  eq(choicesOf(STEPS[r.s.index], r.s), [{ id: "finish", primary: true }], "Terminer, seul");
  r = play([{ kind: "choose", choice: "finish" }], r.s);
  eq(r.effects, [{ kind: "finish" }], "Terminer termine");
}
{
  // Detours of the choice of a game.
  const atSearch = play([{ kind: "choose", choice: "start" }, { kind: "next" }, web({ place: "store" })]).s;
  eq(STEPS[atSearch.index].id, "first-search", "sans nouvelles de la fenêtre, le lieu du magasin compte");
  let r = play([{ kind: "free", appId: "1", free: true }], play([storeApp("1")], atSearch).s);
  eq([r.step, r.s.notice], ["first-search", "free"], "un jeu gratuit : on reste, et on le dit");
  r = play([web({ place: "store" })], r.s);
  eq(r.s.notice, null, "revenu à la recherche : l'avertissement part");
  r = play([storeApp("2", true)], r.s);
  eq([r.step, r.s.notice, r.effects.length], ["first-search", "owned", 0], "un jeu déjà possédé : pas de recherche du prix");
  r = play([storeApp("3", false, true)], r.s);
  eq(r.s.notice, "known", "un jeu qu'LuaVault a déjà");
  eq(choicesOf(STEPS[r.s.index], r.s).map((c) => c.id), ["library", "other"], "ses choix");
  const lib = play([{ kind: "choose", choice: "library" }], r.s);
  eq([lib.step, lib.s.appId], ["first-open-library", "3"], "Voir dans la bibliothèque : ce jeu devient celui du guide");
  eq(lib.effects, [{ kind: "open", place: "library-app:3" }, { kind: "status", appId: "3" }], "Steam l'ouvre, et l'état du jeu est relu");
  // Field report 2026-11-15: a game LuaVault had, not installed by Steam, was told « Vert : tout est en place ».
  let k = play([win({ place: "library-app", appId: "3" })], lib.s);
  eq(k.step, "first-ready", "arrivé sur sa page");
  eq(infoKey(STEPS[k.s.index], k.s), "steamguide.step.first-ready.checking", "l'état pas encore lu : rien n'est affirmé");
  eq(play([{ kind: "next" }], k.s).step, "first-ready", "et pas de Continuer avant de savoir");
  k = play([status("3")], k.s);
  eq(k.step, "first-install-library", "pas installé : Installer, dans la bibliothèque");
  eq(viewsOf(STEPS[k.s.index], k.s), { where: "window", bubble: "window", target: "play-button" }, "le bouton de Steam lui-même");
  eq(play([{ kind: "hit", target: "play-button" }], k.s).step, "first-wait", "le clic : l'attente");
  eq(play([status("3", { installing: true })], k.s).step, "first-wait", "ou Steam qui télécharge");
  const done = play([status("3", { installed: true })], k.s);
  eq(done.step, "first-ready", "installé : le bloc vert");
  eq(infoKey(STEPS[done.s.index], done.s), "steamguide.step.first-ready.info", "et ce n'est dit qu'une fois vrai");
  // Known AND installed: straight to the block, no install step.
  const inst = play([status("3", { installed: true }), win({ place: "library-app", appId: "3" })], lib.s);
  eq(inst.step, "first-ready", "déjà installé : le bloc");
  // The normal flow, installed through the store: the library step skips the install.
  const flow = play([status("5", { installed: true })], { ...INITIAL, index: indexOf("first-wait"), appId: "5", pageId: "5", windowPlace: { place: "library-app", appId: "5" } });
  eq(flow.step, "first-ready", "après l'attente, sur sa page : le bloc, sans étape Installer");
  // « En choisir un autre » does something visible: the store's front page.
  eq(play([{ kind: "choose", choice: "other" }], r.s).effects, [{ kind: "open", place: "store" }], "En choisir un autre ouvre le magasin");
  r = play([web({ place: "store-bundle" })], r.s);
  eq(r.s.notice, "bundle", "une offre groupée");
  r = play([storeApp("4"), { kind: "free", appId: "5", free: false }], r.s);
  eq(r.step, "first-search", "une réponse pour un autre jeu ne compte pas");
  r = play([{ kind: "free", appId: "4", free: false }], r.s);
  eq(r.step, "first-button", "la sienne, si");
  r = play([storeApp("6")], r.s);
  eq([r.step, r.s.pageId], ["first-search", "6"], "une autre page de jeu depuis le bouton : rejugée");
  // Landing on the store directly on a game's page: judged at once.
  const direct = play([{ kind: "choose", choice: "start" }, { kind: "next" }, win({ place: "web-shown" }), storeApp("7")]);
  eq([direct.step, direct.effects], ["first-search", [{ kind: "lookup-free", appId: "7" }]], "déjà sur une page de jeu en arrivant au magasin");
}
{
  // A DLC's page fetches its base game: the guide follows the fetch, the page stays the page.
  const atButton = { ...INITIAL, index: indexOf("first-button"), pageId: "369150", webPlace: { place: "store-app", appId: "369150", owned: false, known: false } as const, windowPlace: { place: "web-shown" } as const };
  let r = play([{ kind: "fetch", appIds: ["255710"] }], atButton);
  eq([r.s.appId, r.s.pageId], ["255710", "369150"], "jeu de base suivi, page du DLC gardée");
  r = play([job("369150", "ok")], r.s);
  eq(r.step, "first-check", "la ligne d'un autre jeu ne compte pas");
  r = play([job("255710", "ok", true)], r.s);
  eq([r.step, needMet(STEPS[r.s.index], r.s)], ["first-panel", true], "le panneau, sur la page du DLC");
  // Failures, on a game's own page.
  const onPage = (id: string): State => ({ ...atButton, pageId: id, webPlace: { place: "store-app", appId: id, owned: false, known: false } });
  const fail = play([{ kind: "fetch", appIds: ["9"] }, job("9", "error")], onPage("9"));
  eq([fail.step, fail.s.notice], ["first-button", "failed"], "un échec : retour au bouton, Réessayer");
  eq(choicesOf(STEPS[fail.s.index], fail.s), [{ id: "retry", primary: true }], "Réessayer en surbrillance");
  eq(play([{ kind: "choose", choice: "retry" }], fail.s).s.notice, null, "Réessayer efface l'avertissement");
  // Download detection switched off: the panel says so, the guide too, and the status moves it on.
  const off = play([{ kind: "fetch", appIds: ["9"] }, job("9", "warn", false, false, true)], onPage("9"));
  eq([off.step, off.s.notice], ["first-check", "detect-off"], "détection coupée : ajoutez les fichiers à la main");
  eq(play([{ kind: "tick" }], off.s).effects, [{ kind: "status", appId: "9" }], "le guide relit l'état du jeu");
  const byHand = play([status("9", { luaInSteam: true })], off.s);
  eq([byHand.step, byHand.s.installable], ["first-panel", true], "le .lua posé à la main : la suite");
  // Already installed by Steam: no « Installer ».
  const ready = play([{ kind: "fetch", appIds: ["9"] }, job("9", "ok", false, true), { kind: "next" }], onPage("9"));
  eq(ready.step, "first-open-library", "Steam a déjà le jeu : droit à la bibliothèque");
}
{
  // Waiting: paused, skipped; and the place.
  const waiting: State = { ...INITIAL, index: indexOf("first-wait"), appId: "42", pageId: "42" };
  eq(choicesOf(STEPS[waiting.index], waiting), [{ id: "skip", primary: false }, { id: "pause", primary: true }], "attendre : Reprendre plus tard recommandé");
  let r = play([{ kind: "choose", choice: "pause" }], waiting);
  ok(r.s.paused, "en pause");
  eq(viewsOf(STEPS[r.s.index], r.s).target, null, "rien de pointé en pause");
  r = play([{ kind: "tick" }], r.s);
  eq(r.effects, [{ kind: "status", appId: "42" }], "la pause écoute toujours Steam");
  r = play([status("42", { installed: true })], r.s);
  eq([r.step, r.s.paused, r.s.notice], ["first-open-library", false, "paused"], "installé : on propose de reprendre");
  eq(choicesOf(STEPS[r.s.index], r.s), [{ id: "resume", primary: true }], "Reprendre");
  eq(play([{ kind: "choose", choice: "skip" }], waiting).step, "more-settings", "Continuer sans attendre : le reste");
  // Field report 2026-11-15: « Visite guidée » during a pause showed nothing.
  const paused = play([{ kind: "choose", choice: "pause" }], waiting).s;
  const again = play([{ kind: "again" }], paused);
  ok(!again.s.paused && again.step === "first-wait", "Visite guidée pendant la pause : la bulle d'attente revient");
  // Steam's own install window cancelled: the wait goes back to the step that asked to install.
  let w = play([{ kind: "hit", target: "play-button" }], { ...INITIAL, index: indexOf("first-install-library"), appId: "42", windowPlace: { place: "library-app", appId: "42" } });
  eq(w.step, "first-wait", "le clic sur Installer");
  for (let i = 0; i < 4; i++) w = play([status("42")], w.s);
  eq(w.step, "first-wait", "Steam a quelques secondes pour répondre");
  w = play([status("42")], w.s);
  eq(w.step, "first-install-library", "puis, rien ne venant : retour à Installer");
  let w2 = play([{ kind: "hit", target: "play-button" }], { ...INITIAL, index: indexOf("first-install-library"), appId: "42", windowPlace: { place: "library-app", appId: "42" } });
  for (let i = 0; i < 4; i++) w2 = play([status("42")], w2.s);
  w2 = play([status("42", { installing: true })], w2.s);
  for (let i = 0; i < 4; i++) w2 = play([status("42")], w2.s);
  eq(w2.step, "first-wait", "un téléchargement vu remet le compte à zéro");
  // After a « free » or « owned » page, the bubble points at the search again.
  const freeAt = play([storeApp("8"), { kind: "free", appId: "8", free: true }], { ...INITIAL, index: indexOf("first-search"), windowPlace: { place: "web-shown" }, webPlace: { place: "store" } }).s;
  eq(viewsOf(STEPS[freeAt.index], freeAt).target, "store-search", "un jeu gratuit : retour à la recherche, entourée");
  eq(actionKey(STEPS[freeAt.index], freeAt), "steamguide.step.first-search.action", "avec le geste à faire");
  // Away from the game's page.
  const away: State = { ...INITIAL, index: indexOf("first-ready"), appId: "42", windowPlace: { place: "library-app", appId: "7" } };
  ok(!needMet(STEPS[away.index], away), "pas sur la page du jeu");
  eq(infoKey(STEPS[away.index], away), "steamguide.away.info", "la bulle le dit");
  eq(choicesOf(STEPS[away.index], away), [{ id: "return", primary: true }], "Y retourner");
  eq(play([{ kind: "choose", choice: "return" }], away).effects, [{ kind: "open", place: "library-app:42" }], "qui ouvre la page du jeu");
  eq(play([{ kind: "next" }], away).step, "first-ready", "Continuer ne passe pas une étape absente");
  eq(viewsOf(STEPS[away.index], away).target, null, "ni anneau sur un autre jeu");
  // A switched-off block: skipped.
  const here: State = { ...away, windowPlace: { place: "library-app", appId: "42" } };
  eq(play([{ kind: "lost", target: "play-block" }], here).step, "more-dot", "bloc éteint : l'étape est sautée");
  eq(play([{ kind: "lost", target: "pill" }], here).step, "first-ready", "une autre cible perdue ne fait rien");
  // Choices not offered are ignored.
  eq(play([{ kind: "choose", choice: "finish" }], here).effects, [], "un choix que la bulle n'offre pas ne fait rien");
  eq(play([{ kind: "choose", choice: "skip" }], here).step, "first-ready", "même un vrai mot du guide");
  eq(play([{ kind: "back" }], { ...INITIAL, index: indexOf("first-button") }).step, "first-button", "pas de Retour sur un geste réel");
  eq(play([{ kind: "back" }], { ...INITIAL, index: indexOf("first-pill") }).step, "first-welcome", "Retour sur une étape d'explication");
  eq(play([{ kind: "quit" }], here).effects, [{ kind: "quit" }], "Quitter, à tout moment");
  eq(currentPlace({ ...INITIAL, windowPlace: { place: "web-shown" } }), { place: "web" }, "la vue web devant, sans nouvelles d'elle : le web");
}

// ── 2. the three lists of targets ────────────────────────────────────
{
  const rust = read("src-tauri/src/steam_guide.rs");
  const block = rust.slice(rust.indexOf("pub const TARGETS"), rust.indexOf("];", rust.indexOf("pub const TARGETS")));
  eq([...block.matchAll(/"([a-z-]+)"/g)].map((m) => m[1]), [...TARGETS], "Rust refuse exactement les cibles que le moteur ne connaît pas");
  const js = read("scripts/steam-guide.js");
  const jsBlock = js.slice(js.indexOf("var TARGETS = {"), js.indexOf("};", js.indexOf("var TARGETS = {")));
  eq([...jsBlock.matchAll(/^\s+"?([a-z-]+)"?:\s*function/gm)].map((m) => m[1]), [...TARGETS], "le script de la page sait trouver chacune");
  for (const st of STEPS) if (st.target) ok(TARGET_VIEW[st.target], `la vue de ${st.target}`);
  const choiceRule = /^[a-z][a-z0-9-]{0,23}$/;
  for (const c of CHOICES) ok(choiceRule.test(c), `l'identifiant ${c} passe la règle de Rust`);
}

// ── 3. the catalogues ────────────────────────────────────────────────
{
  const keys = new Set<string>();
  const notices: Notice[] = ["free", "owned", "known", "bundle", "detect-off", "failed", "paused"];
  for (const st of STEPS) {
    const s: State = { ...INITIAL, index: indexOf(st.id) };
    keys.add(infoKey(st, s));
    keys.add(infoKey(st, { ...s, gameInstalled: true }));
    keys.add(infoKey(st, { ...s, gameInstalled: true, patches: true }));
    const a = actionKey(st, s);
    if (a) keys.add(a);
    for (const n of notices) keys.add(infoKey(st, { ...s, notice: n }));
  }
  ok(ACTIONS.every((id) => STEPS.some((s) => s.id === id)), "chaque action nomme une étape");
  keys.add("steamguide.away.info");
  for (const c of CHOICES) keys.add(`steamguide.choice.${c}`);
  for (const k of ["steamguide.title.first", "steamguide.title.more", "steamguide.progress", "steamguide.next", "steamguide.back", "steamguide.quit"]) keys.add(k);
  for (const r of ["stable", "off", "hidden", "discreet", "steam", "error"]) keys.add(`steamguide.ready.${r}`);
  const FR = fr as Record<string, string>;
  const EN = en as Record<string, string>;
  for (const k of keys) {
    for (const [lang, cat] of [["fr", FR], ["en", EN]] as const) {
      const v = cat[k];
      ok(typeof v === "string" && v.length > 0, `${k} existe en ${lang}`);
      ok(!v.includes("—"), `${k} (${lang}) sans tiret cadratin`);
      ok(!/SteamTools|SteaMidra/.test(v), `${k} (${lang}) sans jargon`);
      if (k.endsWith(".action")) ok(v.length <= 50, `${k} (${lang}) : action de 50 caractères au plus (${v.length})`);
      else if (k.startsWith("steamguide.step.") || k.startsWith("steamguide.notice.")) ok(v.length <= 140, `${k} (${lang}) : 140 au plus (${v.length})`);
      if (k.startsWith("steamguide.choice.")) ok(v.length <= 30, `${k} (${lang}) : un choix tient sur un bouton`);
    }
  }
  const all = Object.keys(FR).filter((k) => k.startsWith("steamguide."));
  for (const k of all) ok(keys.has(k), `${k} sert à quelque chose`);
}

// ── 4. the page script, executed ─────────────────────────────────────
type Listener = (e: { target?: unknown; stopPropagation(): void; preventDefault(): void }) => void;
class El {
  attrs: Record<string, string> = {};
  children: El[] = [];
  parent: El | null = null;
  style: Record<string, string> = {};
  textContent = "";
  tagName: string;
  rect = { left: 0, top: 0, width: 0, height: 0 };
  listeners: Record<string, Listener[]> = {};
  [key: string]: unknown;
  constructor(tag = "DIV") {
    this.tagName = tag.toUpperCase();
  }
  setAttribute(k: string, v: string) {
    this.attrs[k] = String(v);
  }
  getAttribute(k: string) {
    return k in this.attrs ? this.attrs[k] : null;
  }
  appendChild(c: El) {
    if (c.parent) c.parent.removeChild(c);
    c.parent = this;
    this.children.push(c);
    return c;
  }
  insertBefore(c: El, ref: El | null) {
    c.parent = this;
    const at = ref ? this.children.indexOf(ref) : -1;
    if (at < 0) this.children.push(c);
    else this.children.splice(at, 0, c);
    return c;
  }
  removeChild(c: El) {
    this.children = this.children.filter((x) => x !== c);
    c.parent = null;
    return c;
  }
  get parentNode() {
    return this.parent;
  }
  get firstChild() {
    return this.children[0] ?? null;
  }
  addEventListener(kind: string, cb: Listener) {
    (this.listeners[kind] ??= []).push(cb);
  }
  getBoundingClientRect() {
    return this.rect;
  }
  contains(n: unknown): boolean {
    return n === this || this.children.some((c) => c.contains(n));
  }
  matches(sel: string): boolean {
    if (sel === "div") return this.tagName === "DIV";
    if (sel === "input[name=term]") return this.tagName === "INPUT" && this.attrs.name === "term";
    if (sel.startsWith(".")) return String(this.attrs.class || "").split(" ").includes(sel.slice(1));
    const m = /^\[([a-z-]+)\]$/.exec(sel);
    return m !== null && m[1] in this.attrs;
  }
  querySelectorAll(sel: string): El[] {
    const out: El[] = [];
    const walk = (n: El) => {
      for (const c of n.children) {
        if (c.matches(sel)) out.push(c);
        walk(c);
      }
    };
    walk(this);
    return out;
  }
  querySelector(sel: string) {
    return this.querySelectorAll(sel)[0] ?? null;
  }
  /** Click as a browser would: the document's capture listeners first, then the element's. */
  click(doc: FakeDoc) {
    const ev = { target: this, stopPropagation() {}, preventDefault() {} };
    for (const cb of doc.captures) cb(ev);
    for (const cb of this.listeners.click ?? []) cb(ev);
  }
  text(): string {
    return this.textContent + this.children.map((c) => c.text()).join("");
  }
}
interface FakeDoc {
  documentElement: El;
  body: El;
  captures: Listener[];
  visibilityState: string;
  querySelector(sel: string): El | null;
  querySelectorAll(sel: string): El[];
  createElement(tag: string): El;
  addEventListener(kind: string, cb: Listener, capture?: boolean): void;
  removeEventListener(kind: string, cb: Listener, capture?: boolean): void;
}
function makeDoc(): FakeDoc {
  const html = new El("HTML");
  const body = html.appendChild(new El("BODY"));
  const doc: FakeDoc = {
    documentElement: html,
    body,
    captures: [],
    visibilityState: "visible",
    querySelector: (s) => html.querySelector(s),
    querySelectorAll: (s) => html.querySelectorAll(s),
    createElement: (tag) => new El(tag),
    addEventListener: (kind, cb, capture) => {
      if (kind === "click" && capture) doc.captures.push(cb);
    },
    removeEventListener: (kind, cb, capture) => {
      if (kind === "click" && capture) doc.captures = doc.captures.filter((c) => c !== cb);
    },
  };
  return doc;
}
const SCRIPT = read("scripts/steam-guide.js");
function runPage(opts: { hostname: string; pathname?: string; initial?: unknown; known?: Record<string, string>; windowPlace?: () => string }) {
  const doc = makeDoc();
  const sent: string[] = [];
  const win: Record<string, unknown> = {};
  if (opts.windowPlace) win.lvWindowPlace = opts.windowPlace;
  let tick: (() => void) | null = null;
  let clock = 1_000;
  const FakeDate = { now: () => clock };
  const body = new Function(
    "document", "location", "window", "lvStore", "lvGuideView", "lvStoreKnown", "setInterval", "clearInterval", "innerWidth", "innerHeight", "matchMedia", "Date",
    SCRIPT,
  ) as (...a: unknown[]) => void;
  body(
    doc,
    { hostname: opts.hostname, pathname: opts.pathname ?? "/" },
    win,
    (word: string) => sent.push(word),
    opts.initial ?? null,
    opts.known,
    (cb: () => void) => {
      tick = cb;
      return 1;
    },
    () => {},
    1024,
    700,
    () => ({ matches: true }),
    FakeDate,
  );
  const draw = win.lvGuide as (v: unknown) => void;
  return {
    doc,
    sent,
    draw,
    tick: (n = 1) => {
      for (let i = 0; i < n; i++) tick!();
    },
    /** Time passes. */
    advance: (ms: number, closed = false) => {
      clock += ms;
      if (!closed) (doc.documentElement as unknown as { lvAliveAt: number }).lvAliveAt = clock;
    },
    win,
    guide: () => doc.querySelectorAll("[data-lv-guide]"),
  };
}
const view = (over: Record<string, unknown> = {}) => ({
  run: 1,
  where: "web",
  bubble: "web",
  target: "store-find",
  appId: null,
  title: "Votre premier jeu",
  progress: "Étape 5 sur 11",
  info: [{ t: "Ce bouton n'existe que chez vous." }],
  action: [{ t: "Cliquez sur " }, { t: "Trouver sur le site", ui: true }],
  choices: [],
  next: "",
  back: "",
  quit: "Quitter le guide",
  gesture: true,
  veil: true,
  corner: false,
  ...over,
});
{
  // A store page draws the store's step: ring on the button, bubble, veil; the click is reported.
  const p = runPage({ hostname: "store.steampowered.com", pathname: "/app/264710/Subnautica/" });
  const get = p.doc.body.appendChild(new El("A"));
  get.setAttribute("data-lv-find", "264710");
  get.rect = { left: 600, top: 300, width: 200, height: 40 };
  p.draw(view());
  eq(p.sent, ["guide:place:store-app:264710"], "la page dit où elle est dès l'étape reçue");
  const ring = p.doc.querySelector("[data-lv-guide-ring]");
  ok(ring !== null && ring.style.display === "block", "l'anneau entoure le bouton");
  eq([ring!.style.left, ring!.style.top, ring!.style.width], ["594px", "294px", "212px"], "à sa place, avec sa marge");
  eq(p.doc.querySelectorAll("[data-lv-guide-veil]").length, 4, "le voile en quatre pans");
  const bubble = p.doc.querySelector("[data-lv-guide-bubble]");
  ok(bubble !== null && bubble.text().includes("Ce bouton n'existe que chez vous."), "la bulle, en texte");
  ok(p.doc.querySelector("[data-lv-guide-ui]")?.textContent === "Trouver sur le site", "le nom d'interface à part");
  get.click(p.doc);
  eq(p.sent.slice(1), ["guide:hit:store-find"], "le clic sur la cible est rapporté");
  p.doc.querySelector("[data-lv-guide-quit]")!.click(p.doc);
  eq(p.sent.slice(2), ["guide:quit"], "Quitter");
  // The window's step is not drawn here; null takes everything away.
  p.draw(view({ where: "window", bubble: "window", target: "pill", run: 2 }));
  eq(p.guide().filter((n) => n.getAttribute("data-lv-guide-bubble") !== null || n.getAttribute("data-lv-guide-ring") !== null).length, 0, "une étape de la fenêtre ne se dessine pas dans le magasin");
  p.draw(null);
  eq(p.guide().length, 0, "null : tout part");
  // A forged step is refused as a whole.
  p.draw(view({ target: "body", run: 3 }));
  eq(p.guide().length, 0, "une cible inconnue : rien");
  p.draw(view({ choices: [{ id: "Bad Id", label: "x" }], run: 4 }));
  eq(p.guide().length, 0, "un choix forgé : rien");
}
{
  // Choices, the highlighted one, and what they send.
  const p = runPage({ hostname: "store.steampowered.com", pathname: "/" });
  p.draw(view({ target: null, gesture: false, choices: [{ id: "start", label: "C'est parti", primary: true }, { id: "later", label: "Plus tard", primary: false }] }));
  const choices = p.doc.querySelectorAll("[data-lv-guide-choice]");
  eq(choices.map((c) => c.textContent), ["C'est parti", "Plus tard"], "les choix");
  eq(choices.map((c) => c.getAttribute("data-lv-guide-primary")), ["1", null], "un seul en surbrillance");
  eq(p.doc.querySelectorAll("[data-lv-guide-veil]").length, 4, "sans cible : voile entier");
  choices[0].click(p.doc);
  choices[1].click(p.doc);
  eq(p.sent.filter((w) => w.startsWith("guide:choose")), ["guide:choose:start"], "un choix, une fois (le second clic attend l'étape suivante)");
  eq(p.sent.filter((w) => w.startsWith("guide:place")), ["guide:place:store"], "la page d'accueil du magasin");
}
{
  // The window: its place from the pill script, its targets, lost after three seconds.
  let place = "window";
  const p = runPage({ hostname: "steamloopback.host", windowPlace: () => place });
  p.draw(view({ where: "window", bubble: "window", target: "play-block", appId: "264710", gesture: false }));
  eq(p.sent, ["guide:place:window"], "la fenêtre dit son lieu");
  ok(p.doc.querySelector("[data-lv-guide-ring]")!.style.display === "none", "pas de bloc : pas d'anneau");
  eq(p.doc.querySelectorAll("[data-lv-guide-veil]").map((v) => v.style.width), ["0px", "0px", "0px", "0px"], "et pas de voile qui bloquerait tout");
  p.tick();
  p.advance(3_500);
  p.tick();
  eq(p.sent.filter((w) => w.startsWith("guide:lost")), ["guide:lost:play-block"], "trois secondes sans cible : perdue");
  p.advance(5_000);
  p.tick();
  eq(p.sent.filter((w) => w.startsWith("guide:lost")).length, 1, "dite une fois");
  place = "library-app:264710";
  p.tick(4);
  ok(p.sent.includes("guide:place:library-app:264710"), "le lieu suit la fenêtre");
  const block = p.doc.body.appendChild(new El("DIV"));
  block.setAttribute("data-lv-lib", "264710");
  block.rect = { left: 300, top: 500, width: 280, height: 48 };
  p.tick();
  ok(p.doc.querySelector("[data-lv-guide-ring]")!.style.display === "block", "le bloc posé : l'anneau vient");
  // The person opens the « Gérer » menu: the guide steps aside, and comes back once it is closed.
  const layer = p.doc.querySelector("[data-lv-guide]")!;
  const menu = p.doc.body.appendChild(new El("DIV"));
  menu.setAttribute("data-lv-lib-menu", "1");
  menu.rect = { left: 300, top: 560, width: 260, height: 180 };
  p.tick();
  eq(layer.style.display, "none", "un menu ouvert passe devant le voile");
  p.doc.body.removeChild(menu);
  p.tick();
  eq(layer.style.display, "", "fermé : le guide revient");
  // The tab of the store is found by its component's name, read on the real client.
  const tab = p.doc.body.appendChild(new El("DIV"));
  tab.rect = { left: 67, top: 34, width: 103, height: 32 };
  tab.__reactFiber$x = { memoizedProps: { className: "x" }, return: { memoizedProps: { name: "#Menu_Store", rootRoute: "/browser/" } } };
  p.draw(view({ where: "window", bubble: "window", target: "tab-store", run: 9, gesture: false }));
  eq(p.doc.querySelector("[data-lv-guide-ring]")!.style.left, "61px", "l'onglet Magasin, par le nom de son composant");
}
{
  // The store's search: the bubble beside the field, never over the suggestions under it.
  const p = runPage({ hostname: "store.steampowered.com", pathname: "/" });
  const field = p.doc.body.appendChild(new El("INPUT"));
  field.setAttribute("name", "term");
  field.rect = { left: 200, top: 6, width: 367, height: 34 };
  p.draw(view({ target: "store-search", gesture: false, veil: false }));
  const b = p.doc.querySelector("[data-lv-guide-bubble]")!;
  eq([b.style.left, b.style.top], ["585px", "8px"], "à droite du champ, à sa hauteur");
  // In the store the field sits in a form with the magnifier on its right: beside the form.
  const form = p.doc.body.appendChild(new El("FORM"));
  form.rect = { left: 200, top: 6, width: 401, height: 34 };
  (field as unknown as { form: El }).form = form;
  p.tick();
  eq(b.style.left, "619px", "après la loupe, pas dessus");
  (field as unknown as { form: El | null }).form = null;
  field.rect = { left: 600, top: 6, width: 367, height: 34 };
  p.tick();
  eq(b.style.left, "242px", "à gauche quand la droite manque");
}
{
  // Two play bars (the artwork's and the sticky header's): the one on screen is pointed at.
  const p2 = runPage({ hostname: "steamloopback.host", windowPlace: () => "library-app:264710" });
  const hidden = p2.doc.body.appendChild(new El("DIV"));
  hidden.setAttribute("data-lv-lib", "264710");
  hidden.rect = { left: 478, top: 83, width: 278, height: 62 };
  const shownBlock = p2.doc.body.appendChild(new El("DIV"));
  shownBlock.setAttribute("data-lv-lib", "264710");
  shownBlock.rect = { left: 484, top: 415, width: 278, height: 50 };
  const cover = p2.doc.body.appendChild(new El("DIV"));
  // What the screen shows at the hidden block's centre is the artwork, not the block.
  (p2.doc as unknown as { elementsFromPoint: (x: number, y: number) => El[] }).elementsFromPoint = (_x, y) => (y < 200 ? [cover, hidden] : [shownBlock]);
  p2.draw(view({ where: "window", bubble: "window", target: "play-block", appId: "264710", gesture: false }));
  eq(p2.doc.querySelector("[data-lv-guide-ring]")!.style.top, "409px", "l'anneau sur le bloc visible, pas sur l'en-tête caché");
}
{
  // A store page known to LuaVault, owned in Steam; a page that is not shown says nothing.
  const p = runPage({ hostname: "store.steampowered.com", pathname: "/app/620/Portal_2/", known: { "620": "installed" } });
  const banner = p.doc.body.appendChild(new El("DIV"));
  banner.setAttribute("class", "game_area_already_owned");
  p.draw(view({ target: null, gesture: false }));
  eq(p.sent, ["guide:place:store-app:620:ok"], "possédé et déjà à LuaVault");
  const hidden = runPage({ hostname: "store.steampowered.com", pathname: "/app/620/" });
  hidden.doc.visibilityState = "hidden";
  hidden.draw(view({ target: null }));
  eq(hidden.sent, [], "une vue cachée ne parle pas");
  const elsewhere = runPage({ hostname: "evil.example" });
  eq(typeof elsewhere.draw, "undefined", "hors des sites de Valve et de la fenêtre : rien");
  const bundle = runPage({ hostname: "store.steampowered.com", pathname: "/bundle/232/" });
  bundle.draw(view({ target: null }));
  eq(bundle.sent, ["guide:place:store-bundle"], "une offre groupée");
}
{
  // The initial step travels with the script (a page loaded later gets it).
  const p = runPage({ hostname: "store.steampowered.com", pathname: "/", initial: view({ target: null, gesture: false }) });
  ok(p.doc.querySelector("[data-lv-guide-bubble]") !== null, "l'étape en cours, dès le chargement");
}
{
  // The shipped wiring: the script reaches both views after their own, the pill starts it.
  const client = read("src-tauri/src/steam_client.rs");
  ok(client.includes('store_script_with(&known, &jobs), crate::steam_guide::script())'), "le magasin reçoit le guide après ses scripts");
  ok(client.includes('Kind::Window => format!("{WINDOW_SCRIPT}\\n{NOTICE_SCRIPT}\\n;{}", crate::steam_guide::script())'), "la fenêtre aussi");
  ok(client.includes("crate::steam_guide::fetch_seen(&app, std::slice::from_ref(&app_id));"), "le clic « Trouver sur le site » est relayé au guide");
  const pill = read("scripts/steam-client-pill.js");
  ok(pill.includes('lvStore("guide:start")'), "la pastille lance la visite");
  ok(pill.indexOf('setAttribute("data-lv-guide-start"') < pill.indexOf('setAttribute("data-lv-settings-body"'), "en tête du panneau, avant les réglages");
  ok(pill.includes("window.lvWindowPlace = function") && pill.includes("window.lvPlayButtons = function"), "la pastille dit le lieu de la fenêtre et ses barres Jouer");
  // Two applications may each put a pill in Steam's title bar: one at the head of the row must not
  // dislodge the other every second (seen 2026-11-15: both blinked and swapped places).
  ok(/function atHead\(r, mine\)/.test(pill) && !/r\.firstChild === (mine|have)\)/.test(pill), "une pastille accepte d'autres pastilles devant elle");
  ok(/window\.lvOpenSettings = function \(\) \{[\s\S]{0,120}if \(card\) return closeCard\(\);/.test(pill), "un second clic sur la pastille referme le panneau ouvert dans le magasin");
  const store = read("scripts/steam-store-button.js");
  ok(store.includes('install.setAttribute("data-lv-job-install", appId)'), "le bouton Installer du panneau porte son ancre");
}

{
  // Not the guide: the published executables. Rust writes the builder's profile folder into every
  // binary (dependency and toolchain sources); each script that builds one to publish remaps it
  // and refuses an executable that still names it (`release-paths.ps1`, 2026-11-15).
  const helper = read("scripts/release-paths.ps1");
  ok(helper.includes("--remap-path-prefix=$profileDir=~") && helper.includes("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS"), "le dossier de profil est remplacé à la construction");
  ok(/function Assert-NoBuilderPath/.test(helper) && helper.includes("throw"), "un exécutable qui le contient encore est refusé");
  for (const f of ["scripts/build-app.ps1", "scripts/build-exp.ps1"]) {
    const script = read(f);
    ok(script.includes("release-paths.ps1") && script.includes("Enable-PathRemap") && script.includes("Assert-NoBuilderPath"), `${f} retire les chemins et le vérifie`);
  }
  ok(read("package.json").includes('"build:app": "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-app.ps1"'), "npm run build:app passe par ce script");
}

console.log(`test-steam-guide : ${passed} vérifications passées`);

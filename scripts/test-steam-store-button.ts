/**
 * Runs the store script injected into the Steam client (`scripts/steam-store-button.js`) against a
 * minimal DOM shaped after the REAL store pages (read in the client on 2026-11-14, 25 pages):
 * which purchase row gets the « Trouver sur le site » button. Counting motifs in the script's text
 * cannot tell which row a pass ends up dressing; running it can.
 */
// @ts-expect-error — `node:fs` has no types here (same convention as test-library-view.ts).
import { readFileSync as readFileSyncRaw } from "node:fs";

const readFileSync = readFileSyncRaw as (path: string, encoding: "utf8") => string;

const assert = {
  equal(actual: unknown, expected: unknown, msg: string): void {
    if (actual !== expected) throw new Error(`${msg}: attendu ${String(expected)}, obtenu ${String(actual)}`);
  },
};

const SCRIPT = readFileSync("scripts/steam-store-button.js", "utf8");

class El {
  attrs: Record<string, string> = {};
  children: El[] = [];
  parentNode: El | null = null;
  className = "";
  textContent = "";
  value = "";
  style: Record<string, string> & { cssText?: string } = {};
  lang = "fr";
  getAttribute(k: string): string | null {
    return k in this.attrs ? this.attrs[k] : null;
  }
  setAttribute(k: string, v: string): void {
    this.attrs[k] = String(v);
  }
  appendChild(c: El): El {
    c.parentNode = this;
    this.children.push(c);
    return c;
  }
  insertBefore(c: El, ref: El | null): El {
    c.parentNode = this;
    const at = ref ? this.children.indexOf(ref) : -1;
    if (at < 0) this.children.push(c);
    else this.children.splice(at, 0, c);
    return c;
  }
  removeChild(c: El): El {
    this.children = this.children.filter((k) => k !== c);
    c.parentNode = null;
    return c;
  }
  addEventListener(): void {}
  select(): void {}
  get firstChild(): El | null {
    return this.children[0] ?? null;
  }
  matches(sel: string): boolean {
    if (sel.startsWith("[")) return this.getAttribute(sel.slice(1, -1)) !== null;
    if (sel.startsWith(".")) return this.className.split(/\s+/).includes(sel.slice(1));
    if (sel.startsWith("#")) return this.attrs.id === sel.slice(1);
    return false;
  }
  querySelectorAll(sel: string, out: El[] = []): El[] {
    for (const c of this.children) {
      if (c.matches(sel)) out.push(c);
      c.querySelectorAll(sel, out);
    }
    return out;
  }
  querySelector(sel: string): El | null {
    return this.querySelectorAll(sel)[0] ?? null;
  }
}

/** One offer block as the page has it: a block holding a `.game_purchase_action` row. */
function block(className: string, extra?: (b: El) => void) {
  const b = new El();
  b.className = className;
  extra?.(b);
  const row = new El();
  row.className = "game_purchase_action";
  b.appendChild(row);
  return { b, row };
}

function page(appId: string, build: (area: El, root: El) => void) {
  const root = new El();
  const queue = new El();
  queue.attrs.id = "queueActionsCtn";
  root.appendChild(queue);
  const area = new El();
  area.attrs.id = "game_area_purchase";
  root.appendChild(area);
  build(area, root);
  const document = {
    documentElement: root,
    body: root,
    createElement: () => new El(),
    getElementById: (id: string) => root.querySelector("#" + id),
    querySelector: (s: string) => root.querySelector(s),
    querySelectorAll: (s: string) => root.querySelectorAll(s),
    execCommand: () => true,
  };
  class Observer {
    observe(): void {}
    disconnect(): void {}
  }
  const location = { hostname: "store.steampowered.com", pathname: `/app/${appId}/` };
  const body = new Function(
    "document", "location", "MutationObserver", "navigator", "window", "lvStore", "lvStoreKnown", "lvStoreJobs", SCRIPT,
  ) as (...args: unknown[]) => void;
  body(document, location, Observer, { language: "fr" }, {}, () => {}, {}, []);
  return { queue };
}

const found = (n: El) => n.querySelectorAll("[data-lv-page]").length;
let cases = 0;

{
  // Mortal Shell II: an open beta first, then the offers under « Éditions de … ».
  const demo = block("game_area_purchase_game demo_above_purchase");
  const game = block("game_area_purchase_game_wrapper");
  const deluxe = block("game_area_purchase_game_wrapper");
  const { queue } = page("2584270", (area) => {
    area.appendChild(demo.b);
    const editions = new El();
    editions.className = "editions_purchase_options_container";
    editions.appendChild(game.b);
    editions.appendChild(deluxe.b);
    area.appendChild(editions);
  });
  assert.equal(found(game.row), 1, "éditions : dans la ligne de la première édition");
  assert.equal(found(demo.row), 0, "jamais à côté de la démo");
  assert.equal(found(deluxe.row) + found(queue), 0, "une seule fois");
  cases++;
}
{
  // Bodycam: a playtest block comes first.
  const playtest = block("game_area_purchase_game", (b) => {
    const note = new El();
    note.className = "game_pt_note";
    b.appendChild(note);
  });
  const game = block("game_area_purchase_game_wrapper");
  page("2406770", (area) => {
    area.appendChild(playtest.b);
    area.appendChild(game.b);
  });
  assert.equal(found(playtest.row), 0, "pas dans le bloc du playtest");
  assert.equal(found(game.row), 1, "dans la ligne du jeu");
  cases++;
}
{
  // Counter-Strike 2: a free game's own block is bare, before a paid upgrade.
  const play = block("game_area_purchase_game");
  const upgrade = block("game_area_purchase_game_wrapper");
  page("730", (area) => {
    area.appendChild(play.b);
    area.appendChild(upgrade.b);
  });
  assert.equal(found(play.row), 1, "gratuit : dans le bloc « Jouer à »");
  assert.equal(found(upgrade.row), 0, "pas à côté de la mise à niveau payante");
  cases++;
}
{
  // Crusader Kings III: a subscription is never the game's offer.
  const sub = block("game_area_purchase_game_wrapper game_purchase_sub_dropdown");
  const game = block("game_area_purchase_game_wrapper");
  page("1158310", (area) => {
    area.appendChild(sub.b);
    area.appendChild(game.b);
  });
  assert.equal(found(sub.row), 0, "pas dans l'abonnement");
  assert.equal(found(game.row), 1, "dans l'achat du jeu");
  cases++;
}
{
  // Cafe Simulator: not out yet, a demo only. The wishlist row holds the button.
  const demo = block("game_area_purchase_game demo_above_purchase");
  const { queue } = page("2384980", (area) => area.appendChild(demo.b));
  assert.equal(found(demo.row), 0, "pas à côté de la démo");
  assert.equal(found(queue), 1, "dans la rangée liste de souhaits");
  cases++;
}
{
  // The plain page: the game first, then a bundle.
  const game = block("game_area_purchase_game_wrapper");
  const bundle = block("game_area_purchase_game_wrapper dynamic_bundle_description");
  page("264710", (area) => {
    area.appendChild(game.b);
    area.appendChild(bundle.b);
  });
  assert.equal(found(game.row), 1, "le jeu");
  assert.equal(found(bundle.row), 0, "pas le bundle");
  cases++;
}

export const __storeButtonRan = true;
export const storeButtonCases = cases;
console.log(`  bouton de la boutique : ${cases} pages relevées, ligne d'achat juste`);

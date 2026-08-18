//! Virtual scrolling (LOT-18) above its threshold, which nothing had ever
//! crossed at runtime: every other suite seeds three games, and the grid only
//! virtualises past a hundred.
//!
//! Two properties matter, and they pull in opposite directions — which is why
//! both must be pinned together:
//!
//!   * the DOM must NOT hold all 150 cards, or virtualisation buys nothing;
//!   * the *library* is still 150 games, so counting, filtering and sorting must
//!     keep working on the whole set and not on the rendered window. That was
//!     LOT-09's reframing — the reason backend pagination was refused — and a
//!     virtual window is the same trap wearing different clothes.

import { defineSuite, assert } from '../suite.ts';
import { MANY, MANY_LUA } from '../fixtures.ts';

const FILTER = 'input[placeholder="Filtrer par nom ou AppID…"]';
const CARD = 'article.lift-card';

async function cardCount(app: import('../harness.ts').App): Promise<number> {
  return app.session.execute<number>(`return document.querySelectorAll('${CARD}').length`);
}

async function firstCardName(app: import('../harness.ts').App): Promise<string | null> {
  return app.session.execute<string | null>(
    `const h = document.querySelector('${CARD} h3');
     return h ? h.textContent.trim() : null`,
  );
}

export default defineSuite({
  name: 'virtual',
  seed: { index: MANY, lua: MANY_LUA },
  cases: {
    'au-delà du seuil, la grille ne rend pas les 150 cartes': async (app) => {
      const s = app.session;
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]')).click();
      await s.waitFor(FILTER);
      await s.waitUntil('des cartes apparaissent', async () => (await cardCount(app)) > 0);

      const rendered = await cardCount(app);
      assert.ok(
        rendered < MANY.length,
        `les ${MANY.length} cartes sont toutes dans le DOM (${rendered}) — la virtualisation ne fait rien`,
      );
      // And it must still render something worth seeing, not one lonely row:
      // a window collapsed to nothing would also satisfy the assertion above.
      assert.ok(rendered >= 3, `seulement ${rendered} carte(s) rendue(s) — la fenêtre est vide`);
    },

    'la bibliothèque compte toujours 150 jeux, pas la fenêtre rendue': async (app) => {
      // The sidebar badge reads the library, not the grid. If virtualisation
      // ever started driving it, the user would be told they own 30 games.
      const badge = await app.session.execute<string | null>(
        `const b = document.querySelector('nav button[data-tip="Bibliothèque"] span.rounded-full:last-child');
         return b ? b.textContent.trim() : null`,
      );
      assert.equal(badge, String(MANY.length), 'la puce doit annoncer toute la bibliothèque');
    },

    'faire défiler change les cartes rendues': async (app) => {
      const s = app.session;
      const before = await firstCardName(app);
      assert.ok(before !== null, 'une première carte doit être rendue');

      // Scroll the container that actually scrolls, then let the handler run.
      const scrolled = await s.execute<boolean>(`
        const els = [...document.querySelectorAll('*')].filter(
          (e) => e.scrollHeight > e.clientHeight + 200 && getComputedStyle(e).overflowY !== 'visible',
        );
        const el = els[els.length - 1];
        if (!el) return false;
        el.scrollTop = Math.floor(el.scrollHeight / 2);
        el.dispatchEvent(new Event('scroll', { bubbles: true }));
        return true;
      `);
      assert.ok(scrolled, 'un conteneur défilable doit exister');

      await s.waitUntil('la première carte rendue change', async () => (await firstCardName(app)) !== before);

      const after = await firstCardName(app);
      assert.notEqual(after, before, 'le défilement doit renouveler la fenêtre rendue');
      // Still a window, not the whole list unfolded by the scroll.
      assert.ok((await cardCount(app)) < MANY.length, 'la fenêtre reste une fenêtre après défilement');
    },

    'le filtre porte sur toute la bibliothèque, pas sur la fenêtre rendue': async (app) => {
      const s = app.session;
      // « Jeu 149 » is far past the first window — under a filter that only saw
      // rendered cards, it would simply not exist. This is the case that would
      // catch a filter wired to `visibleSlice` instead of the full library.
      const input = await s.waitFor(FILTER);
      await input.click();
      await input.sendKeys('Jeu 149');

      await s.waitUntil('une seule carte reste', async () => (await cardCount(app)) === 1);
      assert.equal(await firstCardName(app), 'Jeu 149', 'le jeu cherché doit être trouvé où qu’il soit');

      // Below the threshold now: the grid must come back to plain rendering
      // rather than stay stuck in a window of one.
      await s.execute(`
        const i = document.querySelector(${JSON.stringify(FILTER)});
        i.value = '';
        i.dispatchEvent(new Event('input', { bubbles: true }));
      `);
      await s.waitUntil('la bibliothèque revient', async () => (await cardCount(app)) > 1);
    },
  },
});

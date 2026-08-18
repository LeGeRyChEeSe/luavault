//! The library view against a seeded index: does the list actually render, and
//! does filtering actually filter?
//!
//! `scripts/test-library-view.ts` pins the *shape* of this component's source.
//! These cases exercise the rendered result — the thing the shape guards were
//! standing in for.

import { defineSuite, assert } from '../suite.ts';
import { Key } from '../webdriver.ts';
import { SAMPLE, SAMPLE_LUA } from '../fixtures.ts';

const FILTER = 'input[placeholder="Filtrer par nom ou AppID…"]';
const CARD = 'article.lift-card';

async function openLibrary(app: import('../harness.ts').App) {
  const s = app.session;
  await (await s.waitFor('nav button[data-tip="Bibliothèque"]')).click();
  await s.waitFor(FILTER);
}

async function shownNames(app: import('../harness.ts').App): Promise<string[]> {
  return app.session.execute<string[]>(
    `return [...document.querySelectorAll('${CARD} h3')].map(h => h.textContent.trim())`,
  );
}

export default defineSuite({
  name: 'library',
  seed: { index: SAMPLE, lua: SAMPLE_LUA },
  cases: {
    'les trois jeux semés sont rendus comme cartes': async (app) => {
      await openLibrary(app);
      await app.session.waitUntil('trois cartes apparaissent', async () => (await shownNames(app)).length === 3);
      const names = await shownNames(app);
      for (const wanted of ['Subnautica', 'Cyberpunk 2077', 'The Witcher 3']) {
        assert.ok(names.includes(wanted), `« ${wanted} » absent de : ${names.join(', ')}`);
      }
    },

    'le compteur de la navigation annonce la taille réelle de la bibliothèque': async (app) => {
      const badge = await app.session.execute<string | null>(
        `const b = document.querySelector('nav button[data-tip="Bibliothèque"] span.rounded-full:last-child');
         return b ? b.textContent.trim() : null`,
      );
      assert.equal(badge, '3', 'la puce de la barre latérale doit afficher 3');
    },

    'filtrer par nom ne garde que le jeu correspondant': async (app) => {
      const s = app.session;
      const input = await s.waitFor(FILTER);
      await input.clear();
      await input.sendKeys('Witcher');
      await s.waitUntil('une seule carte reste', async () => (await shownNames(app)).length === 1);
      assert.equal((await shownNames(app))[0], 'The Witcher 3');
    },

    "filtrer par AppID trouve le jeu dont le nom ne contient pas la saisie": async (app) => {
      const s = app.session;
      const input = await s.waitFor(FILTER);
      await input.clear();
      await input.sendKeys('264710');
      await s.waitUntil('une seule carte reste', async () => (await shownNames(app)).length === 1);
      // Discriminating: "264710" appears nowhere in the string "Subnautica",
      // so a name-only filter cannot pass this case.
      assert.equal((await shownNames(app))[0], 'Subnautica');
    },

    'un filtre sans résultat vide la grille sans casser la vue': async (app) => {
      const s = app.session;
      const input = await s.waitFor(FILTER);
      await input.clear();
      await input.sendKeys('zzzzz-aucun-jeu');
      await s.waitUntil('plus aucune carte', async () => (await shownNames(app)).length === 0);
      // The view must still be alive: the filter field is still there.
      assert.ok(await (await s.find(FILTER)).displayed(), 'le champ de filtre a disparu');
    },

    'effacer le filtre au clavier ramène toute la bibliothèque': async (app) => {
      const s = app.session;
      const input = await s.waitFor(FILTER);
      await input.click();
      // Deliberately NOT WebDriver's `clear()`: it empties the value without
      // firing `input`, so Svelte's `bind:value` never sees it and the grid
      // stays filtered. Select-all then Backspace is the real gesture, and the
      // only one that proves the binding reacts.
      await s.chord([Key.Control, 'a']);
      await s.keys([Key.Backspace]);
      await s.waitUntil('les trois cartes reviennent', async () => (await shownNames(app)).length === 3);
    },

    'le bouton « Effacer la recherche » vide le filtre': async (app) => {
      const s = app.session;
      const input = await s.waitFor(FILTER);
      await input.click();
      await input.sendKeys('Witcher');
      await s.waitUntil('le filtre est actif', async () => (await shownNames(app)).length === 1);

      // The clear button only exists while the field is non-empty.
      await (await s.waitFor('button[aria-label="Effacer la recherche"]')).click();
      await s.waitUntil('les trois cartes reviennent', async () => (await shownNames(app)).length === 3);
      assert.equal(await (await s.find(FILTER)).property('value'), '', 'le champ doit être vide');
    },
  },
});

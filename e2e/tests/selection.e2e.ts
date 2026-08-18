//! Multi-selection (LOT-16) at the screen, where its one real hazard lives.
//!
//! The danger of a selection is not selecting — it is acting on something the
//! user cannot see. LOT-16's review found exactly that: `selectedVisible` was
//! not pinned, so a pass could treat games a filter had hidden. The code now
//! separates "selected" from "selected AND on screen", and every action button
//! counts the second. Nothing exercised that separation at runtime until here.
//!
//! The cases below therefore care far more about the counts than about the
//! clicking: a count is the promise the button makes before it runs.

import { defineSuite, assert } from '../suite.ts';
import { SAMPLE, SAMPLE_LUA } from '../fixtures.ts';

const FILTER = 'input[placeholder="Filtrer par nom ou AppID…"]';
const CARD = 'article.lift-card';

async function openLibrary(app: import('../harness.ts').App) {
  const s = app.session;
  await (await s.waitFor('nav button[data-tip="Bibliothèque"]')).click();
  await s.waitFor(FILTER);
}

/// Text of the "N sélectionné(s)" counter — the selection as a whole.
async function selectedCount(app: import('../harness.ts').App): Promise<number> {
  const txt = await app.session.execute<string | null>(
    `const el = [...document.querySelectorAll('span')].find(s => /sélectionné\\(s\\)/.test(s.textContent ?? ''));
     return el ? el.textContent.trim() : null`,
  );
  const m = txt?.match(/(\d+)/);
  return m ? Number(m[1]) : -1;
}

/// The number a given action button announces it would treat.
async function buttonCount(app: import('../harness.ts').App, label: string): Promise<number> {
  const txt = await app.session.execute<string | null>(
    `const b = [...document.querySelectorAll('button')].find(x => (x.textContent ?? '').includes(${JSON.stringify(label)}));
     return b ? b.textContent.trim() : null`,
  );
  const m = txt?.match(/\((\d+)\)/);
  return m ? Number(m[1]) : -1;
}

async function enterSelectionMode(app: import('../harness.ts').App) {
  const s = app.session;
  const clicked = await s.execute<boolean>(`
    const b = [...document.querySelectorAll('button')].find(x => (x.textContent ?? '').includes('Sélectionner des jeux'));
    if (!b) return false;
    b.click();
    return true;
  `);
  assert.ok(clicked, 'le bouton « Sélectionner des jeux » doit exister');
  await s.waitUntil('la barre de sélection apparaît', async () => (await selectedCount(app)) >= 0);
}

export default defineSuite({
  name: 'selection',
  seed: { index: SAMPLE, lua: SAMPLE_LUA },
  cases: {
    'tout sélectionner ne prend que ce qui est à l’écran': async (app) => {
      const s = app.session;
      await openLibrary(app);
      await s.waitUntil('les trois cartes sont là', async () =>
        (await s.execute<number>(`return document.querySelectorAll('${CARD}').length`)) === 3,
      );
      await enterSelectionMode(app);
      assert.equal(await selectedCount(app), 0, 'on entre dans le mode sans rien de sélectionné');

      const clicked = await s.execute<boolean>(`
        const b = [...document.querySelectorAll('button')].find(x => (x.textContent ?? '').includes('Tout sélectionner'));
        if (!b) return false;
        b.click();
        return true;
      `);
      assert.ok(clicked, 'le bouton « Tout sélectionner » doit exister');
      await s.waitUntil('les trois jeux sont sélectionnés', async () => (await selectedCount(app)) === 3);
    },

    "un filtre qui cache des jeux sélectionnés le dit, et les boutons cessent de les compter": async (app) => {
      const s = app.session;
      // Starting state: three games selected, all visible (previous case).
      assert.equal(await selectedCount(app), 3, 'les trois jeux sont encore sélectionnés');
      const copyBefore = await buttonCount(app, 'Copier vers Steam');
      assert.ok(copyBefore >= 0, 'le bouton « Copier vers Steam » doit annoncer un nombre');

      const input = await s.waitFor(FILTER);
      await input.click();
      await input.sendKeys('Witcher');
      await s.waitUntil('une seule carte reste', async () =>
        (await s.execute<number>(`return document.querySelectorAll('${CARD}').length`)) === 1,
      );

      // The whole selection is untouched — hiding is not deselecting.
      assert.equal(await selectedCount(app), 3, 'filtrer ne désélectionne pas');

      // But the buttons must now count only what is on screen. This is the
      // assertion that would have caught LOT-16's defect: acting on games the
      // user cannot see.
      const copyAfter = await buttonCount(app, 'Copier vers Steam');
      assert.ok(
        copyAfter <= 1,
        `« Copier vers Steam » annonce ${copyAfter} alors qu'un seul jeu est visible`,
      );
      assert.notEqual(
        copyAfter,
        3,
        'le bouton compterait des jeux que le filtre cache — le défaut du LOT-16',
      );

      // And the gap is named rather than left as a mystery between "3
      // sélectionné(s)" and a button that says 1.
      await s.waitUntil('l’écart entre la sélection et ce qui sera traité doit être annoncé', async () =>
        (await (await s.find('body')).text()).includes('ne sont pas visibles avec le filtre actuel'),
      );
      const body = await (await s.find('body')).text();
      assert.includes(body, '2 jeu(x)', 'les deux jeux cachés doivent être comptés');
    },

    'quitter le mode vide la sélection': async (app) => {
      const s = app.session;
      // Clear the filter the way a user does (pitfall 42) so the next reading
      // is not taken through a filtered view.
      const input = await s.waitFor(FILTER);
      await input.click();
      await s.execute(`
        const i = document.querySelector(${JSON.stringify(FILTER)});
        i.value = '';
        i.dispatchEvent(new Event('input', { bubbles: true }));
      `);
      await s.waitUntil('les trois cartes reviennent', async () =>
        (await s.execute<number>(`return document.querySelectorAll('${CARD}').length`)) === 3,
      );

      const clicked = await s.execute<boolean>(`
        const b = [...document.querySelectorAll('button')].find(x => (x.textContent ?? '').includes('Quitter le mode'));
        if (!b) return false;
        b.click();
        return true;
      `);
      assert.ok(clicked, 'le bouton « Quitter le mode » doit exister');

      await s.waitUntil('la barre de sélection disparaît', async () => (await selectedCount(app)) === -1);

      // Re-entering must start empty: a selection surviving the mode is exactly
      // the invisible selection the design forbids.
      await enterSelectionMode(app);
      assert.equal(await selectedCount(app), 0, 'la sélection ne survit pas au mode');
    },
  },
});

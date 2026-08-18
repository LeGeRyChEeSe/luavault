//! LOT-20's keyboard shortcuts, exercised through the real window.
//!
//! `resolveShortcut` is a pure function and is already unit-tested. What was
//! never verified is the wiring around it: that the window listener runs, that
//! the view actually changes, that focus lands in the search field after the
//! `tick()`, and that a shortcut typed inside a text field is ignored.

import { defineSuite, assert } from '../suite.ts';
import { Key } from '../webdriver.ts';
import { SAMPLE, SAMPLE_LUA } from '../fixtures.ts';

const activeTab = `[...document.querySelectorAll('nav button')].find(b => b.className.includes('bg-surface/80'))?.getAttribute('data-tip') ?? null`;

export default defineSuite({
  name: 'shortcuts',
  seed: { index: SAMPLE, lua: SAMPLE_LUA },
  cases: {
    'Ctrl+L ouvre les journaux': async (app) => {
      const s = app.session;
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]')).click();
      await s.waitUntil('Bibliothèque est active', async () => (await s.execute<string>(`return ${activeTab}`)) === 'Bibliothèque');

      await s.chord([Key.Control, 'l']);
      await s.waitUntil('Journaux devient actif', async () => (await s.execute<string>(`return ${activeTab}`)) === 'Journaux');
    },

    '« ? » ouvre l’aide, et Échap la referme': async (app) => {
      const s = app.session;
      await s.keys(['?']);
      const dialog = await s.waitFor('[role="dialog"][aria-modal="true"]');
      assert.includes(await dialog.text(), 'Raccourcis');
      await s.keys([Key.Escape]);
      await s.waitUntil('l’aide se referme', async () => (await s.maybeFind('[role="dialog"][aria-modal="true"]')) === null);
    },

    'un raccourci tapé dans un champ de saisie est ignoré': async (app) => {
      const s = app.session;
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]')).click();
      const filter = await s.waitFor('input[placeholder="Filtrer par nom ou AppID…"]');
      await filter.click();
      await filter.clear();

      // Ctrl+L inside a text field must NOT navigate away: `isEditableTarget`
      // is what stands between the user and losing what they were typing.
      await s.chord([Key.Control, 'l']);
      await new Promise((r) => setTimeout(r, 400));
      const active = await s.execute<string>(`return ${activeTab}`);
      assert.equal(active, 'Bibliothèque', 'Ctrl+L dans un champ a quand même navigué');

      // And the field is still the one holding focus.
      const stillFocused = await s.execute<boolean>(
        `return document.activeElement?.getAttribute('placeholder') === 'Filtrer par nom ou AppID…'`,
      );
      assert.ok(stillFocused, 'le champ de filtre a perdu le focus');
    },

    "un raccourci est ignoré tant qu'une modale est ouverte": async (app) => {
      const s = app.session;
      await (await s.waitFor('nav button[data-tip="Réglages"]')).click();
      await s.waitUntil('Réglages est actif', async () => (await s.execute<string>(`return ${activeTab}`)) === 'Réglages');
      await (await s.waitFor('aside button[data-tip="Raccourcis clavier"]')).click();
      await s.waitFor('[role="dialog"][aria-modal="true"]');

      await s.chord([Key.Control, 'l']);
      await new Promise((r) => setTimeout(r, 400));
      const active = await s.execute<string>(`return ${activeTab}`);
      assert.equal(active, 'Réglages', 'Ctrl+L a navigué alors que la modale était ouverte');

      await s.keys([Key.Escape]);
      await s.waitUntil('la modale se referme', async () => (await s.maybeFind('[role="dialog"][aria-modal="true"]')) === null);
    },
  },
});

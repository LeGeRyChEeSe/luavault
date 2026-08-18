//! LOT-21's fail-closed index, seen from the screen.
//!
//! The sandbox starts with an HMAC key and a **corrupt sidecar** next to a
//! perfectly valid `index.json` — exactly the state of a library folder copied
//! from another machine, which is the case the whole mechanism exists for.
//!
//! What this proves, and what no unit test could: the banner is reachable. The
//! LOT-21 review found it sitting behind a green guard that only checked the
//! markup was *present*, while the branch above it won every time. Here the
//! index holds three games, so "aucun fichier .lua" and the card grid are both
//! live alternatives — the banner has to beat them on the real screen.

import { defineSuite, assert } from '../suite.ts';
import { SAMPLE, SAMPLE_LUA } from '../fixtures.ts';

const BANNER = 'Index périmé';
const CARD = 'article.lift-card';

async function bodyText(app: import('../harness.ts').App): Promise<string> {
  return (await (await app.session.find('body')).text());
}

export default defineSuite({
  name: 'integrity',
  seed: {
    index: SAMPLE,
    lua: SAMPLE_LUA,
    files: {
      // 32 ASCII characters = the 32 bytes `hmac::load_or_create_key` expects.
      // Its presence is what makes a missing/!invalid sidecar fail closed
      // rather than silently migrate.
      'hmac.key': 'cle-de-test-pour-le-banc-e2e-32b',
      // A sidecar that is not a valid tag for this index.
      'library/index.hmac': 'LV-HMAC-v1 ceci-nest-pas-un-tag-valide',
    },
  },
  cases: {
    "un sidecar invalide affiche la bannière d'index périmé": async (app) => {
      const s = app.session;
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]')).click();
      await s.waitUntil('la bannière apparaît', async () => (await bodyText(app)).includes(BANNER));
      const text = await bodyText(app);
      assert.includes(text, 'ne correspond plus à sa signature');
      assert.includes(text, "Accepter l'index tel quel");
    },

    'la bannière gagne sur la grille et sur le message de bibliothèque vide': async (app) => {
      const s = app.session;
      const cards = await s.findAll(CARD);
      assert.equal(cards.length, 0, "aucune carte ne doit s'afficher sous un index non vérifié");
      const text = await bodyText(app);
      // The seeded index holds three games, so this message would be a lie —
      // and it is the branch that used to win.
      assert.excludes(text, 'Aucun fichier .lua pour le moment');
    },

    "la ré-adoption demande une confirmation explicite avant d'agir": async (app) => {
      const s = app.session;
      // Located by its text and tagged, since the button carries no stable
      // selector of its own and CSS cannot match on text content.
      const found = await s.execute<boolean>(
        `const b = [...document.querySelectorAll('button')].find(x => x.textContent.includes("Accepter l'index tel quel"));
         if (!b) return false; b.setAttribute('data-e2e-readopt', ''); return true`,
      );
      assert.ok(found, "le bouton de ré-adoption est introuvable");

      await s.waitUntil('aucun toast ne recouvre l’écran', async () =>
        (await s.findAll('button[aria-label^="Fermer : "]')).length === 0,
      );
      await (await s.find('[data-e2e-readopt]')).click();
      // One click must not re-adopt: the way out of fail-closed is a decision.
      await s.waitUntil('le bouton passe en demande de confirmation', async () =>
        (await bodyText(app)).includes('Accepter sans vérification ?'),
      );
      assert.includes(await bodyText(app), BANNER, 'un seul clic a déjà ré-adopté');
    },

    'la ré-adoption confirmée rend la bibliothèque': async (app) => {
      const s = app.session;
      const confirm = await s.execute<boolean>(
        `const b = [...document.querySelectorAll('button')].find(x => x.textContent.includes('Accepter sans vérification ?'));
         if (!b) return false; b.setAttribute('data-e2e-confirm', ''); return true`,
      );
      assert.ok(confirm, 'le bouton de confirmation est introuvable');
      await s.waitUntil('aucun toast ne recouvre l’écran', async () =>
        (await s.findAll('button[aria-label^="Fermer : "]')).length === 0,
      );
      await (await s.find('[data-e2e-confirm]')).click();

      await s.waitUntil('la bannière disparaît', async () => !(await bodyText(app)).includes(BANNER), 20_000);
      await s.waitUntil('les trois jeux reviennent', async () => (await s.findAll(CARD)).length === 3, 20_000);
    },
  },
});

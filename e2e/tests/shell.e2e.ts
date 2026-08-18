//! The application shell: navigation, theme switch, help dialog.
//!
//! Every one of these was previously "verified" by reading the Svelte source.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { defineSuite, assert } from '../suite.ts';
import { Key } from '../webdriver.ts';

const VIEWS = [
  'Bibliothèque',
  'Nouveautés',
  'Statistiques',
  'Steam & Outils',
  'Journaux',
  'Remerciements',
  'Réglages',
];

const VIEW_MARKERS: Record<string, string> = {
  Bibliothèque: 'Aucun fichier .lua pour le moment.',
  Nouveautés: 'Nouveautés',
  // `stats.empty.title` is also the empty-news title. The hint stays in the
  // StatsView-only branch and therefore lets the exclusion assertion distinguish them.
  Statistiques: 'Recherchez et ajoutez des jeux pour voir leurs statistiques ici.',
  'Steam & Outils': 'État de Steam et SteamTools — installation, réparation et dossiers.',
  Journaux: 'Journaux',
  Remerciements: 'Remerciements',
  Réglages: 'Réglages',
};

export default defineSuite({
  name: 'shell',
  cases: {
    "l'application démarre sur la bibliothèque, licence et onboarding franchis": async (app) => {
      const s = app.session;
      await s.waitFor('nav button[data-tip="Bibliothèque"]');
      // Wait for the heading rather than sampling `body` once: the nav mounts
      // before the main region, so a single read raced the render and made this
      // case fail roughly one run in five.
      await s.waitUntil('la bibliothèque vide est rendue', async () =>
        (await (await s.find('body')).text()).includes('Aucun fichier .lua pour le moment.'),
      );
      // The licence gate and the onboarding both replace the shell entirely,
      // so seeing the nav at all proves neither is standing in the way.
      const body = await (await s.find('body')).text();
      assert.excludes(body, 'Identifiant machine');
      assert.excludes(body, 'Bienvenue dans LuaVault');
    },

    'chaque entrée de navigation ouvre sa vue': async (app) => {
      const s = app.session;
      for (const label of VIEWS) {
        const button = await s.waitFor(`nav button[data-tip="${label}"]`);
        await button.click();
        // The active entry is the one carrying the shadowed surface class.
        await s.waitUntil(`« ${label} » devient l'entrée active`, async () => {
          const cls = (await button.attribute('class')) ?? '';
          return cls.includes('bg-surface/80');
        });
        // And the main region actually re-rendered: exactly one nav entry is active.
        const active = await s.execute<number>(
          `return document.querySelectorAll('nav button[class*="bg-surface/80"]').length`,
        );
        assert.equal(active, 1, `une seule entrée active pendant « ${label} »`);

        const marker = VIEW_MARKERS[label];
        await s.waitUntil(`« ${label} » rend son contenu`, async () =>
          (await (await s.find('main')).text()).includes(marker),
        );
        const mainText = await (await s.find('main')).text();
        for (const [otherLabel, otherMarker] of Object.entries(VIEW_MARKERS)) {
          if (otherLabel !== label) {
            assert.excludes(mainText, otherMarker, `« ${label} » ne rend pas « ${otherLabel} »`);
          }
        }
      }
    },

    'le bouton clair/sombre bascule data-mode sur <html>': async (app) => {
      const s = app.session;
      // `data-mode` carries light/dark; `data-theme` carries the colour theme
      // and must NOT move when only the mode is toggled (app.css hangs the
      // whole palette off the pair).
      const mode = () => s.execute<string | null>(`return document.documentElement.dataset.mode ?? null`);
      const theme = () => s.execute<string | null>(`return document.documentElement.dataset.theme ?? null`);

      const beforeMode = await mode();
      const beforeTheme = await theme();
      assert.ok(beforeMode === 'dark' || beforeMode === 'light', `data-mode inattendu : ${beforeMode}`);

      await (await s.waitFor('aside button[data-tip^="Passer en mode"]')).click();
      await s.waitUntil('data-mode bascule', async () => (await mode()) !== beforeMode);
      assert.equal(await theme(), beforeTheme, 'la teinte ne doit pas bouger avec le mode');

      // Put it back, so the following cases start from the same appearance.
      await (await s.waitFor('aside button[data-tip^="Passer en mode"]')).click();
      await s.waitUntil('data-mode revient', async () => (await mode()) === beforeMode);
    },

    "l'apparence choisie est bien celle qui est sauvegardée": async (app) => {
      // The screen showing the new mode proves nothing about what was written
      // to disk: `persist()` runs outside the View Transition callback, so it
      // can read the value *before* the mutation and save the previous one.
      // Only the real config.json settles it — and only the real WebView2 has
      // a real `startViewTransition`, which is why this case belongs here and
      // not in the unit suite.
      const s = app.session;
      const mode = () => s.execute<string | null>(`return document.documentElement.dataset.mode ?? null`);
      const savedDark = (): boolean | null | undefined => {
        try {
          const raw = readFileSync(join(app.sandbox, 'config.json'), 'utf8');
          return (JSON.parse(raw) as { dark_mode?: boolean | null }).dark_mode;
        } catch {
          return undefined; // not written yet
        }
      };

      const before = await mode();
      await (await s.waitFor('aside button[data-tip^="Passer en mode"]')).click();
      await s.waitUntil('data-mode bascule', async () => (await mode()) !== before);
      const shown = await mode();

      await s.waitUntil('config.json enregistre une apparence', async () => savedDark() !== undefined);
      assert.equal(
        savedDark(),
        shown === 'dark',
        `l'écran affiche « ${shown} » : config.json doit porter la même chose, pas la précédente`,
      );

      // Put the appearance back for the cases that follow.
      await (await s.waitFor('aside button[data-tip^="Passer en mode"]')).click();
      await s.waitUntil('data-mode revient', async () => (await mode()) === before);
    },

    "le dialogue d'aide s'ouvre, piège le focus (LOT-19) et se ferme par Échap": async (app) => {
      const s = app.session;
      await (await s.waitFor('aside button[data-tip="Raccourcis clavier"]')).click();
      const dialog = await s.waitFor('[role="dialog"][aria-modal="true"]');
      assert.includes(await dialog.text(), 'Raccourcis clavier');

      // The focus trap must have moved focus inside the dialog — otherwise
      // Tab would walk the page behind it.
      await s.waitUntil('le focus entre dans le dialogue', async () =>
        s.execute<boolean>(
          `return document.querySelector('[role="dialog"][aria-modal="true"]').contains(document.activeElement)`,
        ),
      );

      // Tab a few times: focus must stay inside, which is the whole contract.
      for (let i = 0; i < 6; i++) {
        await s.keys([Key.Tab]);
        const inside = await s.execute<boolean>(
          `const d = document.querySelector('[role="dialog"][aria-modal="true"]');
           return d ? d.contains(document.activeElement) : false`,
        );
        assert.ok(inside, `le focus est sorti du dialogue après ${i + 1} Tab`);
      }

      await s.keys([Key.Escape]);
      await s.waitUntil('le dialogue se ferme', async () =>
        (await s.maybeFind('[role="dialog"][aria-modal="true"]')) === null,
      );
    },
  },
});

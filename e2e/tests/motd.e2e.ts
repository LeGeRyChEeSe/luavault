//! The message of the day — published on the stand-in release host, signed
//! with the real primary key, and read by the real binary.
//!
//! What only this bench can settle: that the popup opens on its own at
//! startup, that the markdown is rendered as elements rather than injected as
//! markup, that « Ne plus afficher » reaches `config.json`, and that the
//! sidebar button brings the notice back with the box still ticked.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { defineSuite, assert } from '../suite.ts';
import { Key } from '../webdriver.ts';

const ID = 'bench-motd-0001';

/// Expires an hour from launch; a message published in the past and still
/// active is the ordinary case.
const MOTD = {
  schema: 1,
  message: {
    id: ID,
    published_at: '2026-09-16T10:00:00Z',
    expires_at: new Date(Date.now() + 60 * 60 * 1000).toISOString(),
    severity: 'warning',
    title: { fr: 'Panne du banc', en: 'Bench outage' },
    body: {
      // The `<b>` is the point: it must show as text, never become a tag.
      fr: 'Le serveur est {red}**indisponible**{/} jusqu\'à 18 h.\n\n- suivi : [état du service](https://example.com/status)\n- balise brute : <b>pas en gras</b>',
      en: 'The server is {red}**unavailable**{/} until 6 pm.',
    },
  },
};

export default defineSuite({
  name: 'motd',
  seed: { update: { motd: MOTD } },
  cases: {
    "le message s'ouvre de lui-même au lancement, en français, rendu sans injection": async (app) => {
      const s = app.session;
      const dialog = await s.waitFor('[data-motd-dialog]', 20_000);
      const text = await dialog.text();
      assert.includes(text, 'Panne du banc', 'le titre fr doit être affiché');
      assert.includes(text, 'indisponible', 'le corps fr doit être affiché');
      assert.excludes(text, 'unavailable', 'la locale fr ne doit pas afficher le corps en');
      assert.includes(text, 'Avertissement', 'le ton warning porte son libellé');

      // Rendered as elements: a <strong> carrying the bold word, inside a span
      // carrying the rose tone — and the raw `<b>` shown as text, with no <b>
      // element anywhere in the dialog.
      const shape = await s.execute<{ strong: number; rose: number; b: number; link: string | null }>(
        `const d = document.querySelector('[data-motd-body]');
         return {
           strong: [...d.querySelectorAll('strong')].filter(e => e.textContent === 'indisponible').length,
           rose: [...d.querySelectorAll('span[class*="text-rose-deep"] strong')].length,
           b: d.querySelectorAll('b').length,
           link: d.querySelector('a')?.getAttribute('href') ?? null,
         }`,
      );
      assert.equal(shape.strong, 1, 'le mot en ** est un <strong>');
      assert.equal(shape.rose, 1, 'le <strong> vit dans le span du ton rose');
      assert.equal(shape.b, 0, 'la balise <b> du texte ne doit jamais devenir un élément');
      assert.includes(text, '<b>pas en gras</b>', 'la balise brute est affichée comme du texte');
      assert.equal(shape.link, 'https://example.com/status', 'le lien https est un <a> avec son href');

      // The stub answered both signed files — the notice came from the network,
      // not from any seed on disk.
      assert.ok(app.updateStub.calls.includes('/motd.json'), 'motd.json a été demandé');
      assert.ok(app.updateStub.calls.includes('/motd.json.sig'), 'motd.json.sig a été demandé');
    },

    '« Ne plus afficher » puis Fermer écrit l\'identifiant dans config.json': async (app) => {
      const s = app.session;
      const dialog = await s.waitFor('[data-motd-dialog]');
      await (await dialog.find('[data-motd-remember]')).click();
      await (await dialog.find('[data-motd-close]')).click();
      await s.waitUntil('la fenêtre se ferme', async () =>
        s.execute<boolean>(`return document.querySelector('[data-motd-dialog]') === null`),
      );

      const saved = (): string | null | undefined => {
        try {
          const raw = readFileSync(join(app.sandbox, 'config.json'), 'utf8');
          return (JSON.parse(raw) as { motd_dismissed_id?: string | null }).motd_dismissed_id;
        } catch {
          return undefined;
        }
      };
      await s.waitUntil('config.json porte motd_dismissed_id', async () => saved() === ID);
      assert.equal(saved(), ID, "l'identifiant écarté doit être celui du message");
    },

    'le bouton de la barre latérale réaffiche le message, case encore cochée': async (app) => {
      const s = app.session;
      const button = await s.waitFor('aside button[data-motd-open]');
      await button.click();
      const dialog = await s.waitFor('[data-motd-dialog]');
      assert.includes(await dialog.text(), 'Panne du banc');
      const checked = await s.execute<boolean>(`return document.querySelector('[data-motd-remember]').checked`);
      assert.equal(checked, true, 'la case reflète le choix mémorisé');

      // Unticking and closing clears the choice: the next launch shows it again.
      await (await dialog.find('[data-motd-remember]')).click();
      await (await dialog.find('[data-motd-close]')).click();
      await s.waitUntil('la fenêtre se ferme', async () =>
        s.execute<boolean>(`return document.querySelector('[data-motd-dialog]') === null`),
      );
      await s.waitUntil('config.json oublie motd_dismissed_id', async () => {
        const raw = readFileSync(join(app.sandbox, 'config.json'), 'utf8');
        return (JSON.parse(raw) as { motd_dismissed_id?: string | null }).motd_dismissed_id === null;
      });
    },

    'Échap ferme la fenêtre sans toucher au choix': async (app) => {
      const s = app.session;
      await (await s.waitFor('aside button[data-motd-open]')).click();
      await s.waitFor('[data-motd-dialog]');
      await s.keys([Key.Escape]);
      await s.waitUntil('la fenêtre se ferme sur Échap', async () =>
        s.execute<boolean>(`return document.querySelector('[data-motd-dialog]') === null`),
      );
      const raw = readFileSync(join(app.sandbox, 'config.json'), 'utf8');
      assert.equal(
        (JSON.parse(raw) as { motd_dismissed_id?: string | null }).motd_dismissed_id ?? null,
        null,
        'Échap ne mémorise rien',
      );
    },
  },
});

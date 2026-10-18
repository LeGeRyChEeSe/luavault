//! The buttons of a message of the day — signed on the stand-in update server,
//! rendered by the real window, resolved by the real backend.
//!
//! What only this bench can settle: that the closed set of kinds is enforced on
//! the real binary (a kind it does not know shows no button, the others keep
//! their order), that a button which closes Steam asks for a second click and
//! does nothing on the first, that the second reaches the backend (the sandbox's
//! fake Steam has no `steam.exe`, so the answer is a refusal and nothing on disk
//! changes — the end-to-end wiring with no side effect), and that the tools tab
//! offers its compatibility card only while a live message carries the fix.
//!
//! The message is also PERSISTENT and DIAGNOSED: the fake Steam holds SteamTools
//! (`OpenSteamTool.dll` + `xinput1_4.dll`) and a client at the version that breaks
//! it, and is not pinned. So the fix is needed here: the window may not be silenced
//! for good (no « Ne plus afficher »), and a banner stays on every tab.

import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { defineSuite, assert } from '../suite.ts';


const MOTD = {
  schema: 1,
  message: {
    id: 'bench-motd-actions-0001',
    published_at: '2026-10-17T10:00:00Z',
    expires_at: new Date(Date.now() + 60 * 60 * 1000).toISOString(),
    severity: 'warning',
    persistent: true,
    title: { fr: 'Correctif du banc', en: 'Bench fix' },
    body: { fr: 'Une mise à jour de Steam pose problème.', en: 'A Steam update is a problem.' },
    actions: [
      {
        id: 'steam_downgrade',
        kind: 'steam_downgrade',
        snapshot: '20261006145653',
        manifest_sha256: '40ec2f2aac99fb65c62b7b60db77e8e1575e0c8b59cfeba80294eeb1c963731c',
        affects_from: '1791918634',
      },
      // A kind this build has never heard of: its button must not exist.
      { id: 'inconnu', kind: 'format_disk' },
      { id: 'restart_steam', kind: 'restart_steam' },
      { id: 'page', kind: 'open_url', url: 'https://example.com/info' },
    ],
  },
};

export default defineSuite({
  name: 'motd-actions',
  seed: {
    update: { motd: MOTD },
    // A Steam at the version that breaks SteamTools, with SteamTools installed.
    files: {
      'steam/OpenSteamTool.dll': 'x',
      'steam/xinput1_4.dll': 'x',
      'steam/package/steam_client_win64.manifest': '"win64"\n{\n\t"version"\t\t"1791918634"\n}\n',
    },
  },
  cases: {
    "les boutons connus s'affichent dans l'ordre, un type inconnu n'en produit aucun": async (app) => {
      const s = app.session;
      await s.waitFor('[data-motd-dialog]', 20_000);
      const shown = await s.execute<{ id: string; kind: string }[]>(
        `return [...document.querySelectorAll('[data-motd-action]')].map(e => ({ id: e.getAttribute('data-motd-action'), kind: e.getAttribute('data-motd-kind') }))`,
      );
      assert.equal(
        JSON.stringify(shown.map((a) => a.id)),
        JSON.stringify(['steam_downgrade', 'restart_steam', 'page']),
        "l'action de type inconnu est écartée, les autres gardent leur ordre",
      );
      const first = await s.execute<string>(`return document.querySelector('[data-motd-action="steam_downgrade"]').textContent.trim()`);
      assert.includes(first, 'Appliquer le correctif Steam', 'libellé par défaut du type, en français');

      // Persistent and needed: « Ne plus afficher » is withdrawn, the reason is given.
      assert.equal(await s.maybeFind('[data-motd-remember]'), null, 'la case est retirée tant que le correctif est nécessaire');
      assert.includes(await (await s.waitFor('[data-motd-forced]')).text(), 'tant que le correctif', 'et la raison est dite');
    },

    'un bouton qui ferme Steam ne fait rien au premier clic, et le second atteint le backend': async (app) => {
      const s = app.session;
      const button = await s.waitFor('[data-motd-action="steam_downgrade"] button');
      await button.click();
      await s.waitUntil('le bouton est armé', async () =>
        s.execute<boolean>(
          `return document.querySelector('[data-motd-action="steam_downgrade"] button').getAttribute('data-confirm') === 'armed'`,
        ),
      );
      assert.equal(existsSync(join(app.steamDir, 'steam.cfg')), false, 'le premier clic ne touche à rien');
      assert.equal(await s.maybeFind('[data-motd-outcome]'), null, 'le premier clic ne répond rien');

      await button.click();
      await s.waitUntil('le backend répond', async () =>
        s.execute<boolean>(`return document.querySelector('[data-motd-outcome]') !== null`),
      );
      const outcome = await s.execute<{ kind: string | null; text: string }>(
        `const e = document.querySelector('[data-motd-outcome]'); return { kind: e.getAttribute('data-motd-outcome'), text: e.textContent.trim() }`,
      );
      assert.equal(outcome.kind, 'error', 'le faux Steam du banc n\'a pas de steam.exe : refus');
      // Two honest refusals, and which one depends on the machine running the bench:
      // the guard answers first when a real game is running (RunningAppID is read
      // from the real registry), otherwise the missing executable does.
      assert.ok(
        outcome.text.includes('steam.exe') || outcome.text.includes('Un jeu est en cours'),
        `le refus nomme le fichier manquant ou le jeu en cours : ${outcome.text}`,
      );
      assert.equal(existsSync(join(app.steamDir, 'steam.cfg')), false, 'un refus ne laisse aucun blocage derrière lui');
    },

    "l'onglet Outils offre la carte de compatibilité tant que le message porte le correctif": async (app) => {
      const s = app.session;
      await (await s.waitFor('[data-motd-close]')).click();
      await s.waitUntil('la fenêtre se ferme', async () =>
        s.execute<boolean>(`return document.querySelector('[data-motd-dialog]') === null`),
      );
      await (await s.waitFor('nav button[data-tip="Steam & Outils"]')).click();
      const card = await s.waitFor('[data-tools-pin]');
      assert.includes(await card.text(), 'Compatibilité Steam', 'la carte porte son titre');
      assert.includes(await card.text(), 'Appliquer le correctif', 'et son bouton');
    },

    "la bannière reste sur tous les onglets, et son bouton atteint le backend": async (app) => {
      const s = app.session;
      const banner = await s.waitFor('[data-workaround-banner]');
      assert.includes(await banner.text(), 'Correctif Steam requis', 'la bannière dit ce qui est requis');
      // Still there on another tab: it belongs to the machine's state, not to a view.
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]')).click();
      await s.waitFor('[data-workaround-banner]');

      const button = await s.waitFor('[data-workaround-banner] button');
      await button.click();
      await s.waitUntil('le bouton est armé', async () =>
        s.execute<boolean>(`return document.querySelector('[data-workaround-banner] button').getAttribute('data-confirm') === 'armed'`),
      );
      assert.equal(existsSync(join(app.steamDir, 'steam.cfg')), false, 'le premier clic ne touche à rien');
      await button.click();
      await s.waitUntil('le backend répond', async () =>
        s.execute<boolean>(`return document.querySelector('[data-workaround-error]') !== null`),
      );
      const text = await (await s.waitFor('[data-workaround-error]')).text();
      assert.ok(
        text.includes('steam.exe') || text.includes('Un jeu est en cours'),
        `refus attendu (fichier manquant ou jeu en cours) : ${text}`,
      );
      assert.equal(existsSync(join(app.steamDir, 'steam.cfg')), false, 'un refus ne laisse aucun blocage derrière lui');
      // The fix was not applied: the banner is still there.
      assert.ok((await s.maybeFind('[data-workaround-banner]')) !== null, 'rien n\'est appliqué : la bannière reste');
    },
  },
});

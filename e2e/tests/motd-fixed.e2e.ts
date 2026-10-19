//! The same persistent message on a machine that does NOT need the fix: Steam is
//! already pinned (`steam.cfg`). Nothing insists: no banner, the ordinary
//! « Ne plus afficher » is back, and the tools tab offers the way back instead.

import { defineSuite, assert } from '../suite.ts';

const MOTD = {
  schema: 1,
  message: {
    id: 'bench-motd-fixed-0001',
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
    ],
  },
};

export default defineSuite({
  name: 'motd-fixed',
  seed: {
    update: { motd: MOTD },
    files: {
      'steam/OpenSteamTool.dll': 'x',
      'steam/xinput1_4.dll': 'x',
      'steam/package/steam_client_win64.manifest': '"win64"\n{\n\t"version"\t\t"1791918634"\n}\n',
      'steam/steam.cfg': 'BootStrapperInhibitAll=enable\r\nBootStrapperForceSelfUpdate=disable\r\n',
    },
  },
  cases: {
    "Steam déjà figé : le message est ordinaire, la case « Ne plus afficher » est là": async (app) => {
      const s = app.session;
      await s.waitFor('[data-motd-dialog]', 20_000);
      assert.ok((await s.maybeFind('[data-motd-remember]')) !== null, 'la case est de retour');
      assert.equal(await s.maybeFind('[data-motd-forced]'), null, 'rien ne dit que le message insiste');
    },

    "Steam déjà figé : le bouton dit que c'est fait et n'invite pas à recommencer": async (app) => {
      const s = app.session;
      const status = await s.waitFor('[data-motd-status="steam_downgrade"]');
      assert.equal(await status.attribute('data-motd-state'), 'done', 'état vérifié sur le disque, pas deviné');
      assert.includes(await status.text(), 'déjà en place', 'la phrase le dit');
      assert.equal(await s.maybeFind('[data-motd-action="steam_downgrade"]'), null, 'plus de bouton à presser deux fois');
    },

    "aucune bannière, et les Outils offrent le retour arrière": async (app) => {
      const s = app.session;
      await (await s.waitFor('[data-motd-close]')).click();
      await s.waitUntil('la fenêtre se ferme', async () =>
        s.execute<boolean>(`return document.querySelector('[data-motd-dialog]') === null`),
      );
      assert.equal(await s.maybeFind('[data-workaround-banner]'), null, 'Steam est figé : pas de bannière');
      await (await s.waitFor('nav button[data-tip="Steam & Outils"]')).click();
      const card = await s.waitFor('[data-tools-pin]');
      const text = await card.text();
      assert.includes(text, 'mises à jour bloquées', 'la carte dit que Steam est figé');
      assert.includes(text, 'Rétablir les mises à jour', 'et offre le retour arrière');
      assert.equal(await s.maybeFind('[data-workaround-banner]'), null, 'toujours aucune bannière');
    },
  },
});

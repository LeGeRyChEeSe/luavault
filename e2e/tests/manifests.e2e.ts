//! A game file Steam cannot get a code for, seen by the real window.
//!
//! The fake Steam holds SteamTools and an empty `content_log.txt`. The resident guard
//! (switched on for this suite only) reads what is appended to that log, as it reads
//! Steam's own. What only this bench can settle: that the real refusal line reaches the
//! backend and the window, that the banner and the card show up with the two ways to a key,
//! that a file already in place is left alone, that a key of the wrong shape is refused
//! before it is stored, and that nothing here reaches the network (no key is ever valid).

import { appendFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { defineSuite, assert } from '../suite.ts';

const line = (depot: number, gid: string) =>
  `[2026-10-24 05:36:08] CDepotDownloadMgr::BYldRequestDepotManifest(App: 3883570, Depot: ${depot}, Manifest: ${gid}, branch: ): Failed to get manifest request code, 'Access Denied'\r\n`;

const status = (app: any) => app.session.invoke('manifest_fallback_status');

export default defineSuite({
  name: 'manifests',
  seed: {
    guard: true,
    config: { background_guard: true, background_notify: false },
    files: {
      'steam/OpenSteamTool.dll': 'x',
      'steam/xinput1_4.dll': 'x',
      'steam/logs/content_log.txt': '',
    },
  },
  cases: {
    "au départ rien ne manque : ni bannière, ni carte": async (app) => {
      const s = app.session;
      await s.waitFor('nav', 20_000);
      assert.equal(await s.maybeFind('[data-manifest-banner]'), null, 'pas de bannière');
      assert.equal((await status(app)).pending, 0, 'rien en attente');
    },

    "un fichier déjà en place ne déclenche rien": async (app) => {
      const s = app.session;
      const depotcache = join(app.steamDir, 'depotcache');
      const { mkdirSync, writeFileSync } = await import('node:fs');
      mkdirSync(depotcache, { recursive: true });
      writeFileSync(join(depotcache, '3883571_111.manifest'), 'already here');
      appendFileSync(join(app.steamDir, 'logs', 'content_log.txt'), line(3883571, '111'));
      await new Promise((r) => setTimeout(r, 6_000));
      assert.equal((await status(app)).pending, 0, 'le fichier est là, Steam réessaie seul');
      assert.equal(await s.maybeFind('[data-manifest-banner]'), null, 'pas de bannière');
    },

    "le refus de Steam fait apparaître la bannière, avec les deux issues": async (app) => {
      const s = app.session;
      appendFileSync(join(app.steamDir, 'logs', 'content_log.txt'), line(3883571, '3377765570892908199'));
      await s.waitFor('[data-manifest-banner]', 20_000);
      const view = await status(app);
      assert.equal(view.pending, 1, 'un fichier en attente');
      assert.equal(view.key_ready, false, 'aucune clé');
      const text = await (await s.find('[data-manifest-banner]')).text();
      assert.includes(text, 'Un fichier de jeu manque');
      assert.includes(text, 'Obtenir une clé');
      assert.includes(text, "J'ai ma clé");
    },

    "« J'ai ma clé » mène à la carte, avec une page à ouvrir pour chaque source": async (app) => {
      const s = app.session;
      const buttons = await (await s.find('[data-manifest-banner]')).findAll('button');
      let paste = null;
      for (const b of buttons) if ((await b.text()).includes("J'ai ma clé")) paste = b;
      assert.ok(paste, 'le bouton existe');
      await paste!.click();
      await s.waitFor('[data-tools-manifests]', 10_000);
      for (const provider of ['manifesthub', 'hubcap']) {
        const row = await s.find(`[data-manifest-provider="${provider}"]`);
        assert.includes(await row.text(), 'Ouvrir la page', `${provider} a sa page`);
        assert.equal((await row.findAll('input')).length, 1, `${provider} a son champ`);
      }
      assert.includes(await (await s.find('[data-tools-manifests]')).text(), '1 en attente');
    },

    "une clé de mauvaise forme est refusée et rien n'est écrit": async (app) => {
      const s = app.session;
      for (const [command, bad] of [
        ['set_manifesthub_key', 'ceci est une phrase'],
        ['set_manifesthub_key', 'smm1234567890abcdef'],
        ['set_hubcap_key', 'abc1234567890'],
      ] as const) {
        const answer = await s.invoke<string>(command, { key: bad });
        assert.ok(typeof answer === 'string' && answer.startsWith('ERR:'), `${command} refuse « ${bad} »`);
      }
      assert.equal(existsSync(join(app.sandbox, 'manifest_keys.json')), false, 'aucun fichier de clés');
      assert.equal((await status(app)).key_ready, false, 'toujours aucune clé');
    },

    "le champ refuse aussi, à l'écran, et le message le dit": async (app) => {
      const s = app.session;
      const input = await s.find('[data-manifest-provider="manifesthub"] input');
      await input.sendKeys('pas une clé');
      const row = await s.find('[data-manifest-provider="manifesthub"]');
      let save = null;
      for (const b of await row.findAll('button')) if ((await b.text()).includes('Enregistrer')) save = b;
      assert.ok(save, 'bouton Enregistrer');
      await save!.click();
      await s.waitUntil("l'erreur est affichée", async () =>
        s.execute<boolean>(`return document.body.innerText.includes("pas une clé ManifestHub")`),
      );
      assert.equal(existsSync(join(app.sandbox, 'manifest_keys.json')), false, 'toujours rien sur le disque');
    },
  },
});

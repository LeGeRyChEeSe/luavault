//! The downloads watcher on the real window: a file dropped into the downloads
//! folder is recognised, offered in a popup, and added through the library's own
//! import commands — or left alone.
//!
//! The folder is the sandbox's `downloads\` (`LV_DOWNLOADS_DIR`, honoured in debug
//! builds only): the bench must never watch, or write to, the real Downloads of
//! whoever runs it. The watcher polls every 2 s and wants a size seen twice, so a
//! file takes about 5 s to be offered; the waits below are sized for that.

import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { defineSuite, assert } from '../suite.ts';
import { SAMPLE, SAMPLE_LUA } from '../fixtures.ts';
import type { App } from '../harness.ts';

const MODAL = '[data-downloads-modal]';
const OFFER_MS = 30_000;

const lua = (appId: string) =>
  `-- AppID ${appId} | Generated on 2026-01-01 05:55 UTC\n-- Main Application\naddappid(${appId}, 1, "340ae2fcb31a92507c1e0a9139f7cac3ab50f45a8c701f6d9f06b1793aca9981")\n`;

/// Zip magic bytes plus filler: the importer checks the head, never the entries.
const archive = () => Buffer.concat([Buffer.from('PK\x03\x04'), Buffer.alloc(64, 1)]);

async function clickButton(app: App, label: string) {
  const clicked = await app.session.execute<boolean>(
    `const b = [...document.querySelectorAll('button')].find(x => x.textContent.trim().startsWith(arguments[0]));
     if (!b) return false; b.click(); return true`,
    [label],
  );
  assert.ok(clicked, `bouton « ${label} » introuvable`);
}

async function offered(app: App): Promise<string[]> {
  return app.session.execute<string[]>(
    `return [...document.querySelectorAll('[data-downloads-item]')].map(li => li.getAttribute('data-downloads-item'))`,
  );
}

async function badge(app: App): Promise<string | null> {
  return app.session.execute<string | null>(
    `const b = document.querySelector('nav button[data-tip="Bibliothèque"] span.rounded-full:last-child');
     return b ? b.textContent.trim() : null`,
  );
}

const persisted = (sandbox: string): boolean | null | undefined => {
  try {
    return (JSON.parse(readFileSync(join(sandbox, 'config.json'), 'utf8')) as { watch_downloads?: boolean | null }).watch_downloads;
  } catch {
    return undefined;
  }
};

export default defineSuite({
  name: 'downloads',
  seed: { index: SAMPLE, lua: SAMPLE_LUA },
  cases: {
    "le bouton « Trouver un fichier » est là, avec sa phrase, et on ne le presse pas": async (app) => {
      // Pressing it would open the real default browser on a real site: the bench
      // only settles that it is offered, and what it says about itself.
      const s = app.session;
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]', 20_000)).click();
      await s.waitFor('input[placeholder="Filtrer par nom ou AppID…"]');
      const tip = await s.execute<string | null>(
        `const b = [...document.querySelectorAll('button')].find(x => x.textContent.trim() === 'Trouver un fichier');
         return b ? b.getAttribute('data-tip') : null`,
      );
      assert.ok(tip !== null, 'le bouton est dans la barre de la bibliothèque');
      assert.includes(tip as string, "Source testée avec LuaVault, sans lien avec l'app.", 'la phrase convenue est dite');
      assert.excludes(tip as string, 'http', 'aucune adresse dans le texte affiché');
    },

    "un fichier reconnu est proposé, le bruit ne l'est jamais, et rien n'est déplacé": async (app) => {
      const s = app.session;
      const dir = app.downloadsDir;
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]', 20_000)).click();
      await s.waitFor('input[placeholder="Filtrer par nom ou AppID…"]');
      // Nobody pressed anything: the folder has been watched since the app started.
      await s.waitFor('button[data-tip^="L\'application surveille votre dossier Téléchargements"]');

      // Noise first: none of these may ever raise the popup.
      writeFileSync(join(dir, 'notes.txt'), 'rien à voir');
      writeFileSync(join(dir, 'Partiel.lua.crdownload'), lua('111'));
      writeFileSync(join(dir, 'sans-declaration.lua'), 'print("pas un fichier LuaVault")\n');
      writeFileSync(join(dir, 'un-mod-quelconque.zip'), archive());
      writeFileSync(join(dir, 'Mon Jeu Test.lua'), lua('1593030'));

      await s.waitFor(MODAL, OFFER_MS);
      assert.equal(JSON.stringify(await offered(app)), JSON.stringify(['Mon Jeu Test.lua']), 'seul le .lua reconnu est proposé');
      for (const name of ['notes.txt', 'Partiel.lua.crdownload', 'sans-declaration.lua', 'un-mod-quelconque.zip', 'Mon Jeu Test.lua']) {
        assert.ok(existsSync(join(dir, name)), `${name} doit rester où il est`);
      }
    },

    "Ignorer ferme la fenêtre sans rien ajouter": async (app) => {
      const s = app.session;
      await (await s.waitFor('[data-downloads-ignore]')).click();
      await s.waitUntil('la fenêtre se ferme', async () => (await s.maybeFind(MODAL)) === null);
      assert.equal(await badge(app), '3', 'la bibliothèque garde ses trois jeux');
      assert.ok(existsSync(join(app.downloadsDir, 'Mon Jeu Test.lua')), 'le fichier reste dans Téléchargements');
    },

    "Échap ignore aussi, il n'ajoute jamais": async (app) => {
      const s = app.session;
      writeFileSync(join(app.downloadsDir, 'Echap Jeu.lua'), lua('1593031'));
      await s.waitFor(MODAL, OFFER_MS);
      await s.execute(`document.querySelector('[data-downloads-modal]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
      await s.waitUntil('la fenêtre se ferme', async () => (await s.maybeFind(MODAL)) === null);
      assert.equal(await badge(app), '3', 'Échap n\'a rien ajouté');
    },

    "Ajouter importe le .lua et le patch par les commandes de la bibliothèque": async (app) => {
      const s = app.session;
      const dir = app.downloadsDir;
      writeFileSync(join(dir, 'Autre Jeu.lua'), lua('1593032'));
      writeFileSync(join(dir, '264710_online_fix.zip'), archive());
      writeFileSync(join(dir, '999_online_fix.zip'), archive());
      await s.waitFor(MODAL, OFFER_MS);
      await s.waitUntil('les trois fichiers sont listés', async () => (await offered(app)).length === 3, OFFER_MS);

      // The patch for a game that is not in the library is shown, with its reason,
      // and cannot be ticked.
      const blocked = await s.execute<{ disabled: boolean; reason: string }>(
        `const li = document.querySelector('[data-downloads-item="999_online_fix.zip"]');
         return { disabled: li.querySelector('input').disabled, reason: li.querySelector('[data-downloads-reason]').textContent.trim() }`,
      );
      assert.ok(blocked.disabled, 'la case du patch sans jeu est grisée');
      assert.includes(blocked.reason, "n'est pas encore dans votre bibliothèque", 'la raison est dite');

      await (await s.waitFor('[data-downloads-add]')).click();
      await s.waitUntil('la fenêtre se ferme', async () => (await s.maybeFind(MODAL)) === null);
      await s.waitUntil('le jeu importé rejoint la bibliothèque', async () => (await badge(app)) === '4');
      assert.ok(
        existsSync(join(app.sandbox, 'library', 'fixes', '264710_online_fix.rar')),
        "l'archive est rangée à l'emplacement canonique des patchs",
      );
      assert.ok(!existsSync(join(app.sandbox, 'library', 'fixes', '999_online_fix.rar')), "le patch refusé n'est pas importé");
      assert.ok(existsSync(join(dir, 'Autre Jeu.lua')), 'les fichiers d\'origine ne sont pas supprimés');
    },

    "un bypass DRM est détecté, dit ce qu'il remplace, et s'ajoute à l'emplacement du patch": async (app) => {
      const s = app.session;
      const dir = app.downloadsDir;
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]', 20_000)).click();
      const stored = join(app.sandbox, 'library', 'fixes', '264710_online_fix.rar');
      const before = statSync(stored).size;
      // The browser's own duplicate counter must not turn the file into another game.
      const bytes = Buffer.concat([Buffer.from('Rar!\x1a\x07\x01\x00'), Buffer.alloc(300, 7)]);
      writeFileSync(join(dir, '264710_bypass (1).rar'), bytes);
      await s.waitFor(MODAL, OFFER_MS);
      const row = await s.execute<{ label: string; replaces: boolean; reason: boolean }>(
        `const li = document.querySelector('[data-downloads-item="264710_bypass (1).rar"]');
         return li ? { label: li.textContent, replaces: !!li.querySelector('[data-downloads-replaces]'), reason: !!li.querySelector('[data-downloads-reason]') } : null`,
      );
      assert.ok(row !== null, 'le bypass est proposé');
      assert.includes(row.label, 'Bypass DRM', "il est nommé bypass DRM, pas « patch »");
      assert.includes(row.label, 'AppID 264710', 'son AppID est lu dans le nom, pas dans le compteur du navigateur');
      assert.ok(row.replaces, 'il dit qu\'il remplacera le patch déjà enregistré');
      assert.ok(!row.reason, 'le jeu est dans la bibliothèque : rien ne le bloque');
      await (await s.waitFor('[data-downloads-add]')).click();
      await s.waitUntil('la fenêtre se ferme', async () => (await s.maybeFind(MODAL)) === null);
      await s.waitUntil("l'archive stockée est celle du bypass", async () => statSync(stored).size === bytes.length);
      assert.ok(bytes.length !== before, 'la taille a bien changé');
    },

    "ce qu'on ignore reste à un clic : « Fichiers détectés » le liste, sans ce que l'app a déjà": async (app) => {
      const s = app.session;
      const dir = app.downloadsDir;
      writeFileSync(join(dir, 'Ancien Jeu.lua'), lua('1593040'));
      await s.waitFor(MODAL, OFFER_MS);
      await (await s.waitFor('[data-downloads-ignore]')).click();
      await s.waitUntil('la fenêtre se ferme', async () => (await s.maybeFind(MODAL)) === null);

      await clickButton(app, 'Fichiers détectés');
      await s.waitFor(`${MODAL}[data-downloads-mode="browse"]`);
      await s.waitUntil('la liste est chargée', async () => (await offered(app)).includes('Ancien Jeu.lua'));
      const names = (await offered(app)).sort();
      // Ignored earlier or never answered: still there. Already in the library (the .lua added by the
      // previous cases) or the very file already stored (the bypass just added): not offered again.
      for (const wanted of ['Ancien Jeu.lua', 'Echap Jeu.lua', 'Mon Jeu Test.lua']) {
        assert.ok(names.includes(wanted), `${wanted} reste proposé (${names.join(', ')})`);
      }
      // The old patch archive is no longer the stored file (the bypass replaced it): it is another file for
      // the same game, so it is offered again, saying it would replace.
      assert.ok(names.includes('264710_online_fix.zip'), "l'ancien patch, différent du fichier stocké, reste proposé");
      assert.ok(
        await s.execute<boolean>(`return !!document.querySelector('[data-downloads-item="264710_online_fix.zip"] [data-downloads-replaces]')`),
        "et il dit qu'il remplacerait",
      );
      for (const gone of ['Autre Jeu.lua', '264710_bypass (1).rar', 'notes.txt']) {
        assert.ok(!names.includes(gone), `${gone} n'est plus proposé : l'application l'a déjà ou ce n'est pas pour elle`);
      }
      // A list the user opened starts with nothing ticked: adding is a choice.
      const ticked = await s.execute<number>(`return document.querySelectorAll('[data-downloads-list] input:checked').length`);
      assert.equal(ticked, 0, 'rien n\'est coché d\'avance');
      assert.ok(await s.execute<boolean>(`return document.querySelector('[data-downloads-add]').disabled`), '« Ajouter » attend un choix');

      await s.execute(
        `document.querySelector('[data-downloads-item="Ancien Jeu.lua"] input').click()`,
      );
      await (await s.waitFor('[data-downloads-add]')).click();
      await s.waitUntil('la fenêtre se ferme', async () => (await s.maybeFind(MODAL)) === null);
      await s.waitUntil("l'ancien jeu rejoint la bibliothèque", async () => (await badge(app)) === '5');
      assert.ok(existsSync(join(dir, 'Ancien Jeu.lua')), 'le fichier d\'origine reste en place');
    },

    "un .lua et son patch arrivent ensemble : cochés côte à côte, ils s'ajoutent d'un seul geste, le .lua d'abord": async (app) => {
      const s = app.session;
      const dir = app.downloadsDir;
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]', 20_000)).click();
      // The game 1593060 is NOT in the library: its patch alone could not be added.
      writeFileSync(join(dir, '1593060_online_fix.zip'), archive());
      writeFileSync(join(dir, 'Jeu Pair.lua'), lua('1593060'));
      await s.waitFor(MODAL, OFFER_MS);
      await s.waitUntil('les deux fichiers sont listés', async () => (await offered(app)).includes('Jeu Pair.lua') && (await offered(app)).includes('1593060_online_fix.zip'), OFFER_MS);
      const state = async () =>
        s.execute<{ patchDisabled: boolean; patchChecked: boolean; luaChecked: boolean }>(
          `const p = document.querySelector('[data-downloads-item="1593060_online_fix.zip"] input');
           const l = document.querySelector('[data-downloads-item="Jeu Pair.lua"] input');
           return { patchDisabled: p.disabled, patchChecked: p.checked, luaChecked: l.checked }`,
        );
      const together = await state();
      assert.ok(!together.patchDisabled && together.patchChecked, 'le patch est ajoutable : son .lua est coché à côté');
      // Untick the .lua: the patch loses its footing and is greyed again, with its reason.
      await s.execute(`document.querySelector('[data-downloads-item="Jeu Pair.lua"] input').click()`);
      await s.waitUntil('le patch se grise', async () => (await state()).patchDisabled);
      await s.execute(`document.querySelector('[data-downloads-item="Jeu Pair.lua"] input').click()`);
      await s.waitUntil('le patch redevient ajoutable', async () => !(await state()).patchDisabled);
      // Leave the two other candidates of earlier cases out: only this pair is added.
      await s.execute(
        `for (const li of document.querySelectorAll('[data-downloads-item]')) {
           const name = li.getAttribute('data-downloads-item');
           const input = li.querySelector('input');
           if (name !== 'Jeu Pair.lua' && name !== '1593060_online_fix.zip' && input.checked) input.click();
         }`,
      );
      await (await s.waitFor('[data-downloads-add]')).click();
      await s.waitUntil('la fenêtre se ferme', async () => (await s.maybeFind(MODAL)) === null);
      await s.waitUntil('le jeu rejoint la bibliothèque', async () => (await badge(app)) === '6');
      assert.ok(
        existsSync(join(app.sandbox, 'library', 'fixes', '1593060_online_fix.rar')),
        'le patch est rangé : le .lua a été importé avant lui',
      );
    },

    "un second dossier surveillé : ses fichiers sont proposés, et on revient au dossier Téléchargements": async (app) => {
      const s = app.session;
      const second = join(app.sandbox, 'downloads-bis');
      mkdirSync(second, { recursive: true });
      const set = await s.invoke<unknown>('set_downloads_dirs', { dirs: [app.downloadsDir, second] });
      assert.ok(!String(set).startsWith('ERR:'), `set_downloads_dirs : ${JSON.stringify(set)}`);
      const rejected = await s.invoke<string>('set_downloads_dirs', { dirs: [join(app.sandbox, 'n-existe-pas')] });
      assert.ok(String(rejected).startsWith('ERR:'), 'un dossier qui n\'existe pas est refusé');

      await (await s.waitFor('nav button[data-tip="Réglages"]')).click();
      await s.waitUntil('les deux dossiers sont listés', async () => (await s.findAll('[data-settings-downloads-dir]')).length === 2);
      const saved = JSON.parse(readFileSync(join(app.sandbox, 'config.json'), 'utf8')) as { watch_dirs?: string[] | null };
      assert.equal(saved.watch_dirs?.length, 2, 'la liste est écrite sur le disque');

      writeFileSync(join(second, 'Bis Jeu.lua'), lua('1593050'));
      await s.waitFor(MODAL, OFFER_MS);
      await s.waitUntil('le fichier du second dossier est proposé', async () => (await offered(app)).includes('Bis Jeu.lua'), OFFER_MS);
      await s.execute(`document.querySelector('[data-downloads-modal]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))`);
      await s.waitUntil('la fenêtre se ferme', async () => (await s.maybeFind(MODAL)) === null);

      await (await s.waitFor('[data-settings-downloads-dir-reset]')).click();
      await s.waitUntil('un seul dossier est listé', async () => (await s.findAll('[data-settings-downloads-dir]')).length === 1);
      await s.waitUntil('watch_dirs est effacé du disque', async () => {
        const again = JSON.parse(readFileSync(join(app.sandbox, 'config.json'), 'utf8')) as { watch_dirs?: string[] | null };
        return again.watch_dirs === null || again.watch_dirs === undefined;
      });
    },

    "l'interrupteur de Réglages est écrit sur le disque": async (app) => {
      const s = app.session;
      await (await s.waitFor('nav button[data-tip="Réglages"]')).click();
      const box = await s.waitFor('[data-settings-downloads-switch]', 20_000);
      assert.ok(persisted(app.sandbox) !== false, 'allumé au départ : personne ne l\'a éteint');
      assert.ok(await s.execute<boolean>(`return document.querySelector('[data-settings-downloads-switch]').checked`), "l'interrupteur se montre allumé");
      await box.click();
      await s.waitUntil('watch_downloads vaut false sur le disque', async () => persisted(app.sandbox) === false);
      await (await s.waitFor('[data-settings-downloads-switch]')).click();
      await s.waitUntil('watch_downloads vaut true sur le disque', async () => persisted(app.sandbox) === true);
    },
  },
});

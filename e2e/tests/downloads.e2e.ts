//! The downloads watcher on the real window: a file dropped into the downloads
//! folder is recognised, offered in a popup, and added through the library's own
//! import commands — or left alone.
//!
//! The folder is the sandbox's `downloads\` (`LV_DOWNLOADS_DIR`, honoured in debug
//! builds only): the bench must never watch, or write to, the real Downloads of
//! whoever runs it. The watcher polls every 2 s and wants a size seen twice, so a
//! file takes about 5 s to be offered; the waits below are sized for that.

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
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
      await clickButton(app, 'Détecter un téléchargement');
      await s.waitFor('button[data-tip="Cesse de surveiller le dossier Téléchargements."]');

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

    "l'interrupteur de Réglages est écrit sur le disque": async (app) => {
      const s = app.session;
      await (await s.waitFor('nav button[data-tip="Réglages"]')).click();
      const box = await s.waitFor('[data-settings-downloads-switch]', 20_000);
      assert.ok(persisted(app.sandbox) !== true, 'éteint au départ');
      await box.click();
      await s.waitUntil('watch_downloads vaut true sur le disque', async () => persisted(app.sandbox) === true);
      await (await s.waitFor('[data-settings-downloads-switch]')).click();
      await s.waitUntil('watch_downloads vaut false sur le disque', async () => persisted(app.sandbox) === false);
    },
  },
});

//! The password retry from the real game-card action. This has to execute the
//! dialog because PWD-01 only protects remembering a successful password, not
//! the `submitPassword()` → `applyFix()` connection itself.

import { Buffer } from 'node:buffer';
import { defineSuite, assert } from '../suite.ts';
import { entry } from '../fixtures.ts';

const APP_ID = '480';
const GAME_NAME = 'Password Fixture';
const ARCHIVE = Buffer.from(
  'UEsDBDMAAQBjAAAAIQAAAAAAKQAAAAsAAAAPAAsAT25saW5lRml4NjQuZGxsAZkHAAIAQUUDCACjg7XjprZY+860EIm+CeqfLqjSyh9Vc/EJS+3xAjvgkra9h90iXRlrl1BLAQIzADMAAQBjAAAAIQAAAAAAKQAAAAsAAAAPAAsAAAAAAAAAAACkgQAAAABPbmxpbmVGaXg2NC5kbGwBmQcAAgBBRQMIAFBLBQYAAAAAAQABAEgAAABhAAAAAAA=',
  'base64',
);

async function bodyText(app: import('../harness.ts').App): Promise<string> {
  return (await (await app.session.find('body')).text());
}

export default defineSuite({
  name: 'password-fix',
  seed: {
    index: [entry(APP_ID, GAME_NAME)],
    lua: { [APP_ID]: `addappid(${APP_ID})\n` },
    // Keep the Defender prompt out of this test: it is unrelated to the
    // archive retry, and `false` records the user's explicit choice.
    config: { defender_exclusions: false },
    files: {
      // A real AES-256 ZIP, deliberately named `.rar`: production detects the
      // ZIP magic bytes and asks for the archive password when no password is
      // supplied. Its sole payload replaces the fake game's original DLL.
      [`library/fixes/${APP_ID}_online_fix.rar`]: ARCHIVE,
      [`steam/steamapps/appmanifest_${APP_ID}.acf`]:
        `"AppState" { "appid" "${APP_ID}" "name" "${GAME_NAME}" "installdir" "${GAME_NAME}" "StateFlags" "4" }`,
      [`steam/steamapps/common/${GAME_NAME}/steam_api64.dll`]: 'pristine',
      // Startup synchronisation owns the Steam copy. Seed it too, so it
      // refreshes this fixture instead of treating the local entry as stale.
      [`steam/config/lua/${APP_ID}.lua`]: `addappid(${APP_ID})\n`,
    },
  },
  cases: {
    'le mot de passe soumis réessaie et installe réellement le patch': async (app) => {
      const s = app.session;
      await (await s.waitFor('nav button[data-tip="Bibliothèque"]')).click();

      await s.waitUntil("l'action d'installation du patch semé apparaît", () =>
        s.execute<boolean>(
          `const button = [...document.querySelectorAll('button')].find((b) =>
             b.textContent?.trim() === 'Installer le patch en ligne');
           if (!button) return false;
           button.setAttribute('data-e2e-install-password-fix', '');
           return true;`,
        ),
        20_000,
      );
      await (await s.find('[data-e2e-install-password-fix]')).click();

      await s.waitUntil('le dialogue de mot de passe s’ouvre après le premier échec', async () =>
        (await bodyText(app)).includes('Mot de passe requis'),
      );
      const password = await s.waitFor('#archive-password');
      await password.sendKeys('LuaVault');
      const retryMarked = await s.execute<boolean>(
        `const button = [...document.querySelectorAll('button')].find((b) =>
           b.textContent?.trim() === 'Réessayer');
         if (!button) return false;
         button.setAttribute('data-e2e-password-retry', '');
         return true;`,
      );
      assert.ok(retryMarked, 'le bouton de soumission du dialogue est introuvable');
      await (await s.find('[data-e2e-password-retry]')).click();

      await s.waitUntil('le patch est installé après la soumission du mot de passe', async () =>
        (await bodyText(app)).includes(`${GAME_NAME} — patch en ligne installé (1 fichier(s)).`),
        20_000,
      );
    },
  },
});

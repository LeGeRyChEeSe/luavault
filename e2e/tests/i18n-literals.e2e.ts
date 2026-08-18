//! English-only regression checks for strings that used to bypass the catalog.
//!
//! This suite deliberately starts in English.  A French default followed by a
//! locale switch would miss text rendered before that switch or mask a failure
//! behind a stale component.

import { defineSuite, assert } from '../suite.ts';
import { entry } from '../fixtures.ts';

type App = import('../harness.ts').App;

const GAME = entry('620', 'Portal 2', {
  icon: 'https://cdn.akamai.steamstatic.com/steam/apps/620/header.jpg',
});
// Kept in the index but deliberately absent from `lua`: this is the real
// no_lua state where GameActions.getLua() reports its local-import guidance.
const MISSING_LUA_GAME = entry('621', 'Missing Lua');

async function bodyText(app: App): Promise<string> {
  // WebDriver's element-text endpoint drops overflow-clipped descendants.
  // The gallery heading starts below the fold in the compact test window, so
  // read the body's rendered text directly instead of letting viewport height
  // decide which user-visible strings are audited.
  return app.session.execute<string>('return document.body.innerText;');
}

function hasFrenchScreenshotsHeading(text: string): boolean {
  return /captures d['’]écran/i.test(text);
}

function hasEnglishScreenshotsHeading(text: string): boolean {
  return /screenshots/i.test(text);
}

async function clickText(app: App, text: string): Promise<void> {
  const tagged = await app.session.execute<boolean>(
    `const button = [...document.querySelectorAll('button')]
       .find((item) => item.textContent.trim() === arguments[0]);
     if (!button) return false;
     button.setAttribute('data-e2e-i18n-literal', '');
     return true;`,
    [text],
  );
  assert.ok(tagged, `bouton « ${text} » introuvable`);
  await (await app.session.find('[data-e2e-i18n-literal]')).click();
}

async function openSpotlightFor(app: App, name: string): Promise<void> {
  const tagged = await app.session.execute<boolean>(
    `const card = [...document.querySelectorAll('article.lift-card')]
       .find((item) => item.querySelector('h3')?.textContent.trim() === arguments[0]);
     const button = card?.querySelector('button');
     if (!button) return false;
     button.setAttribute('data-e2e-i18n-spotlight', '');
     return true;`,
    [name],
  );
  assert.ok(tagged, `carte « ${name} » introuvable`);
  await (await app.session.find('[data-e2e-i18n-spotlight]')).click();
}

export default defineSuite({
  name: 'i18n-literals',
  seed: {
    index: [GAME, MISSING_LUA_GAME],
    lua: { '620': 'addappid(620)\n' },
    config: { locale: 'en' },
  },
  cases: {
    "la galerie de captures est en anglais dès le démarrage": async (app) => {
      const s = app.session;
      // Prime the real backend cache explicitly. The screen performs this
      // request too, but asserting the response here keeps the visual case
      // about its heading rather than about an external Steam timeout.
      const details = await s.invoke<{ screenshots?: unknown[] }>('get_steam_details', {
        appId: '620',
        lang: 'english',
      });
      assert.ok(Array.isArray(details.screenshots) && details.screenshots.length > 0,
        `Steam doit fournir au moins une capture, reçu ${JSON.stringify(details)}`);
      await (await s.waitFor('nav button[data-tip="Library"]')).click();
      await openSpotlightFor(app, 'Portal 2');
      await s.waitUntil('les captures Steam arrivent dans la fiche', async () =>
        hasFrenchScreenshotsHeading(await bodyText(app)) || hasEnglishScreenshotsHeading(await bodyText(app)),
        30_000,
      );
      assert.ok(!hasFrenchScreenshotsHeading(await bodyText(app)), "le titre français des captures est encore affiché");
    },

    "Steam & Tools ne laisse pas le libellé du dossier en français": async (app) => {
      const s = app.session;
      await (await s.waitFor('button[aria-label="Close"]')).click();
      await (await s.waitFor('nav button[data-tip="Steam & Tools"]')).click();
      await s.waitUntil('le contrôle SteamTools est rendu', async () =>
        (await bodyText(app)).includes('dossier config\\lua') || (await bodyText(app)).includes('config\\lua folder'),
      );
      assert.excludes(await bodyText(app), 'dossier config\\lua');
    },

  },
});

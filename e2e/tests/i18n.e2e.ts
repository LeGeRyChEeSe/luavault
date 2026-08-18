//! The i18n suite — proves that changing language is a reactive swap, not a
//! page reload, and that the choice survives to disk.
//!
//! A unit test with `startViewTransition` stubbed only ever proved the stub's
//! fidelity. This project's real appearance defect — the screen showed the new
//! theme while `config.json` received the old one — was settled by this bench
//! and nothing else, because it drives the real WebView2 and reads the real
//! file. When correctness hinges on a browser API's scheduling, only the real
//! browser answers.
//!
//! The two cases share one app on purpose (`run.ts` launches once per suite),
//! so the second starts from whatever the first left behind and resets itself.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { defineSuite, assert } from '../suite.ts';
import { Key } from '../webdriver.ts';
import { SAMPLE, SAMPLE_LUA } from '../fixtures.ts';

type App = import('../harness.ts').App;

/// Le placeholder est traduit : les deux orthographes sont acceptées, comme pour les
/// entrées de la barre latérale. Le cas bascule en anglais entre deux usages du sélecteur.
const FILTER =
  'input[placeholder="Filtrer par nom ou AppID…"], input[placeholder="Filter by name or AppID…"]';
const CARD = 'article.lift-card';
/// The sidebar entries are translated, so every selector that targets one has to
/// accept both spellings — including the one used to reach Settings in order to
/// switch back.
const SETTINGS = 'nav button[data-tip="Réglages"], nav button[data-tip="Settings"]';
const LIBRARY = 'nav button[data-tip="Bibliothèque"], nav button[data-tip="Library"]';

async function shownNames(app: App): Promise<string[]> {
  return app.session.execute<string[]>(
    `return [...document.querySelectorAll('${CARD} h3')].map(h => h.textContent.trim())`,
  );
}

/// The language the *screen* is in, read from the sidebar rather than from any
/// state we control. `null` when neither label is present — a failure worth
/// seeing rather than a silent `false`.
async function shownLocale(app: App): Promise<string | null> {
  return app.session.execute<string | null>(
    `if (document.querySelector('nav button[data-tip="Library"]')) return 'en';
     if (document.querySelector('nav button[data-tip="Bibliothèque"]')) return 'fr';
     return null;`,
  );
}

/// Bring the language section into view. Found by its **buttons**, never by its
/// heading: the heading is itself translated ("Langue" / "Language"), so an
/// anchor on it works in French and silently fails in English — which is the
/// exact state the second case starts in.
async function scrollToLanguageSection(app: App): Promise<void> {
  const found = await app.session.execute<boolean>(
    `const btn = [...document.querySelectorAll('section button')]
       .find(b => b.textContent.trim() === 'Français' || b.textContent.trim() === 'English');
     if (!btn) return false;
     btn.closest('section').scrollIntoView({ block: 'center' });
     return true;`,
  );
  if (!found) throw new Error('section Langue introuvable dans Réglages');
}

/// Click a language button by its visible label. The labels are endonyms, so
/// they read the same whatever the current locale is — which is what makes them
/// usable as the anchor above.
async function clickLanguage(app: App, label: string): Promise<void> {
  const clicked = await app.session.execute<boolean>(
    `const btn = [...document.querySelectorAll('button')].find(b => b.textContent.trim() === '${label}');
     if (!btn) return false;
     btn.click();
     return true;`,
  );
  if (!clicked) throw new Error(`bouton « ${label} » introuvable dans le sélecteur de langue`);
}

async function switchTo(app: App, label: string, expected: string): Promise<void> {
  const s = app.session;
  await (await s.waitFor(SETTINGS)).click();
  await scrollToLanguageSection(app);
  await clickLanguage(app, label);
  await s.waitUntil(`la barre latérale passe en « ${expected} »`, async () =>
    (await shownLocale(app)) === expected,
  );
}

/// What the sandbox's real config.json carries. Re-read on every call: the write
/// is asynchronous, so a single read right after the click loses the race.
function savedLocale(app: App): string | undefined {
  try {
    const raw = readFileSync(join(app.sandbox, 'config.json'), 'utf8');
    return (JSON.parse(raw) as { locale?: string }).locale;
  } catch {
    return undefined;
  }
}

export default defineSuite({
  name: 'i18n',
  seed: { index: SAMPLE, lua: SAMPLE_LUA },
  cases: {
    'changer de langue ne recharge pas : le filtre saisi avant survit': async (app) => {
      const s = app.session;

      // 1. Library, with a filter fragment that keeps exactly one game.
      await (await s.waitFor(LIBRARY)).click();
      const input = await s.waitFor(FILTER);
      await input.click();
      // `clear()` does not fire the `input` event, so Svelte's `bind:value`
      // never sees it: the field looks empty while the state keeps the old text.
      // Clear it the way a user does.
      await s.chord([Key.Control, 'a']);
      await s.keys([Key.Backspace]);
      await input.sendKeys('Witcher');
      await s.waitUntil("le filtre ne retient plus qu'un jeu", async () =>
        (await shownNames(app)).length === 1,
      );
      assert.equal((await shownNames(app))[0], 'The Witcher 3');

      // 2. Switch to English from Settings. That the sidebar follows is already
      //    proof the swap reached a surface other than the one clicked.
      await switchTo(app, 'English', 'en');

      // 3. Back to Library — reached through the English label, which nothing
      //    but a live re-render could have produced.
      await (await s.waitFor(LIBRARY)).click();

      // 4. THE assertion of this case. Without it a `window.location.reload()`
      //    passes everything above, and we ship a reload believing we shipped a
      //    reactive swap. The field must still hold the text typed BEFORE the
      //    switch, and the grid must still be filtered by it.
      //    The element handle from step 1 is re-found rather than reused: the
      //    re-render replaces the node, and a stale handle fails for a reason
      //    that has nothing to do with what is being tested.
      const field = await s.waitFor(FILTER);
      const value = await field.property('value');
      assert.equal(
        value,
        'Witcher',
        `le filtre doit avoir survécu au changement de langue, trouvé « ${value} »`,
      );
      await s.waitUntil('la grille est toujours filtrée', async () =>
        (await shownNames(app)).length === 1,
      );
      assert.equal((await shownNames(app))[0], 'The Witcher 3');
    },

    "le choix tient sur le disque, et le disque dit ce que montre l'écran": async (app) => {
      // The first case left the app in English; come back to French so the
      // switch under test is a real transition and not a no-op.
      await switchTo(app, 'Français', 'fr');
      await switchTo(app, 'English', 'en');

      await app.session.waitUntil('config.json enregistre une langue', async () =>
        savedLocale(app) !== undefined,
      );

      // The property that matters is not "the file says en" — it is that the
      // file and the screen agree. The appearance bug this bench exists for
      // showed exactly here: the window read one value while config.json held
      // the previous one, and every screen-only assertion stayed green.
      const onScreen = await shownLocale(app);
      assert.equal(
        savedLocale(app),
        onScreen,
        `l'écran est en « ${onScreen} » : config.json doit porter la même chose, pas « ${savedLocale(app)} »`,
      );
      assert.equal(onScreen, 'en', "le basculement demandé était vers l'anglais");
    },
  },
});

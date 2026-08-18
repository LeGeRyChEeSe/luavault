//! The onboarding window — the second screen a new user meets, and the other
//! one nothing exercised.
//!
//! `harness.ts` seeds `first_run_done: true` for every other suite precisely so
//! this screen stays out of the way; that convenience is also what kept it
//! untested. Here we seed it `false` on purpose.
//!
//! It carries the discretion notice the user asked for on 2026-08-03 — the one
//! that says never to mention the application in a public space. Verified once
//! by eye, never since.

import { defineSuite, assert } from '../suite.ts';
import { Key } from '../webdriver.ts';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

export default defineSuite({
  name: 'onboarding',
  // Licence present (the gate wins over onboarding in App.svelte), first run not done.
  seed: { config: { first_run_done: false } },
  cases: {
    "au premier lancement, l'accueil tient, et le focus ne peut pas en sortir": async (app) => {
      const s = app.session;
      await s.waitUntil("l'accueil est rendu", async () =>
        (await (await s.find('body')).text()).includes('Bienvenue dans LuaVault'),
      );
      // Like the licence gate, it is an overlay over a shell that stays mounted,
      // so "the nav is gone" is not the property to check — the focus trap is.
      // Without it, Tab reaches the sidebar behind and the user lands in the app
      // without ever having read the notice this screen exists for.
      await s.waitUntil("le focus entre dans l'accueil", async () =>
        s.execute<boolean>(
          `return document.querySelector('[role="dialog"][aria-modal="true"]').contains(document.activeElement)`,
        ),
      );
      for (let i = 0; i < 8; i++) {
        await s.keys([Key.Tab]);
        const inside = await s.execute<boolean>(
          `const d = document.querySelector('[role="dialog"][aria-modal="true"]');
           return d ? d.contains(document.activeElement) : false`,
        );
        assert.ok(inside, `le focus est sorti de l'accueil apres ${i + 1} Tab`);
      }
    },

    "l'accueil se franchit, et il ne revient pas": async (app) => {
      const s = app.session;
      // Whatever the final control is called, it is the last button of the
      // screen; find it by role rather than by a label that may be reworded.
      const clicked = await s.execute<boolean>(`
        const buttons = Array.from(document.querySelectorAll('button'));
        const go = buttons[buttons.length - 1];
        if (!go) return false;
        go.click();
        return true;
      `);
      assert.ok(clicked, 'un bouton doit permettre de franchir l’accueil');

      // NOT `nav button > 0`: the sidebar is mounted behind the overlay from the
      // first frame, so that assertion is true before the click and proves
      // nothing (class of defect #2 — true by construction). What must change is
      // the overlay going away.
      await s.waitUntil("l'accueil disparaît", async () =>
        (await s.maybeFind('[role="dialog"][aria-modal="true"]')) === null,
      );

      // And it must have been recorded: an onboarding that reappears at every
      // launch is exactly what `first_run_done` exists to prevent — the same
      // flag LOT-22 nearly lost on import.
      const body = await (await s.find('body')).text();
      assert.excludes(body, 'Bienvenue dans LuaVault');

      const raw = readFileSync(join(app.sandbox, 'config.json'), 'utf-8');
      const persisted = JSON.parse(raw);
      assert.equal(
        persisted.first_run_done,
        true,
        'first_run_done doit être écrit sur le disque, pas seulement en mémoire',
      );
    },
  },
});

//! Background notifications on the real window — what the bench CAN settle.
//!
//! The bench runs the app **portable**, and portable mode is exactly where the
//! feature must stay off: the folder may move, and the logon entry and the
//! scheduled task would point at nothing. So this suite pins the refusal the
//! product makes there: no offer, an honest line in Settings, no switch, nothing
//! written. It cannot open the offer itself — that needs an installed copy — and
//! it must never press « Activer »: that writes the real `HKCU` and the real task
//! scheduler of whoever runs the bench, pointing at a sandbox executable about to
//! be deleted (the sandbox isolates files, not the registry).

import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { defineSuite, assert } from '../suite.ts';

const saved = (sandbox: string): boolean | null | undefined => {
  try {
    const raw = readFileSync(join(sandbox, 'config.json'), 'utf8');
    return (JSON.parse(raw) as { background_notify?: boolean | null }).background_notify;
  } catch {
    return undefined;
  }
};

export default defineSuite({
  name: 'notify-offer',
  // The default seed answers the offer (`false`); this suite is the one that never answered.
  seed: { config: { background_notify: null } },
  cases: {
    "en mode portable, l'offre n'est jamais proposée": async (app) => {
      const s = app.session;
      // Settings asks the same backend question as the startup check: once its
      // section is drawn, the answer is in and the offer would already be up.
      await (await s.waitFor('nav button[data-tip="Réglages"]', 20_000)).click();
      await s.waitFor('[data-settings-notify]', 20_000);
      assert.equal(await s.maybeFind('[data-notify-offer]'), null, 'portable : pas d\'offre');
    },

    "Réglages le dit, sans interrupteur": async (app) => {
      const s = app.session;
      const section = await s.waitFor('[data-settings-notify]');
      assert.includes(await section.text(), 'Indisponible en mode portable', 'la raison est affichée');
      assert.equal(await s.maybeFind('[data-settings-notify-switch]'), null, 'et rien à cocher');
    },

    "rien n'est écrit : le choix n'a jamais été fait": async (app) => {
      const v = saved(app.sandbox);
      assert.ok(v === null || v === undefined, `background_notify doit rester vide, lu : ${String(v)}`);
    },
  },
});

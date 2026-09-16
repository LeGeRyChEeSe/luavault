//! The property that matters most about the message of the day, checked on
//! the real binary: a document the primary key did not sign is NOT shown —
//! no popup, no sidebar button — however well-formed it is.
//!
//! This is the counterpart of `motd.e2e.ts`: without it, a build that skipped
//! signature verification would pass that suite just as happily.

import { defineSuite, assert } from '../suite.ts';

const MOTD = {
  schema: 1,
  message: {
    id: 'bench-motd-forged',
    published_at: '2026-09-16T10:00:00Z',
    expires_at: new Date(Date.now() + 60 * 60 * 1000).toISOString(),
    severity: 'critical',
    title: { fr: 'Message forgé' },
    body: { fr: 'Ce texte ne doit jamais apparaître.' },
  },
};

export default defineSuite({
  name: 'motd-tamper',
  seed: { update: { motd: MOTD, motdBadSignature: true } },
  cases: {
    "un message dont la signature ne correspond pas n'est jamais affiché": async (app) => {
      const s = app.session;
      await s.waitUntil("la bibliothèque est rendue", async () =>
        (await (await s.find('body')).text()).includes('Aucun fichier .lua pour le moment.'),
      );
      // The client did ask — so silence below is a decision, not an absence
      // of network. Both files are requested together (`tokio::join!`).
      await s.waitUntil('le client a demandé motd.json et sa signature', async () =>
        app.updateStub.calls.includes('/motd.json') && app.updateStub.calls.includes('/motd.json.sig'),
      );
      // Give the verification every chance to (wrongly) succeed before judging.
      await new Promise((r) => setTimeout(r, 1500));
      const shape = await s.execute<{ dialog: boolean; button: boolean }>(
        `return {
           dialog: document.querySelector('[data-motd-dialog]') !== null,
           button: document.querySelector('aside button[data-motd-open]') !== null,
         }`,
      );
      assert.equal(shape.dialog, false, 'aucune fenêtre pour un message non signé par la clé');
      assert.equal(shape.button, false, 'aucun bouton de rappel non plus : le message n\'existe pas');
      assert.excludes(await (await s.find('body')).text(), 'Message forgé');
    },
  },
});

//! The property the whole suite rests on: the app under test writes into the
//! throwaway sandbox and reads a fake Steam — never the real installation.
//!
//! This suite exists because an adversarial review proved it was missing.
//! It pointed `steam_dir` outside the sandbox and **every case stayed green**:
//! no test read `app.sandbox` or `app.steamDir`, so the isolation was held by
//! construction alone. Deleting one line of `harness.ts` would have sent the
//! suite at the user's real Steam install, silently.
//!
//! These cases ask the backend where it actually is, through the same
//! `detect_all` command the UI uses.

import { defineSuite, assert } from '../suite.ts';

interface DetectionReport {
  portable: boolean;
  data_dir: string;
  library_dir: string;
  steam: { path: string } | null;
}

/// Windows paths compare case-insensitively, and the backend may hand back a
/// different separator or capitalisation than Node produced.
function normalise(p: string): string {
  return p.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
}

function isInside(child: string, parent: string): boolean {
  const c = normalise(child);
  const p = normalise(parent);
  return c === p || c.startsWith(p + '\\');
}

export default defineSuite({
  name: 'isolation',
  cases: {
    "l'application tourne en mode portable dans le bac à sable": async (app) => {
      const report = await app.session.invoke<DetectionReport>('detect_all');
      assert.ok(
        typeof report === 'object' && report !== null,
        `detect_all n'a pas répondu un rapport : ${JSON.stringify(report)}`,
      );
      assert.equal(report.portable, true, 'le mode portable doit être actif');
      assert.ok(
        isInside(report.data_dir, app.sandbox),
        `data_dir est hors du bac à sable : ${report.data_dir} (attendu sous ${app.sandbox})`,
      );
    },

    'la bibliothèque utilisée est celle du bac à sable, pas celle de la machine': async (app) => {
      const report = await app.session.invoke<DetectionReport>('detect_all');
      assert.ok(
        isInside(report.library_dir, app.sandbox),
        `library_dir est hors du bac à sable : ${report.library_dir}`,
      );
      // Belt and braces: the real install must not be what we are driving.
      const real = `${process.env.LOCALAPPDATA}\\LuaVault`;
      assert.ok(
        !isInside(report.library_dir, real),
        `library_dir pointe sur la vraie installation : ${report.library_dir}`,
      );
    },

    'le dossier Steam vu par le backend est le faux, pas le vrai': async (app) => {
      const report = await app.session.invoke<DetectionReport>('detect_all');
      assert.ok(report.steam !== null, 'aucun Steam détecté — le faux devrait être trouvé');
      assert.ok(
        isInside(report.steam!.path, app.steamDir),
        `Steam pointe hors du bac à sable : ${report.steam!.path} (attendu ${app.steamDir})`,
      );
      // `commands::resolve_steam` falls back to `detect::detect_steam_path`
      // when `steam_dir` is unset — that fallback finds the user's real Steam.
      // This is the assertion that catches a dropped seed.
      assert.ok(
        isInside(report.steam!.path, app.sandbox),
        `Steam est hors du bac à sable : ${report.steam!.path}`,
      );
    },
  },
});

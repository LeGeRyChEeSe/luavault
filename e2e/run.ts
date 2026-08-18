//! The graphical end-to-end runner.
//!
//!   npm run e2e             — every suite
//!   npm run e2e -- nav      — only the suites whose name contains "nav"
//!
//! A failing case writes a PNG of the actual window to `.e2e/failures/`. For
//! eight lots in a row this project ended on "rendu visuel non vérifié"; a
//! screenshot of the real screen at the moment of failure is the point.

import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { App, ROOT, checkPrerequisites } from './harness.ts';
import type { Suite } from './suite.ts';

// Suites are listed, not globbed: the order is deterministic and a file that
// fails to import is a loud error rather than a silently skipped suite.
import isolation from './tests/isolation.e2e.ts';
import shell from './tests/shell.e2e.ts';
import library from './tests/library.e2e.ts';
import shortcuts from './tests/shortcuts.e2e.ts';
import integrity from './tests/integrity.e2e.ts';
import onboarding from './tests/onboarding.e2e.ts';
import selection from './tests/selection.e2e.ts';
import virtual from './tests/virtual.e2e.ts';
import tooltip from './tests/tooltip.e2e.ts';
import i18n from './tests/i18n.e2e.ts';
import i18nLiterals from './tests/i18n-literals.e2e.ts';
import passwordFix from './tests/password-fix.e2e.ts';

// `isolation` runs first on purpose: if the app under test is not the sandboxed
// one, every result that follows is meaningless — better to fail on the first
// suite than to report 20 green cases obtained against the user's real install.
const SUITES: Suite[] = [isolation, shell, library, shortcuts, integrity, onboarding, selection, virtual, tooltip, i18n, i18nLiterals, passwordFix];

const filter = process.argv.slice(2).filter((a) => !a.startsWith('-'));
const keepSandbox = process.argv.includes('--keep');

const problems = checkPrerequisites();
if (problems.length) {
  console.error('\nPrérequis manquants :\n  - ' + problems.join('\n  - ') + '\n');
  process.exit(2);
}

const selected = filter.length
  ? SUITES.filter((s) => filter.some((f) => s.name.includes(f)))
  : SUITES;

if (!selected.length) {
  console.error(`Aucune suite ne correspond à : ${filter.join(', ')}`);
  process.exit(2);
}

const failuresDir = join(ROOT, '.e2e', 'failures');
let passed = 0;
const failures: { suite: string; case: string; error: string; shot?: string }[] = [];

for (const suite of selected) {
  console.log(`\n${suite.name}`);
  let app: App;
  try {
    app = await App.launch(suite.name, suite.seed);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.log(`  ✗ [démarrage] ${msg}`);
    failures.push({ suite: suite.name, case: '[démarrage]', error: msg });
    continue;
  }

  try {
    for (const [name, run] of Object.entries(suite.cases)) {
      const started = Date.now();
      try {
        await run(app);
        console.log(`  ✓ ${name} (${Date.now() - started} ms)`);
        passed++;
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        console.log(`  ✗ ${name}`);
        console.log(`      ${msg.split('\n').join('\n      ')}`);
        let shot: string | undefined;
        try {
          mkdirSync(failuresDir, { recursive: true });
          shot = join(failuresDir, `${suite.name}-${name.replace(/[^\w]+/g, '_').slice(0, 60)}.png`);
          writeFileSync(shot, await app.session.screenshot());
          console.log(`      capture : ${shot}`);
        } catch {
          // A window already gone cannot be photographed; the message stands alone.
        }
        failures.push({ suite: suite.name, case: name, error: msg, shot });
      }
    }
  } finally {
    await app.close(keepSandbox);
  }
}

console.log('');
if (failures.length) {
  console.log(`${passed} test(s) au vert, ${failures.length} en échec :`);
  for (const f of failures) console.log(`  - ${f.suite} › ${f.case}`);
  process.exit(1);
}
console.log(`Les ${passed} tests graphiques passent.`);

//! Feasibility probe, kept in the tree: does the toolchain actually drive the
//! real window? Run it when the E2E suite fails in a way that smells like the
//! driver rather than the app (`npm run e2e:smoke`).

import { App, checkPrerequisites } from './harness.ts';

const problems = checkPrerequisites();
if (problems.length) {
  console.error('Prérequis manquants :\n  - ' + problems.join('\n  - '));
  process.exit(2);
}

const app = await App.launch('smoke');
try {
  const title = await app.session.execute<string>('return document.title');
  const url = await app.session.execute<string>('return location.href');
  const body = await app.session.waitFor('body');
  const text = (await body.text()).slice(0, 400);
  console.log('titre   :', title);
  console.log('url     :', url);
  console.log('sandbox :', app.sandbox);
  console.log('--- texte visible ---');
  console.log(text);
} finally {
  await app.close();
}

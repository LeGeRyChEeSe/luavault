//! The tooltip layer (`lib/tooltip.ts`) — the last of the units the harness
//! could not reach, and the one that never needed a fake DOM: it needs a real
//! one, which the bench has.
//!
//! Its whole reason for existing is pitfall 19: a `::after` bubble was being
//! clipped by every panel with `overflow: hidden`, so the tooltip is drawn as a
//! single element parented to `<body>` and positioned with fixed coordinates.
//! That parentage IS the fix — checking only that "a bubble appears" would pass
//! just as happily on the CSS version that had the bug.

import { defineSuite, assert } from '../suite.ts';
import { SAMPLE, SAMPLE_LUA } from '../fixtures.ts';

const BUBBLE = '#lv-tooltip';

export default defineSuite({
  name: 'tooltip',
  seed: { index: SAMPLE, lua: SAMPLE_LUA },
  cases: {
    'survoler un élément à data-tip affiche sa bulle, avec son texte': async (app) => {
      const s = app.session;
      const target = await s.waitFor('nav button[data-tip="Bibliothèque"]');
      await s.hover(target);

      await s.waitUntil('la bulle devient visible', async () =>
        s.execute<boolean>(
          `const b = document.querySelector('${BUBBLE}');
           return !!b && getComputedStyle(b).opacity === '1'`,
        ),
      );
      const text = await s.execute<string | null>(
        `const b = document.querySelector('${BUBBLE}');
         return b ? b.textContent.trim() : null`,
      );
      assert.equal(text, 'Bibliothèque', 'la bulle doit porter le texte du data-tip survolé');
    },

    'la bulle est parentée au body, pas au panneau survolé': async (app) => {
      const s = app.session;
      // This is the assertion that encodes pitfall 19. A bubble nested inside
      // the sidebar would be clipped by its `overflow`, which is exactly the bug
      // this layer replaced — and a test that only looked for a visible bubble
      // would not have noticed.
      const parent = await s.execute<string | null>(
        `const b = document.querySelector('${BUBBLE}');
         return b && b.parentElement ? b.parentElement.tagName : null`,
      );
      assert.equal(parent, 'BODY', 'la bulle doit être un enfant direct de <body>');

      const fixed = await s.execute<string | null>(
        `const b = document.querySelector('${BUBBLE}');
         return b ? getComputedStyle(b).position : null`,
      );
      assert.equal(fixed, 'fixed', 'positionnement fixe : sinon elle suit le défilement du panneau');

      // And above every overlay the app can raise, or it would hide behind a modal.
      const z = await s.execute<number>(
        `return Number(getComputedStyle(document.querySelector('${BUBBLE}')).zIndex)`,
      );
      assert.ok(z > 1000, `z-index trop bas (${z}) — la bulle passerait derrière une modale`);
    },

    'la bulle est réellement dans la fenêtre, pas rognée hors écran': async (app) => {
      const s = app.session;
      const box = await s.execute<{ top: number; left: number; w: number; h: number }>(
        `const r = document.querySelector('${BUBBLE}').getBoundingClientRect();
         return { top: r.top, left: r.left, w: r.width, h: r.height }`,
      );
      assert.ok(box.w > 0 && box.h > 0, 'la bulle doit avoir une taille');
      assert.ok(box.top >= 0, `la bulle sort par le haut (top=${box.top})`);
      assert.ok(box.left >= 0, `la bulle sort par la gauche (left=${box.left})`);
      const within = await s.execute<boolean>(
        `const r = document.querySelector('${BUBBLE}').getBoundingClientRect();
         return r.right <= window.innerWidth && r.bottom <= window.innerHeight`,
      );
      assert.ok(within, 'la bulle doit tenir dans la fenêtre — elle bascule du côté où il y a la place');
    },

    'quitter l’élément la fait disparaître': async (app) => {
      const s = app.session;
      // Move the pointer somewhere with no `data-tip` under it.
      await s.pointerTo(2, 2);
      await s.waitUntil('la bulle se cache', async () =>
        s.execute<boolean>(
          `const b = document.querySelector('${BUBBLE}');
           return !b || getComputedStyle(b).opacity === '0'`,
        ),
      );
    },
  },
});

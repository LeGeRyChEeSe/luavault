/**
 * `$state` for plain Node, installed before anything else is imported.
 *
 * Runes are a compiler macro: in a `.svelte.ts` file `$state(v)` is rewritten
 * at build time and never exists at runtime. Under `tsx` there is no compiler,
 * so the identifier has to be a real global or the module dies on import with
 * `$state is not defined`.
 *
 * This lives in its own module, imported **first** by the runner, because of a
 * failure that cost a whole round: each suite used to install its own shim just
 * before its dynamic import, which works only as long as no *earlier* import
 * reaches a `.svelte.ts`. The moment `stages.ts` started importing the i18n
 * store, `library-sort.ts` — imported near the top of the runner — pulled the
 * store in at line 296, hundreds of lines before the first suite installed a
 * shim. The agent that hit it concluded `$state` "does not work in .ts files"
 * and looped on that false premise; the real answer is that a global concern
 * cannot be initialised by whichever suite happens to need it first.
 *
 * Static imports are hoisted and evaluated in source order, so importing this
 * module on the first line is what makes the guarantee hold.
 */
(globalThis as Record<string, unknown>).$state ??= (v: unknown) => v;

export const __runeShimInstalled = true;

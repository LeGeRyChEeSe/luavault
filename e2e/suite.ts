//! Suite definition and assertions for the graphical end-to-end tests.
//!
//! Same shape as `scripts/test-virtual-scroll.ts`: no assertion library, a
//! throw is a failure, and the process exit code is what `validate.ps1` reads.
//!
//! A suite launches the app **once** and runs its cases against that one
//! window. Starting the binary costs a few seconds, so cases inside a suite
//! share it — which means a case must leave the app in a usable state for the
//! next one. Anything needing a different seed gets its own suite.

import type { App, Seed } from './harness.ts';

export interface Suite {
  name: string;
  /// What the sandbox holds before the app starts.
  seed?: Seed;
  cases: Record<string, (app: App) => Promise<void>>;
}

export function defineSuite(suite: Suite): Suite {
  return suite;
}

export const assert = {
  ok(cond: unknown, msg?: string): void {
    if (!cond) throw new Error(msg ?? 'assertion échouée');
  },
  equal(actual: unknown, expected: unknown, msg?: string): void {
    if (actual !== expected) {
      const where = msg ? `${msg} — ` : '';
      throw new Error(`${where}attendu ${JSON.stringify(expected)}, obtenu ${JSON.stringify(actual)}`);
    }
  },
  notEqual(actual: unknown, expected: unknown, msg?: string): void {
    if (actual === expected) {
      const where = msg ? `${msg} — ` : '';
      throw new Error(`${where}les deux valeurs sont ${JSON.stringify(actual)}, elles devaient différer`);
    }
  },
  includes(haystack: string, needle: string, msg?: string): void {
    if (!haystack.includes(needle)) {
      const where = msg ? `${msg} — ` : '';
      throw new Error(`${where}« ${needle} » absent de : ${haystack.slice(0, 400)}`);
    }
  },
  excludes(haystack: string, needle: string, msg?: string): void {
    if (haystack.includes(needle)) {
      const where = msg ? `${msg} — ` : '';
      throw new Error(`${where}« ${needle} » présent alors qu'il ne devait pas l'être`);
    }
  },
};

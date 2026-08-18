/** Behavioural test for the app-store fallback when the Tauri command fails. */
import "./rune-shim";

const g = globalThis as Record<string, unknown>;
const previousInternals = g.__TAURI_INTERNALS__;
g.__TAURI_INTERNALS__ = {
  invoke: (command: string) =>
    command === "get_reachability"
      ? Promise.reject(new Error("reachability command timed out"))
      : Promise.resolve(null),
  transformCallback: (callback: unknown) => callback,
};

const assert = {
  equal(actual: unknown, expected: unknown, message: string): void {
    if (actual !== expected) throw new Error(`${message} — expected ${expected}, got ${actual}`);
  },
};

let cases = 0;
export const reachabilityStateCases = () => cases;
export const __reachabilityStateRan = true;

export const reachabilityStateSuite = (async () => {
  const { appState } = await import("../src/lib/app-state.svelte");
  appState.setReachability({
    online: true,
    consecutive_failures: 0,
    last_failure_secs_ago: null,
    tip: null,
  });

  await appState.refreshReachability();
  assert.equal(appState.online, false, "an invoke rejection must switch the store offline");
  assert.equal(appState.offlineTip, null, "the command failure must not retain a stale tip");
  cases++;
  console.log("  ✓ échec de get_reachability → appState.online=false");

  g.__TAURI_INTERNALS__ = previousInternals;
})();

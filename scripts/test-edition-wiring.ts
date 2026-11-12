// @ts-expect-error Node types are deliberately not a frontend dependency.
import assert from "node:assert/strict";
// @ts-expect-error Node types are deliberately not a frontend dependency.
import { readFileSync } from "node:fs";
import { EDITION, editionFrom, editionsAgree, isExperimental, showChannelChoice } from "../src/lib/edition";

// The two editions: what the bundle says it is, and when Settings offers the channel choice.
let cases = 0;

{
  assert.equal(editionFrom("experimental"), "experimental");
  for (const raw of ["stable", "", undefined, null, "Experimental", "beta", 1]) {
    assert.equal(editionFrom(raw), "stable", `only the exact name is experimental (${String(raw)})`);
  }
  cases++;
}

{
  assert.ok(editionsAgree("stable", "stable"));
  assert.ok(editionsAgree("experimental", "experimental"));
  assert.ok(!editionsAgree("stable", "experimental"));
  assert.ok(!editionsAgree("experimental", "stable"));
  cases++;
}

{
  // Outside Vite the edition reads as stable: the runner never sees a badge.
  assert.equal(EDITION, "stable");
  assert.equal(isExperimental, false);
  cases++;
}

{
  // A stable is offered the choice only while an experimental is published...
  assert.ok(!showChannelChoice({ experimental: false, channel: "stable", offered: false }));
  assert.ok(!showChannelChoice({ experimental: false, channel: null, offered: false }));
  assert.ok(showChannelChoice({ experimental: false, channel: "stable", offered: true }));
  // ...and keeps it on the way back, as the experimental binary does.
  assert.ok(showChannelChoice({ experimental: false, channel: "experimental", offered: false }));
  assert.ok(showChannelChoice({ experimental: true, channel: "stable", offered: false }));
  cases++;
}

{
  // The build sets the edition and Vite freezes it: both ends must spell the same variable.
  const vite = readFileSync("vite.config.js", "utf8");
  const build = readFileSync("scripts/build-exp.ps1", "utf8");
  assert.ok(vite.includes('"import.meta.env.VITE_LV_EDITION"'), "vite.config.js freezes VITE_LV_EDITION");
  assert.ok(build.includes("VITE_LV_EDITION"), "build-exp.ps1 sets VITE_LV_EDITION");
  cases++;
}

{
  // A downgrade installs by itself only when it is the way back (`returning`).
  const state = readFileSync("src/lib/app-state.svelte.ts", "utf8");
  assert.ok(
    state.includes("(update.downgrade && !update.returning)"),
    "installAutomatically refuses a downgrade the person did not ask for",
  );
  cases++;
}

export const __editionWiringRan = true;
export const editionWiringSuite = Promise.resolve();
export const editionWiringCases = () => cases;

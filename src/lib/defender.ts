import { confirm } from "@tauri-apps/plugin-dialog";
import {
  defenderStatus,
  installOnlineFix,
  setDefenderChoice,
  setupDefenderExclusions,
  verifyDefenderExclusions,
} from "./api";
import type { FixReport } from "./api";
import { appState } from "./app-state.svelte";
import { t } from "./i18n.svelte";

/**
 * Pre-flight guard for an online-fix install.
 *
 * The exclusion over the Steam games folders is offered once, at startup, and
 * added behind a UAC prompt. Defender's exclusion list can only be read with
 * admin rights, so it can't be re-verified here without elevating on every
 * install — which is exactly what we avoid. We therefore trust the recorded
 * choice and only step in when no choice was ever made (e.g. Steam showed up
 * after the first-startup prompt).
 *
 * Best-effort: any failure falls through to a normal install, never blocks it.
 */
export async function ensureFixExclusions(): Promise<void> {
  try {
    const choice = appState.report?.defender_exclusions;
    if (choice === true || choice === false) return; // already decided — trust it
    if (!appState.report?.steam) return; // no Steam → nothing to exclude

    const status = await defenderStatus();
    if (!status.available) return; // not Defender — nothing to manage

    const ok = await confirm(
      t("shell.defender.body"),
      {
        title: t("shell.defender.title"),
        kind: "warning",
        okLabel: t("shell.defender.ok"),
        cancelLabel: t("shell.defender.cancel"),
      },
    );
    if (ok) {
      await setupDefenderExclusions();
    } else {
      await setDefenderChoice(false);
    }
    await appState.refresh();
  } catch {
    // Defender handling is a convenience — never let it break the install.
  }
}

/**
 * Choose the post-installation recovery from what integrity verification
 * observed. A present-but-modified file is not evidence of deletion, and an
 * inactive Defender cannot make use of an exclusion.
 */
function repairMessageFor(
  report: FixReport,
  defenderActive: boolean,
): "missing" | "passive" | "modified" | null {
  if (report.missing.length > 0) {
    return defenderActive ? "missing" : "passive";
  }
  if (report.modified.length > 0) return "modified";
  return null;
}

/**
 * Offer the recovery that matches an integrity report. Returns true only when
 * the caller should retry the install after the selected recovery action.
 */
async function offerExclusionRepair(report: FixReport): Promise<boolean> {
  try {
    const status = await defenderStatus();
    const message = repairMessageFor(report, status.available && status.active);
    if (!message) return false;

    if (message === "modified") {
      return await confirm(t("shell.defender.modified.body"), {
        title: t("shell.defender.modified.title"),
        kind: "warning",
        okLabel: t("shell.defender.modified.ok"),
        cancelLabel: t("shell.defender.cancel"),
      });
    }

    if (message === "passive") {
      await confirm(t("shell.defender.passive.body"), {
        title: t("shell.defender.passive.title"),
        kind: "warning",
        okLabel: t("shell.defender.passive.ok"),
        cancelLabel: t("shell.defender.cancel"),
      });
      return false;
    }

    if (message === "missing") {
      const ok = await confirm(
        t("shell.defender.repair.body"),
        {
          title: t("shell.defender.repair.title"),
          kind: "warning",
          okLabel: t("shell.defender.repair.ok"),
          cancelLabel: t("shell.defender.cancel"),
        },
      );
      if (!ok) return false;

      await verifyDefenderExclusions();
      await appState.refresh();
      return true;
    }

    return false;
  } catch {
    return false;
  }
}

/**
 * Install an online fix, Defender-aware. Ensures the exclusion is set up, runs
 * the install, and if the result is damaged (or the install fails outright —
 * Defender can strike mid-copy) offers to repair the exclusion and retries once.
 */
export async function installFixWithRepair(
  appId: string,
  password?: string | null,
): Promise<FixReport> {
  await ensureFixExclusions();
  try {
    const report = await installOnlineFix(appId, password ?? undefined);
    if (report.health !== "healthy" && (await offerExclusionRepair(report))) {
      return await installOnlineFix(appId, password ?? undefined);
    }
    return report;
  } catch (error) {
    // An exception carries no FixReport and the existing API exposes no reliable
    // antivirus-specific cause. The former retry assumed Defender deleted files;
    // offering that repair blindly would undo this distinction.
    throw error;
  }
}

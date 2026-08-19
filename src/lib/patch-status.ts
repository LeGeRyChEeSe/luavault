import type { FixHealth } from "./api";

export type InstallFixStatus = {
  fix_downloaded: boolean;
  stage: string;
  fix: { health: FixHealth };
};

/** Whether the secondary "Install patch" action remains useful. */
export function shouldOfferInstallFix(status: InstallFixStatus): boolean {
  return status.fix_downloaded && !status.stage.startsWith("fix_") && status.fix.health !== "healthy";
}

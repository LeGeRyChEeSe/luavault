import type { GameStage } from "./api";

/**
 * Colour vocabulary. Kept deliberately small — a colour only appears when it
 * means something, so "everything is fine" stays visually quiet.
 */
export type Tone = "neutral" | "info" | "action" | "good" | "bad" | "progress";

export const TONE_CLASS: Record<Tone, string> = {
  neutral: "bg-white/60 text-azure-900/60 border-white/70",
  info: "bg-sky-soft/70 text-sky-deep border-sky/25",
  action: "bg-peach-soft/70 text-peach-deep border-peach/30",
  good: "bg-mint-soft/70 text-mint-deep border-mint/30",
  bad: "bg-rose-soft/75 text-rose-deep border-rose/30",
  progress: "bg-lilac-soft/70 text-lilac-deep border-lilac/30",
};

/** Left accent bar on a game card, echoing the badge colour. */
export const TONE_EDGE: Record<Tone, string> = {
  neutral: "bg-azure-200/60",
  info: "bg-sky/60",
  action: "bg-peach/70",
  good: "bg-mint/60",
  bad: "bg-rose/70",
  progress: "bg-lilac/60",
};

export interface StageInfo {
  label: string;
  icon: string;
  tone: Tone;
  /** Shown on hover — explains what the state means and what to do next. */
  tip: string;
  /** True while the state is expected to change on its own. */
  live?: boolean;
}

import { t } from "./i18n.svelte";

/** Visual style only — no text. Labels and tips come from the i18n catalogue. */
export const STAGE_STYLE: Record<GameStage, Omit<StageInfo, "label" | "tip">> = {
  no_lua: {
    icon: "info",
    tone: "neutral",
  },
  lua_not_in_steam: {
    icon: "copy",
    tone: "action",
  },
  needs_steam_install: {
    icon: "steam",
    tone: "action",
  },
  installing: {
    icon: "clock",
    tone: "progress",
    live: true,
  },
  ready: {
    icon: "check",
    tone: "good",
  },
  fix_downloaded: {
    icon: "patch",
    tone: "action",
  },
  fix_installed: {
    icon: "shield",
    tone: "good",
  },
  fix_damaged: {
    icon: "alert",
    tone: "bad",
  },
  fix_external: {
    icon: "shield",
    tone: "info",
  },
  fix_game_moved: {
    icon: "alert",
    tone: "bad",
  },
};

/**
 * Style + text for a stage, composed at call time so a language switch
 * re-renders every badge — the whole point of this file's migration.
 *
 * The unknown branch is a diagnostic, not a translation: an id the backend
 * sends but the frontend does not know has no catalogue entry, so its label
 * stays the raw id (that is what one needs to see in order to fix it) and only
 * the tip is translated.
 */
export function stageInfo(stage: GameStage): StageInfo {
  const style = STAGE_STYLE[stage];
  if (!style) {
    return { label: stage, icon: "info", tone: "neutral", tip: t("stage.unknown.tip") };
  }
  return {
    ...style,
    label: t(`stage.${stage}.label`),
    tip: t(`stage.${stage}.tip`),
  };
}

/** Urgency order for the library sort mode "stage". Most urgent first.
 * Unknown stages are sorted last rather than crashing the sort.
 * States that demand a user action come first, then self-resolving states,
 * then states that look nice.
 */
export const STAGE_ORDER: GameStage[] = [
  "fix_damaged",
  "fix_game_moved",
  "lua_not_in_steam",
  "needs_steam_install",
  "fix_downloaded",
  "no_lua",
  "installing",
  "fix_external",
  "fix_installed",
  "ready",
];

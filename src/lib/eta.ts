/**
 * Remaining-time estimate for a bulk pass (LOT-19, task 4). Pure module —
 * BulkProgress.svelte stamps each progress event with its arrival time and
 * renders the label; the arithmetic and the formatting live here, tested in
 * test-virtual-scroll.ts.
 *
 * The one rule the whole module hangs off: an ETA can only measure time the
 * MACHINE owns. The `games` phase waits on the user validating each install
 * in Steam — five seconds or twenty minutes — so it never enters a sample
 * window, and a pass that mixes games then patches restarts its measurement
 * when the automatic phase begins.
 */

export type EtaPhase = "games" | "fixes" | "repair" | "selection";

export type EtaStatus = "working" | "ok" | "error" | "skipped";

export interface EtaSample {
  phase: EtaPhase;
  app_id: string;
  status: EtaStatus;
  /** The per-phase total carried by the progress events. */
  total: number;
  cancelled: boolean;
  /** Arrival timestamp in ms — any monotone clock, passed in by the caller. */
  at: number;
}

export interface EtaEstimate {
  /** Estimated remaining milliseconds; null when no honest estimate exists. */
  ms: number | null;
  /** Short display label ("< 1 min", "~2 min", …); null when nothing may show. */
  label: string | null;
  /** Unique finished operations in the measured phase. */
  completed: number;
  /** Operations still to come in that phase. */
  remaining: number;
  /** The total carried by the phase's events. */
  total: number;
}

/** The phases whose pace the machine sets — the only ones an ETA can measure. */
export function isAutoPhase(phase: EtaPhase): boolean {
  return phase === "fixes" || phase === "repair" || phase === "selection";
}

/**
 * Estimate the remaining time of the automatic phase currently running.
 *
 * `samples` are the deduplication input: every progress event, each stamped
 * with its arrival time. The active phase is the one of the most recent
 * sample — a games → fixes pass therefore measures fixes only, from its own
 * first event, because the games samples sit in another phase. Duplicates
 * collapse per (phase, app_id), keeping the latest status, so one operation
 * is never counted twice. An error or a skip is finished work and feeds the
 * rate; a `working` status is not.
 *
 * Returns a null label whenever the data cannot support an estimate: no
 * samples, a non-automatic phase, a cancellation, zero finished operations,
 * or a finished phase.
 */
export function estimateEta(samples: EtaSample[], now: number): EtaEstimate {
  const none: EtaEstimate = { ms: null, label: null, completed: 0, remaining: 0, total: 0 };
  if (samples.length === 0) return none;

  const active = samples[samples.length - 1].phase;
  if (!isAutoPhase(active)) return none;

  const phaseSamples = samples.filter((s) => s.phase === active);
  if (phaseSamples.some((s) => s.cancelled)) return none;

  const latest = new Map<string, EtaSample>();
  for (const s of phaseSamples) latest.set(s.app_id, s);
  const completed = [...latest.values()].filter((s) => s.status !== "working").length;
  const total = Math.max(...phaseSamples.map((s) => s.total));

  if (completed === 0 || total <= 0) return { ...none, completed, total };
  const remaining = total - completed;
  if (remaining <= 0) return { ...none, completed, remaining: 0, total };

  const start = Math.min(...phaseSamples.map((s) => s.at));
  const elapsed = now - start;
  if (!Number.isFinite(elapsed) || elapsed <= 0) {
    return { ...none, completed, remaining, total };
  }

  const ms = (remaining * elapsed) / completed;
  return { ms, label: formatEta(ms), completed, remaining, total };
}

/**
 * Honest durations only: under a minute says "< 1 min", minutes are rounded
 * to the unit, and beyond an hour the figure rounds to five-minute steps —
 * the rate of patch installs cannot support second-level precision.
 */
export function formatEta(ms: number): string {
  if (!Number.isFinite(ms) || ms < 60_000) return "< 1 min";
  const minutes = Math.round(ms / 60_000);
  if (minutes < 60) return `~${minutes} min`;
  const rounded = Math.round(minutes / 5) * 5;
  const h = Math.floor(rounded / 60);
  const m = rounded % 60;
  return m === 0 ? `~${h} h` : `~${h} h ${String(m).padStart(2, "0")}`;
}

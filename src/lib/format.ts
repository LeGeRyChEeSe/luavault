import { i18n, t } from "./i18n.svelte";

/**
 * Orders of magnitude, `as const` on purpose: it makes the element type the
 * literal union, so `` t(`format.bytes.${…}`) `` type-checks against the
 * catalogue without a cast. Written first with `as Key`, which compiled and
 * would have crashed at runtime — `t()` looks the key up and interpolates, so a
 * key renamed in the catalogue reaches `.split` on `undefined`. A cast does not
 * silence a warning here, it silences the only thing that was watching.
 */
const BYTE_UNITS = ["b", "kb", "mb", "gb", "tb"] as const;

export function formatBytes(n?: number | null): string {
  if (!n) return "—";
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  return t(`format.bytes.${BYTE_UNITS[unit]}`, {
    n: value.toFixed(value >= 100 || unit === 0 ? 0 : 1),
  });
}

export function formatDate(iso?: string | null): string {
  if (!iso) return "—";
  try {
    return new Date(iso).toLocaleDateString(i18n.locale, {
      day: "2-digit",
      month: "short",
      year: "numeric",
    });
  } catch {
    return iso;
  }
}

/**
 * Steam reports changelog dates in Unix **seconds**, and `new Date` wants
 * **milliseconds** — feeding the seconds raw yields a date in January 1970.
 * This is the only formatter allowed to touch a numeric Steam date.
 */
export function formatUnixDate(seconds?: number | null): string {
  if (!seconds) return "—";
  return new Date(seconds * 1000).toLocaleDateString(i18n.locale, {
    day: "2-digit",
    month: "short",
    year: "numeric",
  });
}

/**
 * LOT-13 — local playtime, in minutes, as Steam records it in the account's
 * `localconfig.vdf`. Rendered in clear French: `3 h 37`, `45 min`,
 * `jamais joué`.
 *
 * `minutes` semantics: `null`/`undefined` = unknown (no readable data) and
 * NEVER rendered as a zero; `0` with no recorded session = "jamais joué";
 * `0` while a genuine last session exists = Steam never wrote the minutes
 * ("< 1 min"). `lastPlayed` (Unix seconds) is only there to keep
 * "jamais joué" honest — a game with a recorded session was played.
 */
export function formatPlaytime(minutes?: number | null, lastPlayed?: number | null): string {
  if (minutes == null) return t("format.playtime.unknown");
  if (minutes === 0) return lastPlayed ? t("format.playtime.under1min") : t("format.playtime.never");
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return t("format.playtime.minutes", { m: rest });
  if (rest === 0) return t("format.playtime.hours", { h: hours });
  return t("format.playtime.hoursMinutes", { h: hours, m: String(rest).padStart(2, "0") });
}

/**
 * The stats section's total. The backend sums only KNOWN playtimes, so a
 * zero total has two meanings the raw number cannot tell apart: every
 * known game was never played ("jamais joué"), or no game has any data at
 * all — the backend refused to count an unknown as zero, and the display
 * must not reintroduce it. `unknownCount === gameCount` is that second
 * case: the total is unknown too.
 */
export function formatTotalPlaytime(
  totalMinutes: number,
  unknownCount: number,
  gameCount: number,
): string {
  if (gameCount > 0 && unknownCount >= gameCount) return t("format.playtime.unknown");
  return formatPlaytime(totalMinutes);
}

export function formatDateTime(iso?: string | null): string {
  if (!iso) return "—";
  try {
    return new Date(iso).toLocaleString(i18n.locale, {
      day: "2-digit",
      month: "short",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return iso;
  }
}

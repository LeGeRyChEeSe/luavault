/**
 * Pure rules of the log view (LOT-18).
 *
 * Lives in a plain `.ts` file — `logs-state.svelte.ts` carries runes and
 * cannot be imported by the tsx test runner; same split as
 * library-sort.ts (LOT-13) and search-logic.ts (LOT-17).
 */

/** Level buckets. `other` = TRACE, DEBUG and anything unknown. */
export type LogLevelFilter = "all" | "error" | "warn" | "info" | "other";

/** The fields the view and its rules read on a log entry. */
export interface LogLike {
  level: string;
  message: string;
  timestamp?: string;
}

/** The view never renders more than this many rows (DOM budget). */
export const LOG_DISPLAY_LIMIT = 200;

/** Rust appends i18n metadata after this non-printable delimiter. */
export const I18N_LOG_SEPARATOR = "\u001f";

export interface I18nLogPayload {
  key: string;
  args: Record<string, unknown>;
}

/**
 * The first segment stays readable in the file log. A malformed payload is
 * deliberately represented as `null`: log ingestion is diagnostic plumbing
 * and must never throw or hide a following entry.
 */
export interface DecodedI18nLogMessage {
  fallback: string;
  payload: I18nLogPayload | null;
}

/** Extract Rust's optional structured suffix without importing Svelte i18n. */
export function decodeI18nLogMessage(message: string): DecodedI18nLogMessage {
  const fallback = message.split(I18N_LOG_SEPARATOR, 1)[0];
  const parts = message.split(I18N_LOG_SEPARATOR);
  if (parts.length !== 3 || parts[1] === "") return { fallback, payload: null };

  try {
    const args: unknown = JSON.parse(parts[2]);
    if (args === null || Array.isArray(args) || typeof args !== "object") {
      return { fallback, payload: null };
    }
    return { fallback, payload: { key: parts[1], args: args as Record<string, unknown> } };
  } catch {
    return { fallback, payload: null };
  }
}

/**
 * Translate only keys a caller has positively recognised. The translator is
 * also contained as a final safeguard: a corrupt future catalogue may never
 * make `attachLogger` throw and stop the webview's log stream.
 */
export function resolveI18nLogMessage<K extends string>(
  message: string,
  isKnownKey: (key: string) => key is K,
  translate: (key: K, args: Record<string, unknown>) => string,
): string {
  const decoded = decodeI18nLogMessage(message);
  if (decoded.payload === null || !isKnownKey(decoded.payload.key)) return decoded.fallback;
  try {
    return translate(decoded.payload.key, decoded.payload.args);
  } catch {
    return decoded.fallback;
  }
}

/**
 * plugin-log hands the webview numeric codes (Trace=1 … Error=5), while
 * the rest of the app speaks names. One place translates, so the level
 * vocabulary is the same everywhere.
 */
export function levelName(level: string): string {
  switch (level) {
    case "1":
      return "TRACE";
    case "2":
      return "DEBUG";
    case "3":
      return "INFO";
    case "4":
      return "WARN";
    case "5":
      return "ERROR";
    default:
      return level.toUpperCase();
  }
}

/** Every level string in one of the four buckets, case-insensitive. */
export function levelBucket(level: string): Exclude<LogLevelFilter, "all"> {
  switch (levelName(level)) {
    case "ERROR":
      return "error";
    case "WARN":
      return "warn";
    case "INFO":
      return "info";
    default:
      return "other";
  }
}

export function matchesLevel(entry: LogLike, filter: LogLevelFilter): boolean {
  return filter === "all" || levelBucket(entry.level) === filter;
}

/** Lowercase, accents stripped — the search is announced as both. */
export function normalizeText(text: string): string {
  return text
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase();
}

/** Full-text match on the message. An empty (or whitespace) query
 * matches everything. */
export function logMatches(entry: LogLike, query: string): boolean {
  const q = normalizeText(query.trim());
  if (q === "") return true;
  return normalizeText(entry.message).includes(q);
}

/** Level filter and search combined; order of the input preserved. */
export function filterLogs<T extends LogLike>(
  entries: T[],
  level: LogLevelFilter,
  query: string,
): T[] {
  return entries.filter((e) => matchesLevel(e, level) && logMatches(e, query));
}

/**
 * The slice the view renders. Filtering happens BEFORE the truncation,
 * never after: searching a word that only appears in an entry past the
 * display limit must still find it — a slice-first pipeline would drop
 * the entry and tell the user it does not exist.
 */
export function displayLogs<T extends LogLike>(
  entries: T[],
  level: LogLevelFilter,
  query: string,
  limit: number,
): T[] {
  if (limit <= 0) return [];
  return filterLogs(entries, level, query).slice(-limit);
}

export function isFilterActive(level: LogLevelFilter, search: string): boolean {
  return level !== "all" || search.trim() !== "";
}

/**
 * One entry as text. The time part is the same `slice(11, 19)` the view
 * renders, so a copy reads like the screen it comes from.
 */
export function formatLogLine(entry: LogLike): string {
  const level = levelName(entry.level);
  const time = entry.timestamp?.slice(11, 19) ?? "";
  return time === "" ? `${level} ${entry.message}` : `${time} ${level} ${entry.message}`;
}

export function formatLogText(entries: LogLike[]): string {
  return entries.map(formatLogLine).join("\n");
}

/**
 * A clipboard write that never settles must still be reported: race it
 * against a deadline so a hung promise becomes a visible error instead
 * of a button that says nothing. A hung writeText has never been
 * observed in this app — the case is a hypothesis, so this is a
 * precaution, not the fix of a measured bug.
 */
export const CLIPBOARD_TIMEOUT_MS = 2000;

export function withTimeout(
  promise: Promise<unknown>,
  ms: number,
  timeoutMessage: string,
): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(timeoutMessage)), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

/**
 * The one colour vocabulary for log levels — rows and filter chips both
 * read it, so a level never changes colour between the two.
 */
export function levelColor(level: string): string {
  switch (levelBucket(level)) {
    case "error":
      return "text-rose-deep";
    case "warn":
      return "text-peach-deep";
    case "info":
      return "text-sky-deep";
    default:
      return "text-azure-900/70";
  }
}

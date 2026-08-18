import type { LogLevelFilter } from "./log-filter";

/**
 * The log tab's state lives here, outside the component.
 *
 * Svelte destroys a view when you navigate away, which would throw away
 * the level filter, the search and the auto-scroll switch. Keeping the
 * state in a module means leaving the tab and coming back restores the
 * exact same screen — the convention set by search-state and
 * library-state.
 */
class LogsStore {
  level = $state<LogLevelFilter>("all");
  search = $state("");
  autoScroll = $state(true);
}

export const logsState = new LogsStore();

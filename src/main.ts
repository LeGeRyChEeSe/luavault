import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { attachConsole, attachLogger } from "@tauri-apps/plugin-log";
import { appState } from "./lib/app-state.svelte";
import { fr, type Key } from "./lib/i18n/fr";
import { t } from "./lib/i18n.svelte";
import { levelName, resolveI18nLogMessage } from "./lib/log-filter";

function isKnownI18nKey(key: string): key is Key {
  return Object.prototype.hasOwnProperty.call(fr, key);
}

// Paint the system's light/dark preference before the first frame. The saved
// appearance replaces it a moment later, once the config comes back from Rust.
document.documentElement.dataset.theme = "azur";
document.documentElement.dataset.mode = window.matchMedia?.(
  "(prefers-color-scheme: dark)",
).matches
  ? "dark"
  : "light";

// Mirror Rust logs to the browser console and to the in-app log viewer.
void attachConsole();
void attachLogger(({ message, level }) => {
  appState.addLog(
    levelName(String(level)),
    resolveI18nLogMessage(message, isKnownI18nKey, (key, args) => t(key, args)),
  );
});

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;

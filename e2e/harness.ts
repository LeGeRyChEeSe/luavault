//! Launches the real LuaVault binary under WebDriver and gives a test a
//! live `Session` on its window.
//!
//! Two properties matter more than anything else here, and both are about not
//! touching the machine running the tests:
//!
//!  1. **The app runs in portable mode inside a throwaway folder.** `config.rs`
//!     resolves `data_dir()` to the executable's own directory as soon as an
//!     `LuaVault.portable` marker sits beside it. So the suite copies the
//!     binary into `.e2e/run-<tag>/`, drops the marker, and every write the app
//!     performs — config, library, index, backups, logs — lands in a folder the
//!     teardown deletes. The real `%LOCALAPPDATA%\LuaVault` is never
//!     opened.
//!  2. **Steam is a fake too.** `steam_dir` is seeded to a directory inside the
//!     sandbox. Without this, `sync_from_steam` would read the user's real
//!     `{Steam}\config\lua` at startup and adopt their actual games into the
//!     test library — and the copy-to-Steam actions would write into their live
//!     Steam install.
//!
//! The licence is machine-bound, not path-bound (`license.rs` hashes a machine
//! seed), so the developer's own `license.json` activates the sandboxed copy
//! and the licence gate does not stand in the way.

import { spawn, type ChildProcess } from 'node:child_process';
import { existsSync, mkdirSync, copyFileSync, writeFileSync, rmSync, readdirSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Session } from './webdriver.ts';

const HERE = dirname(fileURLToPath(import.meta.url));
export const ROOT = join(HERE, '..');

export const APP_BINARY = join(ROOT, 'src-tauri', 'target', 'debug', 'LuaVault.exe');
export const EDGE_DRIVER = join(ROOT, '.e2e', 'bin', 'msedgedriver.exe');
const REAL_DATA_DIR = join(process.env.LOCALAPPDATA ?? '', 'LuaVault');

/// What a test may seed into the sandbox before the app starts.
export interface Seed {
  /// Entries written verbatim to `library/index.json`.
  index?: unknown[];
  /// `{ "480": "-- lua body" }` → `library/480.lua`.
  lua?: Record<string, string>;
  /// Merged over the default config.json.
  config?: Record<string, unknown>;
  /// Extra files, relative to the sandbox root: path → contents. Binary
  /// fixtures are useful when the production path identifies a format by its
  /// magic bytes rather than its filename.
  files?: Record<string, string | Uint8Array>;
  /// Skip copying the developer licence — for testing the licence gate itself.
  withoutLicence?: boolean;
}

export interface AppHandle {
  session: Session;
  /// The sandbox root, which is also the app's data dir in portable mode.
  sandbox: string;
  /// The fake Steam directory the app was pointed at.
  steamDir: string;
}

function freePort(base: number): number {
  // The suite runs one app at a time; a per-process offset is enough to keep
  // two concurrent local runs from colliding on 4444.
  return base + (process.pid % 500);
}

async function waitForDriver(base: string, timeoutMs = 20_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const res = await fetch(`${base}/status`);
      if (res.ok) return;
    } catch {
      /* not up yet */
    }
    if (Date.now() > deadline) throw new Error(`tauri-driver n'a pas répondu sur ${base} en ${timeoutMs} ms`);
    await new Promise((r) => setTimeout(r, 150));
  }
}

/// Assert the toolchain is present, with an actionable message rather than a
/// stack trace from deep inside `fetch`.
export function checkPrerequisites(): string[] {
  const problems: string[] = [];
  if (!existsSync(APP_BINARY)) {
    // NOT `cargo build`: that yields a *dev* binary pointing at localhost:1420
    // and the window lands on chrome-error:// (pitfall 41).
    problems.push(`binaire absent : ${APP_BINARY}\n    → npm run e2e:build`);
  }
  if (!existsSync(EDGE_DRIVER)) {
    problems.push(
      `msedgedriver absent : ${EDGE_DRIVER}\n` +
        `    → npm run e2e:setup  (télécharge la version qui correspond au runtime WebView2)`,
    );
  }
  return problems;
}

/// Remove `run-*` sandboxes left by a runner that was killed mid-flight.
///
/// `App.close` only ever deletes its own, so a hard kill leaks tens of MB per
/// suite — one was found sitting at 33 MB. The age gate keeps a concurrent run
/// safe: a sandbox in use has been created seconds ago, not an hour.
function sweepStaleSandboxes(): void {
  const base = join(ROOT, '.e2e');
  const cutoff = Date.now() - 60 * 60 * 1000;
  let entries: string[];
  try {
    entries = readdirSync(base);
  } catch {
    return; // first run: .e2e does not exist yet
  }
  for (const name of entries) {
    if (!name.startsWith('run-')) continue;
    const path = join(base, name);
    try {
      if (statSync(path).mtimeMs < cutoff) rmSync(path, { recursive: true, force: true });
    } catch {
      // A sandbox that vanished or is locked is not this run's problem.
    }
  }
}

export class App {
  private driver: ChildProcess | null = null;
  private constructor(
    readonly handle: AppHandle,
    driver: ChildProcess,
  ) {
    this.driver = driver;
  }

  get session(): Session {
    return this.handle.session;
  }
  get sandbox(): string {
    return this.handle.sandbox;
  }
  get steamDir(): string {
    return this.handle.steamDir;
  }
  static async launch(tag: string, seed: Seed = {}): Promise<App> {
    sweepStaleSandboxes();
    const sandbox = join(ROOT, '.e2e', `run-${tag}-${process.pid}`);
    rmSync(sandbox, { recursive: true, force: true });
    mkdirSync(join(sandbox, 'library'), { recursive: true });

    const steamDir = join(sandbox, 'steam');
    mkdirSync(join(steamDir, 'config', 'lua'), { recursive: true });
    mkdirSync(join(steamDir, 'steamapps'), { recursive: true });

    // Portable marker — this is what redirects every write into `sandbox`.
    writeFileSync(join(sandbox, 'LuaVault.portable'), '');
    const exe = join(sandbox, 'LuaVault.exe');
    copyFileSync(APP_BINARY, exe);

    if (!seed.withoutLicence) {
      const licence = join(REAL_DATA_DIR, 'license.json');
      if (existsSync(licence)) copyFileSync(licence, join(sandbox, 'license.json'));
    }

    // `first_run_done` skips the onboarding window; without it every test would
    // have to dismiss it before reaching the screen it actually tests.
    const config = {
      first_run_done: true,
      steam_dir: steamDir,
      defender_exclusions: false,
      locale: "fr",
      ...(seed.config ?? {}),
    };
    writeFileSync(join(sandbox, 'config.json'), JSON.stringify(config, null, 2));

    if (seed.index) {
      writeFileSync(join(sandbox, 'library', 'index.json'), JSON.stringify(seed.index, null, 2));
    }
    for (const [appId, body] of Object.entries(seed.lua ?? {})) {
      writeFileSync(join(sandbox, 'library', `${appId}.lua`), body);
    }
    for (const [rel, body] of Object.entries(seed.files ?? {})) {
      const dest = join(sandbox, rel);
      mkdirSync(dirname(dest), { recursive: true });
      writeFileSync(dest, body);
    }

    const port = freePort(4444);
    const nativePort = freePort(5555);
    const base = `http://127.0.0.1:${port}`;
    const driver = spawn(
      'tauri-driver',
      ['--port', String(port), '--native-port', String(nativePort), '--native-driver', EDGE_DRIVER],
      // No `shell: true`. Going through cmd.exe leaves tauri-driver and
      // msedgedriver orphaned when the wrapper is killed, and the very first
      // run of this suite hung exactly that way.
      { stdio: ['ignore', 'pipe', 'pipe'], env: process.env },
    );
    driver.stderr?.on('data', (d) => {
      const line = String(d).trim();
      if (line) process.stderr.write(`  [tauri-driver] ${line}\n`);
    });

    try {
      await waitForDriver(base);
      const session = await Session.create(base, {
        'tauri:options': { application: exe },
      });
      return new App({ session, sandbox, steamDir }, driver);
    } catch (e) {
      driver.kill();
      throw e;
    }
  }

  async close(keepSandbox = false): Promise<void> {
    await this.handle.session.quit();
    this.driver?.kill();
    this.driver = null;
    // tauri-driver spawns the app and msedgedriver as children; killing the
    // shell wrapper does not always reap them.
    await new Promise((r) => setTimeout(r, 300));
    if (!keepSandbox) {
      rmSync(this.handle.sandbox, { recursive: true, force: true });
    }
  }
}

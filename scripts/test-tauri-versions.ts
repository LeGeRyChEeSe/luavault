// @ts-nocheck
// `tauri build` refuses to compile when an npm package and its Rust crate are not on the same
// major.minor ("Found version mismatched Tauri packages"). Dependabot bumps the two ecosystems
// in separate pull requests, so the gap only showed on release day, after the validation had
// passed. This reads both lock files and fails as soon as one pair drifts.
import { readFileSync } from "fs";

const PAIRS: Array<[string, string]> = [
  ["@tauri-apps/api", "tauri"],
  ["@tauri-apps/plugin-dialog", "tauri-plugin-dialog"],
  ["@tauri-apps/plugin-log", "tauri-plugin-log"],
  ["@tauri-apps/plugin-opener", "tauri-plugin-opener"],
];

export function majorMinor(version: string): string {
  return version.split(".").slice(0, 2).join(".");
}

export function cargoVersions(lock: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const m of lock.matchAll(/\[\[package\]\]\r?\nname = "([^"]+)"\r?\nversion = "([^"]+)"/g)) out.set(m[1], m[2]);
  return out;
}

export function mismatches(npmLock: any, cargoLock: string): string[] {
  const crates = cargoVersions(cargoLock);
  const problems: string[] = [];
  for (const [pkg, crate] of PAIRS) {
    const npm = npmLock.packages?.[`node_modules/${pkg}`]?.version;
    const rust = crates.get(crate);
    if (!npm || !rust) problems.push(`${pkg} / ${crate}: absent d'un des deux lock files (npm ${npm}, cargo ${rust})`);
    else if (majorMinor(npm) !== majorMinor(rust)) problems.push(`${pkg} ${npm} / ${crate} ${rust}`);
  }
  return problems;
}

let cases = 0;
function check(name: string, ok: boolean) {
  if (!ok) throw new Error(`échec : ${name}`);
  cases++;
}

// The check itself must be able to go red and to stay green.
const cargoFixture = (v: string) => `[[package]]\nname = "tauri"\nversion = "${v}"\n`;
const npmFixture = (v: string) => ({ packages: { "node_modules/@tauri-apps/api": { version: v } } });
check("deux versions du même mineur sont d'accord", !mismatches(npmFixture("2.12.4"), cargoFixture("2.12.5")).some((p) => p.startsWith("@tauri-apps/api ")));
check("un mineur d'écart est signalé", mismatches(npmFixture("2.12.4"), cargoFixture("2.11.5")).some((p) => p.startsWith("@tauri-apps/api ")));

const problems = mismatches(JSON.parse(readFileSync("package-lock.json", "utf8")), readFileSync("src-tauri/Cargo.lock", "utf8"));
if (problems.length) {
  console.error("Paquets Tauri désalignés entre npm et Cargo :\n  - " + problems.join("\n  - "));
  process.exit(1);
}
cases++;
console.log(`${cases} cas au vert : les paquets Tauri npm et Rust sont alignés.`);

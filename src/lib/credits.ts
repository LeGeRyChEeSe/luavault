/**
 * Everything this app is built on, and everywhere its knowledge came from.
 * Kept as data so the credits page stays a rendering of the truth rather than
 * a hand-maintained copy of it.
 */

export const DISCORD_INVITE = "https://discord.gg/vSczZGT7aQ";

export interface Credit {
  id: string;
  name: string;
  role: string;
  url: string;
  /** Licence as published by the project, when it has one. */
  licence?: string;
}

export interface CreditGroup {
  id: string;
  title: string;
  blurb: string;
  icon: string;
  items: Credit[];
}

/**
 * Raw group data — no text at all. `as const` is load-bearing, not decoration:
 * it makes `id` a literal union, so `` t(`credits.${grp.id}.title`) `` type-checks
 * against the catalogue. Without it `id` is `string`, the template literal is
 * `string`, and the only way to compile is to widen `t()` itself — which is what
 * the first cut of this round did, disarming the key check for the whole
 * application in one line.
 */
const CREDIT_GROUPS_DATA = [
  {
    id: "core",
    icon: "sparkle",
    items: [
      {
        id: "LuaVault",
        name: "LuaVault",
        url: DISCORD_INVITE,
        licence: undefined,
      },
      {
        id: "steamtools",
        name: "SteamTools",
        url: "https://www.steamtools.net/",
        licence: undefined,
      },
      {
        id: "steam",
        name: "Steam",
        url: "https://store.steampowered.com/",
        licence: undefined,
      },
    ],
  },
  {
    id: "shell",
    icon: "tools",
    items: [
      {
        id: "tauri",
        name: "Tauri",
        url: "https://tauri.app/",
        licence: "MIT / Apache-2.0",
      },
      {
        id: "rust",
        name: "Rust",
        url: "https://www.rust-lang.org/",
        licence: "MIT / Apache-2.0",
      },
      {
        id: "svelte",
        name: "Svelte 5",
        url: "https://svelte.dev/",
        licence: "MIT",
      },
      {
        id: "vite",
        name: "Vite",
        url: "https://vite.dev/",
        licence: "MIT",
      },
      {
        id: "tailwind",
        name: "Tailwind CSS",
        url: "https://tailwindcss.com/",
        licence: "MIT",
      },
      {
        id: "typescript",
        name: "TypeScript",
        url: "https://www.typescriptlang.org/",
        licence: "Apache-2.0",
      },
    ],
  },
  {
    id: "rust",
    icon: "archive",
    items: [
      {
        id: "reqwest",
        name: "reqwest + rustls",
        url: "https://github.com/seanmonstar/reqwest",
        licence: "MIT / Apache-2.0",
      },
      {
        id: "tokio",
        name: "tokio",
        url: "https://tokio.rs/",
        licence: "MIT",
      },
      {
        id: "serde",
        name: "serde",
        url: "https://serde.rs/",
        licence: "MIT / Apache-2.0",
      },
      {
        id: "unrar",
        name: "unrar",
        url: "https://github.com/muja/unrar.rs",
        licence: "MIT",
      },
      {
        id: "zip",
        name: "zip",
        url: "https://github.com/zip-rs/zip2",
        licence: "MIT",
      },
      {
        id: "sha2",
        name: "sha2",
        url: "https://github.com/RustCrypto/hashes",
        licence: "MIT / Apache-2.0",
      },
      {
        id: "ed25519",
        name: "ed25519-dalek",
        url: "https://github.com/dalek-cryptography/curve25519-dalek",
        licence: "BSD-3-Clause",
      },
      {
        id: "winreg",
        name: "winreg",
        url: "https://github.com/gentoo90/winreg-rs",
        licence: "MIT",
      },
      {
        id: "utils",
        name: "walkdir · chrono · anyhow · dirs",
        url: "https://crates.io/",
        licence: "MIT / Apache-2.0",
      },
    ],
  },
  {
    id: "services",
    icon: "globe",
    items: [
      {
        id: "openlua",
        name: "openlua.cloud",
        url: "https://openlua.cloud",
        licence: undefined,
      },
      {
        id: "steamwebapi",
        name: "Steam Web API",
        url: "https://partner.steamgames.com/doc/webapi_overview",
        licence: undefined,
      },
    ],
  },
  {
    id: "refs",
    icon: "logs",
    items: [
      {
        id: "vdf",
        name: "Format KeyValues (VDF/ACF) de Valve",
        url: "https://developer.valvesoftware.com/wiki/KeyValues",
        licence: undefined,
      },
      {
        id: "tauridocs",
        name: "Documentation Tauri v2 — capabilities & IPC distant",
        url: "https://tauri.app/security/capabilities/",
        licence: undefined,
      },
      {
        id: "viewtransitions",
        name: "MDN — View Transitions API",
        url: "https://developer.mozilla.org/docs/Web/API/View_Transitions_API",
        licence: undefined,
      },
      {
        id: "oklch",
        name: "OKLCH & couleurs perceptuelles",
        url: "https://oklch.com/",
        licence: undefined,
      },
    ],
  },
] as const;

import { t } from "./i18n.svelte";

/**
 * Group metadata + text for credits, composed at call time so a language switch
 * re-renders every group — the whole point of this file's migration.
 */
export function credits(): CreditGroup[] {
  const result: CreditGroup[] = [];
  for (const grp of CREDIT_GROUPS_DATA) {
    result.push({
      id: grp.id,
      icon: grp.icon,
      items: grp.items.map((item) => ({
        id: item.id,
        name: item.name,
        role: t(`credits.item.${item.id}.role`),
        url: item.url,
        licence: item.licence,
      })),
      title: t(`credits.${grp.id}.title`),
      blurb: t(`credits.${grp.id}.blurb`),
    });
  }
  return result;
}

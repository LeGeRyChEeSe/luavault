import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// The edition this bundle is built for: `scripts/build-exp.ps1` sets
// VITE_LV_EDITION=experimental; anything else is the stable. Frozen here so the
// UI knows its edition before any command answers (src/lib/edition.ts).
// @ts-expect-error process is a nodejs global
const edition = process.env.VITE_LV_EDITION === "experimental" ? "experimental" : "stable";

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  define: {
    "import.meta.env.VITE_LV_EDITION": JSON.stringify(edition),
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // Tauri uses Chromium on Windows; target a compatible baseline
    target: "chrome105",
    // don't minify for debug builds
    // @ts-expect-error process is a nodejs global
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    // produce sourcemaps for debug builds
    // @ts-expect-error process is a nodejs global
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    rollupOptions: {
      input: {
        main: "index.html",
      },
    },
  },
});

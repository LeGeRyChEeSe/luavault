import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  // Vite handles the actual compilation; this exists for svelte-check and tooling.
  preprocess: vitePreprocess(),
};

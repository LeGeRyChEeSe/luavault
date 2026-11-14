<script lang="ts">
  /**
   * Said before the app installs SteamTools by itself (`steamtools_auto.rs`): the app is about to
   * close, Windows will ask for permission once, and the app comes back with the result. No button:
   * the installation is the default, decided with the author on 2026-11-14.
   */
  import { t } from "../lib/i18n.svelte";
  import Icon from "./Icons.svelte";
</script>

<div class="enter-fade fixed inset-0 z-[400] flex items-center justify-center lv-veil p-4" data-steamtools-auto>
  <div role="status" aria-live="polite" class="glass enter-fade w-full max-w-md rounded-xl2 p-6 shadow-xl">
    <div class="mb-2 flex items-center gap-3">
      <span class="shrink-0 animate-spin text-sky-deep [animation-duration:2.4s]"><Icon name="refresh" size={22} /></span>
      <h2 class="text-lg leading-tight font-semibold">{t("steamtools.auto.title")}</h2>
    </div>
    <p class="text-sm text-azure-900/80">{t("steamtools.auto.body")}</p>
    <p class="mt-2 text-xs text-azure-900/55">{t("steamtools.auto.prompt")}</p>
  </div>
</div>

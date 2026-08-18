<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import Icon from "../components/Icons.svelte";
  import { credits, DISCORD_INVITE } from "../lib/credits";
  import { t } from "../lib/i18n.svelte";

</script>

<div class="flex h-full flex-col gap-4 p-1">
  <header class="glass enter-up rounded-xl2 p-5">
    <h2 class="flex items-center gap-2 text-lg font-semibold">
      <Icon name="sparkle" size={20} />
      {t("credits.heading")}
    </h2>
    <p class="mt-0.5 text-sm text-azure-900/60">
      {t("credits.intro")}
    </p>
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto pr-1">
    <div class="flex flex-col gap-4">
      <!-- Discord first: it's the thing we actually want people to do. -->
      <section class="glass enter-up overflow-hidden rounded-xl2">
        <div class="flex flex-wrap items-center gap-5 p-6">
          <div
            class="flex h-14 w-14 shrink-0 items-center justify-center rounded-2xl shadow-lg"
            style="background: linear-gradient(135deg, #5865f2, #4752c4);"
          >
            <Icon name="discord" size={28} tone="current" class="text-white" />
          </div>
          <div class="min-w-0 flex-1">
            <h3 class="text-base font-semibold">{t("credits.discord.heading")}</h3>
            <p class="mt-0.5 text-sm text-azure-900/60">
              {t("credits.discord.lead")}
            </p>
          </div>
          <button
            onclick={() => void openUrl(DISCORD_INVITE)}
            data-tip={DISCORD_INVITE}
            class="lift sheen flex items-center gap-2.5 rounded-xl px-5 py-3 text-sm font-bold text-white shadow-lg"
            style="background: linear-gradient(135deg, #5865f2, #4752c4);"
          >
            <Icon name="discord" size={18} tone="current" />
            {t("credits.discord.action")}
          </button>
        </div>
      </section>

      {#each credits() as group (group.id)}
        <section class="glass enter-up rounded-xl2 p-5">
          <h3 class="flex items-center gap-2 text-sm font-semibold">
            <Icon name={group.icon} size={17} />
            {group.title}
          </h3>
          <p class="mt-0.5 text-xs text-azure-900/50">{group.blurb}</p>

          <div class="mt-3 grid grid-cols-2 gap-2 max-lg:grid-cols-1">
            {#each group.items as item (item.name)}
              <button
                onclick={() => void openUrl(item.url)}
                data-tip={item.url}
                class="lift group flex flex-col items-start gap-1 rounded-xl border border-surface/55 bg-surface/45 px-3.5 py-3 text-left hover:border-azure-300/50 hover:bg-surface/75"
              >
                <span class="flex w-full items-center gap-2">
                  <span class="truncate text-sm font-semibold">{item.name}</span>
                  {#if item.licence}
                    <span
                      class="ml-auto shrink-0 rounded-full bg-azure-500/12 px-1.5 py-px text-[0.6rem] font-bold text-azure-700"
                    >
                      {item.licence}
                    </span>
                  {/if}
                  <Icon
                    name="globe"
                    size={13}
                    class="shrink-0 opacity-0 transition group-hover:opacity-70 {item.licence
                      ? ''
                      : 'ml-auto'}"
                  />
                </span>
                <span class="text-xs leading-relaxed text-azure-900/60">{item.role}</span>
              </button>
            {/each}
          </div>
        </section>
      {/each}

      <p class="px-2 pb-2 text-center text-xs text-azure-900/40">
        {t("credits.thanks")}
      </p>
    </div>
  </div>
</div>

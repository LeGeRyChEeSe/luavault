<script lang="ts">
  /**
   * The files the downloads watcher recognised, offered for import. Nothing is
   * added without a press on « Ajouter » : Escape and the backdrop mean « Ignorer »,
   * because closing a dialog must never be a yes. The files themselves are never
   * moved or deleted.
   */
  import type { DownloadFound } from "../lib/api";
  import { BLOCK_KEYS, planDownload } from "../lib/downloads";
  import { focusTrap } from "../lib/focus-trap";
  import { formatBytes } from "../lib/format";
  import { t } from "../lib/i18n.svelte";
  import Icon from "./Icons.svelte";

  interface Props {
    items: DownloadFound[];
    /** AppID → game name, for the games already in the library. */
    library: ReadonlyMap<string, string>;
    busy: boolean;
    onadd: (paths: string[]) => void;
    onignore: () => void;
  }

  let { items, library, busy, onadd, onignore }: Props = $props();

  const plans = $derived(items.map((item) => ({ item, plan: planDownload(item, library) })));
  // Paths the user unticked; everything importable starts ticked.
  let unticked = $state<string[]>([]);
  const chosen = $derived(
    plans.filter((row) => row.plan.importable && !unticked.includes(row.item.path)).map((row) => row.item.path),
  );

  function toggle(path: string) {
    unticked = unticked.includes(path) ? unticked.filter((p) => p !== path) : [...unticked, path];
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div
  class="enter-fade fixed inset-0 z-[300] flex items-center justify-center lv-veil"
  role="presentation"
  onclick={(e: MouseEvent) => { if (e.target === e.currentTarget && !busy) onignore(); }}
>
  <div
    role="dialog"
    aria-modal="true"
    aria-labelledby="downloads-modal-title"
    tabindex="-1"
    onkeydown={(e) => { if (e.key === "Escape" && !busy) onignore(); }}
    use:focusTrap
    data-downloads-modal
    class="glass enter-fade w-full max-w-lg rounded-xl2 p-6 shadow-xl"
  >
    <div class="mb-3 flex items-start gap-3">
      <span class="mt-0.5 shrink-0"><Icon name="download" size={22} /></span>
      <h2 id="downloads-modal-title" class="text-lg leading-tight font-semibold">
        {t("downloads.modal.title", { count: items.length })}
      </h2>
    </div>
    <p class="text-sm text-azure-900/80">{t("downloads.modal.body")}</p>

    <ul class="mt-4 flex max-h-72 flex-col gap-2 overflow-y-auto" data-downloads-list>
      {#each plans as { item, plan } (item.path)}
        <li class="rounded-xl bg-surface/55 px-3 py-2.5 text-sm" data-downloads-item={item.name}>
          <label class="flex cursor-pointer items-start gap-3 {plan.importable ? '' : 'cursor-not-allowed opacity-70'}">
            <input
              type="checkbox"
              class="mt-1 accent-azure-500"
              checked={plan.importable && !unticked.includes(item.path)}
              disabled={!plan.importable || busy}
              onchange={() => toggle(item.path)}
            />
            <span class="min-w-0 flex-1">
              <span class="block truncate font-medium" data-tip={item.name}>{item.name}</span>
              <span class="block text-xs text-azure-900/60">
                {item.kind === "lua" ? t("downloads.kind.lua") : t("downloads.kind.archive")}
                · {formatBytes(item.size)}
                {#if item.app_id}
                  · {plan.importable && plan.gameName
                    ? t("downloads.item.game", { name: plan.gameName, appId: item.app_id })
                    : t("downloads.item.appid", { appId: item.app_id })}
                {/if}
              </span>
              {#if !plan.importable}
                <span class="mt-1 block text-xs text-peach-deep" data-downloads-reason>
                  {t(BLOCK_KEYS[plan.reason], { appId: item.app_id ?? "" })}
                </span>
              {/if}
            </span>
          </label>
        </li>
      {/each}
    </ul>

    <div class="mt-5 flex flex-wrap justify-end gap-2">
      <button
        onclick={onignore}
        disabled={busy}
        data-downloads-ignore
        class="rounded-lg border border-surface/70 bg-surface/60 px-3 py-1.5 text-xs font-medium text-azure-800 hover:bg-surface/90 disabled:opacity-50"
      >
        {t("downloads.modal.ignore")}
      </button>
      <button
        onclick={() => onadd(chosen)}
        disabled={busy || chosen.length === 0}
        data-downloads-add
        class="rounded-lg bg-azure-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-azure-600 disabled:opacity-50"
      >
        {busy ? t("downloads.modal.busy") : t("downloads.modal.add", { count: chosen.length })}
      </button>
    </div>
  </div>
</div>

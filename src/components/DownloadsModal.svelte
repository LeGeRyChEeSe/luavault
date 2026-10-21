<script lang="ts">
  /**
   * The files the detection recognised, offered for import. Nothing is added without a
   * press on « Ajouter » : Escape and the backdrop mean « Ignorer » (or « Fermer »), because
   * closing a dialog must never be a yes. The files themselves are never moved or deleted.
   *
   * Two uses of the same list. A fresh arrival starts with everything ticked; the list the
   * user opens on purpose (« Fichiers détectés »: old files, ignored ones) starts with
   * nothing ticked, so adding is always a choice.
   */
  import type { DownloadFound, SteamName } from "../lib/api";
  import { BLOCK_KEYS, checkLine, flavorKey, planDownload, titleInFileName, titleLooksRight } from "../lib/downloads";
  import { focusTrap } from "../lib/focus-trap";
  import { formatBytes } from "../lib/format";
  import { t } from "../lib/i18n.svelte";
  import Icon from "./Icons.svelte";

  interface Props {
    items: DownloadFound[];
    /** AppID → game name, for the games already in the library. */
    library: ReadonlyMap<string, string>;
    /** What Steam calls the other AppIDs, when it has answered. */
    steamNames: Readonly<Record<string, SteamName>>;
    /** Opened by the user on the whole inventory, not by a fresh arrival. */
    browsing: boolean;
    busy: boolean;
    onadd: (paths: string[]) => void;
    onignore: () => void;
  }

  let { items, library, steamNames, browsing, busy, onadd, onignore }: Props = $props();

  /** The game's name: the library's, else Steam's. */
  const nameOf = (item: DownloadFound, planned: string | null) =>
    planned ?? (item.app_id ? (steamNames[item.app_id]?.name ?? null) : null);

  /** A warning about the AppID itself, from what Steam says. */
  function steamWarning(item: DownloadFound): { key: "downloads.steam.unknown" | "downloads.steam.mismatch"; name: string } | null {
    const said = item.app_id ? steamNames[item.app_id] : undefined;
    if (!said || (item.app_id && library.has(item.app_id))) return null;
    if (said.status === "unknown") return { key: "downloads.steam.unknown", name: "" };
    const title = titleInFileName(item.name);
    if (said.status === "known" && said.name && title && !titleLooksRight(title, said.name)) {
      return { key: "downloads.steam.mismatch", name: said.name };
    }
    return null;
  }

  // What the user ticked or unticked; anything untouched follows the default of the list.
  let touched = $state<Record<string, boolean>>({});
  const isTicked = (path: string) => touched[path] ?? !browsing;

  // The `.lua` files ticked here: a patch for one of those games can be added with it.
  const tickedLuaIds = $derived(
    new Set(
      items
        .filter((item) => item.kind === "lua" && item.app_id && isTicked(item.path))
        .map((item) => item.app_id as string),
    ),
  );
  const plans = $derived(items.map((item) => ({ item, plan: planDownload(item, library, tickedLuaIds) })));
  const chosen = $derived(
    plans.filter((row) => row.plan.importable && isTicked(row.item.path)).map((row) => row.item.path),
  );
  const importable = $derived(plans.filter((row) => row.plan.importable).map((row) => row.item.path));

  function toggle(path: string) {
    touched = { ...touched, [path]: !isTicked(path) };
  }

  function setAll(on: boolean) {
    touched = Object.fromEntries(importable.map((path) => [path, on]));
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
    data-downloads-mode={browsing ? "browse" : "arrival"}
    class="glass enter-fade w-full max-w-lg rounded-xl2 p-6 shadow-xl"
  >
    <div class="mb-3 flex items-start gap-3">
      <span class="mt-0.5 shrink-0"><Icon name="download" size={22} /></span>
      <h2 id="downloads-modal-title" class="text-lg leading-tight font-semibold">
        {browsing ? t("downloads.inventory.title") : t("downloads.modal.title", { count: items.length })}
      </h2>
    </div>
    <p class="text-sm text-azure-900/80">{browsing ? t("downloads.inventory.body") : t("downloads.modal.body")}</p>

    {#if items.length === 0}
      <p class="mt-4 rounded-xl bg-surface/55 px-3 py-3 text-sm text-azure-900/70" data-downloads-empty>
        {t("downloads.inventory.empty")}
      </p>
    {:else}
      {#if importable.length > 1}
        <div class="mt-3 flex gap-3 text-xs">
          <button type="button" class="text-azure-700 underline-offset-2 hover:underline" onclick={() => setAll(true)} disabled={busy} data-downloads-all>
            {t("downloads.modal.all")}
          </button>
          <button type="button" class="text-azure-700 underline-offset-2 hover:underline" onclick={() => setAll(false)} disabled={busy} data-downloads-none>
            {t("downloads.modal.none")}
          </button>
        </div>
      {/if}
      <ul class="mt-3 flex max-h-72 flex-col gap-2 overflow-y-auto" data-downloads-list>
        {#each plans as { item, plan } (item.path)}
          <li class="rounded-xl bg-surface/55 px-3 py-2.5 text-sm" data-downloads-item={item.name}>
            <label class="flex cursor-pointer items-start gap-3 {plan.importable ? '' : 'cursor-not-allowed opacity-70'}">
              <input
                type="checkbox"
                class="mt-1 accent-azure-500"
                checked={plan.importable && isTicked(item.path)}
                disabled={!plan.importable || busy}
                onchange={() => toggle(item.path)}
              />
              <span class="min-w-0 flex-1">
                <span class="block truncate font-medium" data-tip={item.name}>{item.name}</span>
                <span class="block text-xs text-azure-900/60">
                  {t(flavorKey(item))}
                  · {formatBytes(item.size)}
                  {#if item.app_id}
                    · {nameOf(item, plan.importable ? plan.gameName : null)
                      ? t("downloads.item.game", { name: nameOf(item, plan.importable ? plan.gameName : null) ?? "", appId: item.app_id })
                      : t("downloads.item.appid", { appId: item.app_id })}
                  {/if}
                </span>
                {#if checkLine(item)}
                  {@const line = checkLine(item)}
                  {#if line}
                    <span class="mt-1 block text-xs {line.good ? 'text-mint-deep' : 'text-peach-deep'}" data-downloads-check={item.archive_check}>
                      {t(line.key)}
                    </span>
                  {/if}
                {/if}
                {#if steamWarning(item)}
                  {@const warning = steamWarning(item)}
                  {#if warning}
                    <span class="mt-1 block text-xs text-peach-deep" data-downloads-steam-warning>
                      {t(warning.key, { name: warning.name })}
                    </span>
                  {/if}
                {/if}
                {#if item.replaces}
                  <span class="mt-1 block text-xs text-peach-deep" data-downloads-replaces>
                    {t("downloads.replaces")}
                  </span>
                {/if}
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
    {/if}

    <div class="mt-5 flex flex-wrap justify-end gap-2">
      <button
        onclick={onignore}
        disabled={busy}
        data-downloads-ignore
        class="rounded-lg border border-surface/70 bg-surface/60 px-3 py-1.5 text-xs font-medium text-azure-800 hover:bg-surface/90 disabled:opacity-50"
      >
        {browsing ? t("downloads.modal.close") : t("downloads.modal.ignore")}
      </button>
      {#if items.length > 0}
        <button
          onclick={() => onadd(chosen)}
          disabled={busy || chosen.length === 0}
          data-downloads-add
          class="rounded-lg bg-azure-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-azure-600 disabled:opacity-50"
        >
          {busy ? t("downloads.modal.busy") : t("downloads.modal.add", { count: chosen.length })}
        </button>
      {/if}
    </div>
  </div>
</div>

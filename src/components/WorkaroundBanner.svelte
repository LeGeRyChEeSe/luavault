<script lang="ts">
  /**
   * The Steam fix is the only thing that makes this app work on a machine where
   * SteamTools is broken by Steam's own update: so it stays in sight, on every tab,
   * until it is applied. It never acts alone: one click, then a confirmation, and
   * the backend refuses while a game runs. It goes away by itself once the machine
   * no longer needs the fix (`motd::workaround`).
   */
  import { appState } from "../lib/app-state.svelte";
  import { runMotdAction, steamUpdateRunning } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  import Icon from "./Icons.svelte";
  import ConfirmButton from "./ConfirmButton.svelte";

  let failure = $state<string | null>(null);
  // The move to the classic SteamTools (repair, then Steam's restart) speaks its own texts; the
  // old Steam keeps its own. Every key is written out: the catalogue guard reads them here.
  function texts(kind: string | undefined) {
    if (kind === "repair_tools") {
      return { title: t("workaround.repair.title"), body: t("workaround.repair.body"), apply: t("workaround.repair.apply"), confirm: t("workaround.repair.confirm"), tip: t("workaround.repair.tip") };
    }
    if (kind === "steam_update") {
      return { title: t("workaround.update.title"), body: t("workaround.update.body"), apply: t("workaround.update.apply"), confirm: t("workaround.update.confirm"), tip: t("workaround.update.tip") };
    }
    return { title: t("workaround.banner.title"), body: t("workaround.banner.body"), apply: t("workaround.banner.apply"), confirm: t("workaround.banner.confirm"), tip: t("workaround.banner.tip") };
  }
  const text = $derived(texts(appState.workaround?.kind));
  // The app is bringing Steam up to date: progress, no button.
  const updating = $derived(steamUpdateRunning(appState.steamUpdate));
  const progress = $derived.by(() => {
    const p = appState.steamUpdate;
    if (p?.phase === "downloading") return t("steam_update.downloading", { percent: p.percent });
    if (p?.phase === "installing") return t("steam_update.installing");
    return t("steam_update.starting");
  });

  async function apply(): Promise<void> {
    const message = appState.motd;
    const w = appState.workaround;
    if (!message || !w) return;
    failure = null;
    try {
      const text = await runMotdAction(message.id, w.action_id);
      if (text) appState.toast("info", text);
    } catch (e) {
      failure = String(e);
    }
    await appState.refreshWorkaround();
    await appState.refreshMotdStatuses();
    void appState.refresh();
  }
</script>

<div
  role="alert"
  data-workaround-banner
  class="glass enter-up flex shrink-0 flex-wrap items-center gap-3 rounded-xl2 border border-peach/40 bg-peach-soft/40 px-4 py-3"
>
  <Icon name="alert" size={20} />
  <div class="min-w-0 flex-1">
    {#if updating}
      <p class="text-sm font-semibold">{t("steam_update.title")}</p>
      <p class="text-xs text-azure-900/70" data-steam-update>{progress}</p>
    {:else}
      <p class="text-sm font-semibold">{text.title}</p>
      <p class="text-xs text-azure-900/70">{text.body}</p>
    {/if}
    {#if failure}
      <p class="mt-1 text-xs text-rose-deep" data-workaround-error>{failure}</p>
    {/if}
  </div>
  {#if !updating}
  <ConfirmButton
    label={text.apply}
    confirmLabel={text.confirm}
    onconfirm={apply}
    primary
    title={text.tip}
  />
  {/if}
</div>

<script lang="ts">
  /**
   * The Steam fix is the only thing that makes this app work on a machine where
   * SteamTools is broken by Steam's own update: so it stays in sight, on every tab,
   * until it is applied. It never acts alone: one click, then a confirmation, and
   * the backend refuses while a game runs. It goes away by itself once the machine
   * no longer needs the fix (`motd::workaround`).
   */
  import { appState } from "../lib/app-state.svelte";
  import { runMotdAction } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  import Icon from "./Icons.svelte";
  import ConfirmButton from "./ConfirmButton.svelte";

  let failure = $state<string | null>(null);

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
    <p class="text-sm font-semibold">{t("workaround.banner.title")}</p>
    <p class="text-xs text-azure-900/70">{t("workaround.banner.body")}</p>
    {#if failure}
      <p class="mt-1 text-xs text-rose-deep" data-workaround-error>{failure}</p>
    {/if}
  </div>
  <ConfirmButton
    label={t("workaround.banner.apply")}
    confirmLabel={t("workaround.banner.confirm")}
    onconfirm={apply}
    primary
    title={t("workaround.banner.tip")}
  />
</div>

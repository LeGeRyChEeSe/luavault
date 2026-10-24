<script lang="ts">
  /**
   * Steam is stuck on a game file and no key is on hand to fetch it. A key comes from a
   * provider's own page, where the user passes its verification by themselves, so this stays
   * in sight on every tab until a key is pasted: nobody should have to hunt for the screen
   * that asks. It never acts alone, and it never touches the page it opens.
   */
  import { appState } from "../lib/app-state.svelte";
  import { openManifestPage } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  import ActionButton from "./ActionButton.svelte";
  import Icon from "./Icons.svelte";

  let { onpaste }: { onpaste: () => void } = $props();

  async function openPage(): Promise<void> {
    try {
      await openManifestPage("manifesthub");
    } catch (e) {
      appState.toast("error", String(e));
    }
  }
</script>

<div
  role="alert"
  data-manifest-banner
  class="glass enter-up flex shrink-0 flex-wrap items-center gap-3 rounded-xl2 border border-peach/40 bg-peach-soft/40 px-4 py-3"
>
  <Icon name="alert" size={20} />
  <div class="min-w-0 flex-1">
    <p class="text-sm font-semibold">{t("manifest.banner.title")}</p>
    <p class="text-xs text-azure-900/70">{t("manifest.banner.body")}</p>
  </div>
  <ActionButton
    label={t("manifest.banner.open")}
    icon="globe"
    variant="primary"
    onclick={openPage}
    tip={t("manifest.banner.open_tip")}
  />
  <ActionButton label={t("manifest.banner.paste")} icon="check" onclick={onpaste} tip={t("manifest.banner.paste_tip")} />
</div>

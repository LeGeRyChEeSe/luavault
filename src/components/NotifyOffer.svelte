<script lang="ts">
  /**
   * The one-time offer to be warned by a Windows notification even when the app
   * is closed. Asked once: either answer is remembered (`background_notify`), and
   * the switch in Settings is the only way back to it.
   *
   * Nothing is registered until « Activer » is pressed. Escape and the backdrop
   * mean « Pas maintenant »: closing a dialog must never be a yes.
   */
  import { focusTrap } from "../lib/focus-trap";
  import { t } from "../lib/i18n.svelte";
  import Icon from "./Icons.svelte";

  interface Props {
    onanswer: (enable: boolean) => void;
  }

  let { onanswer }: Props = $props();
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div
  class="enter-fade fixed inset-0 z-[300] flex items-center justify-center lv-veil"
  role="presentation"
  onclick={(e: MouseEvent) => { if (e.target === e.currentTarget) onanswer(false); }}
>
  <div
    role="dialog"
    aria-modal="true"
    aria-labelledby="notify-offer-title"
    tabindex="-1"
    onkeydown={(e) => { if (e.key === "Escape") onanswer(false); }}
    use:focusTrap
    data-notify-offer
    class="glass enter-fade w-full max-w-md rounded-xl2 p-6 shadow-xl"
  >
    <div class="mb-3 flex items-start gap-3">
      <span class="mt-0.5 shrink-0"><Icon name="info" size={22} /></span>
      <h2 id="notify-offer-title" class="text-lg leading-tight font-semibold">
        {t("notify.offer.title")}
      </h2>
    </div>
    <p class="text-sm text-azure-900/80">{t("notify.offer.body")}</p>
    <p class="mt-2 text-xs text-azure-900/55">{t("notify.offer.how")}</p>
    <div class="mt-5 flex flex-wrap justify-end gap-2">
      <button
        onclick={() => onanswer(false)}
        data-notify-offer-no
        class="rounded-lg border border-surface/70 bg-surface/60 px-3 py-1.5 text-xs font-medium text-azure-800 hover:bg-surface/90"
      >
        {t("notify.offer.no")}
      </button>
      <button
        onclick={() => onanswer(true)}
        data-notify-offer-yes
        class="rounded-lg bg-azure-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-azure-600"
      >
        {t("notify.offer.yes")}
      </button>
    </div>
  </div>
</div>

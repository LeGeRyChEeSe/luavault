<script lang="ts">
  /**
   * La fenêtre du message du jour. Ouverte au démarrage quand un message
   * actif n'a pas été écarté, et à la demande depuis la barre latérale.
   *
   * « Ne plus afficher » ne s'applique qu'à CET identifiant : une nouvelle
   * publication, même du même texte, réapparaît. C'est ce qui permet à
   * l'auteur d'insister sans jamais pouvoir contourner un choix passé — le
   * choix portait sur un message, pas sur la fonctionnalité.
   */
  import { focusTrap } from "../lib/focus-trap";
  import { t, i18n } from "../lib/i18n.svelte";
  import { motdSeverity, resolveMotdText } from "../lib/motd";
  import type { MotdSeverity } from "../lib/motd";
  import type { MotdMessage } from "../lib/api";
  import { formatUnixDate } from "../lib/format";
  import Icon from "./Icons.svelte";
  import MotdMarkdown from "./MotdMarkdown.svelte";

  interface Props {
    message: MotdMessage;
    /** True when the user had already asked to forget this id. */
    dismissed: boolean;
    onclose: (remember: boolean) => void;
  }

  let { message, dismissed, onclose }: Props = $props();

  // The box starts where the user left it; later changes to the prop are not
  // meant to move it under their hand, so capturing the initial value is right.
  // svelte-ignore state_referenced_locally
  let remember = $state(dismissed);

  const severity = $derived(motdSeverity(message.severity));
  const title = $derived(resolveMotdText(message.title, i18n.locale));
  const body = $derived(resolveMotdText(message.body, i18n.locale));
  const publishedAt = $derived(Math.floor(Date.parse(message.published_at) / 1000));

  // Le libellé du ton est `motd.severity.${severity}` : un gabarit typé par
  // l'union `MotdSeverity`, que svelte-check vérifie contre le catalogue —
  // pas un cast, qui rendrait muette la garde des clés (test-i18n-wiring).
  const TONE: Record<MotdSeverity, { icon: string; badge: string }> = {
    info: { icon: "info", badge: "bg-sky-soft/60 text-sky-deep" },
    warning: { icon: "alert", badge: "bg-peach-soft/60 text-peach-deep" },
    critical: { icon: "error", badge: "bg-rose-soft/60 text-rose-deep" },
  };

  function close(): void {
    onclose(remember);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div
  class="enter-fade fixed inset-0 z-[300] flex items-center justify-center lv-veil"
  role="presentation"
  onclick={(e: MouseEvent) => { if (e.target === e.currentTarget) close(); }}
>
  <div
    role="dialog"
    aria-modal="true"
    aria-labelledby="motd-dialog-title"
    tabindex="-1"
    onkeydown={(e) => { if (e.key === "Escape") close(); }}
    use:focusTrap
    data-motd-dialog
    class="glass enter-fade w-full max-w-lg rounded-xl2 p-6 shadow-xl"
  >
    <div class="mb-3 flex items-start gap-3">
      <span class="mt-0.5 shrink-0"><Icon name={TONE[severity].icon} size={22} /></span>
      <div class="min-w-0 flex-1">
        <h2 id="motd-dialog-title" class="text-lg leading-tight font-semibold">
          {title || t("motd.title.default")}
        </h2>
        <p class="mt-1 flex flex-wrap items-center gap-2 text-xs text-azure-900/60">
          <span class="rounded-full px-2 py-0.5 font-medium {TONE[severity].badge}">
            {t(`motd.severity.${severity}`)}
          </span>
          {#if Number.isFinite(publishedAt)}
            <span>{t("motd.published", { date: formatUnixDate(publishedAt) })}</span>
          {/if}
        </p>
      </div>
    </div>

    <div class="max-h-[60vh] overflow-y-auto rounded-xl bg-surface/50 px-4 py-3" data-motd-body>
      <MotdMarkdown source={body} />
    </div>

    <div class="mt-4 flex flex-wrap items-center justify-between gap-3">
      <label class="flex cursor-pointer items-center gap-2 text-xs text-azure-900/70">
        <input type="checkbox" bind:checked={remember} class="accent-azure-500" data-motd-remember />
        <span>{t("motd.remember")}</span>
      </label>
      <button
        onclick={close}
        data-motd-close
        class="rounded-lg bg-azure-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-azure-600"
      >
        {t("motd.close")}
      </button>
    </div>
  </div>
</div>

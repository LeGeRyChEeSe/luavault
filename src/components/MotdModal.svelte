<script lang="ts">
  /**
   * La fenêtre du message du jour. Ouverte au démarrage quand un message
   * actif n'a pas été écarté, et à la demande depuis la barre latérale.
   *
   * « Ne plus afficher » ne s'applique qu'à CET identifiant : une nouvelle
   * publication, même du même texte, réapparaît. C'est ce qui permet à
   * l'auteur d'insister sans jamais pouvoir contourner un choix passé — le
   * choix portait sur un message, pas sur la fonctionnalité.
   *
   * Les boutons sous le texte font la chose recommandée en un clic. Ils ne
   * portent aucun paramètre : on rend au backend « ce message, ce bouton », et
   * c'est lui qui retrouve l'action dans le message qu'il a vérifié. Ceux qui
   * ferment ou relancent Steam demandent un second clic (`ConfirmButton`).
   */
  import { focusTrap } from "../lib/focus-trap";
  import { t, i18n } from "../lib/i18n.svelte";
  import { actionLabel, actionNeedsConfirm, isForced, motdSeverity, presentationOf, resolveMotdText, showsMainButton } from "../lib/motd";
  import type { MotdSeverity } from "../lib/motd";
  import { runMotdAction } from "../lib/api";
  import type { MotdAction, MotdMessage } from "../lib/api";
  import { appState } from "../lib/app-state.svelte";
  import ActionButton from "./ActionButton.svelte";
  import ConfirmButton from "./ConfirmButton.svelte";
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

  const actions = $derived(message.actions ?? []);

  // Une seule action à la fois : relancer Steam deux fois de suite n'aide personne.
  let busyAction = $state<string | null>(null);
  let outcome = $state<{ ok: boolean; text: string } | null>(null);

  // Le libellé par défaut est `motd.action.${kind}` : le gabarit est typé par
  // l'union `MotdActionKind`, comme `motd.severity.*` plus haut.
  function labelOf(action: MotdAction): string {
    return actionLabel(action, i18n.locale) || t(`motd.action.${action.kind}`);
  }

  async function run(action: MotdAction): Promise<void> {
    busyAction = action.id;
    outcome = null;
    try {
      const text = await runMotdAction(message.id, action.id);
      if (text) outcome = { ok: true, text };
      // Steam and the add-on may have changed under the app: read them again, and
      // let the button say it was pressed instead of inviting a second press.
      if (action.kind !== "open_url") void appState.refresh();
      await appState.refreshMotdStatuses();
    } catch (e) {
      outcome = { ok: false, text: String(e) };
    } finally {
      busyAction = null;
    }
  }

  // A persistent notice about a fix still needed here cannot be silenced for good:
  // the box is withdrawn, and closing the window leaves the choice where it was.
  const forced = $derived(isForced(message, appState.workaround));

  function close(): void {
    onclose(forced ? dismissed : remember);
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

    {#if actions.length > 0}
      <div class="mt-4 flex flex-col gap-2" data-motd-actions>
        {#each actions as action, i (action.id)}
          {@const status = appState.motdStatuses[action.id]}
          {@const view = presentationOf(status)}
          {#if view !== "button"}
            <p
              class="flex items-start gap-2 rounded-xl px-3 py-2 text-sm {view === 'lost'
                ? 'bg-peach-soft/55 text-peach-deep'
                : 'bg-mint-soft/55 text-mint-deep'}"
              data-motd-status={action.id}
              data-motd-state={view}
            >
              <span class="mt-0.5 shrink-0"><Icon name={view === "lost" ? "alert" : "check"} size={16} /></span>
              <span>
                {t(`motd.state.${view}`, {
                  label: labelOf(action),
                  date: formatUnixDate(status?.ran_at),
                })}
              </span>
            </p>
          {/if}
          <div class="flex flex-wrap gap-2">
            {#if showsMainButton(view)}
              <div class="contents" data-motd-action={action.id} data-motd-kind={action.kind}>
                {#if actionNeedsConfirm(action.kind)}
                  <ConfirmButton
                    label={labelOf(action)}
                    confirmLabel={t("motd.action.confirm")}
                    onconfirm={() => run(action)}
                    disabled={busyAction !== null}
                    primary={i === 0}
                  />
                {:else}
                  <ActionButton
                    label={labelOf(action)}
                    icon="arrow-right"
                    variant={i === 0 ? "primary" : "ghost"}
                    disabled={busyAction !== null}
                    busy={busyAction === action.id}
                    onclick={() => run(action)}
                  />
                {/if}
              </div>
            {:else if view === "recorded"}
              <!-- Pressed before and nothing can verify it: a way to redo, never the main gesture. -->
              <div class="contents" data-motd-redo={action.id} data-motd-kind={action.kind}>
                <ConfirmButton
                  label={t("motd.action.redo")}
                  confirmLabel={t("motd.action.confirm")}
                  onconfirm={() => run(action)}
                  disabled={busyAction !== null}
                />
              </div>
            {/if}
          </div>
        {/each}
      </div>
      {#if outcome}
        <p
          role="status"
          data-motd-outcome={outcome.ok ? "ok" : "error"}
          class="mt-3 rounded-xl px-3 py-2 text-sm {outcome.ok
            ? 'bg-mint-soft/55 text-mint-deep'
            : 'bg-rose-soft/55 text-rose-deep'}"
        >
          {outcome.text}
        </p>
      {/if}
    {/if}

    <div class="mt-4 flex flex-wrap items-center justify-between gap-3">
      {#if forced}
        <span class="text-xs text-azure-900/60" data-motd-forced>{t("motd.persistent_note")}</span>
      {:else}
        <label class="flex cursor-pointer items-center gap-2 text-xs text-azure-900/70">
          <input type="checkbox" bind:checked={remember} class="accent-azure-500" data-motd-remember />
          <span>{t("motd.remember")}</span>
        </label>
      {/if}
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

<script lang="ts">
  /**
   * The experimental edition's welcome: what it adds, what each feature is for and where to find
   * it. Shown once per `EXP_WELCOME_VERSION` (`exp_welcome_seen`), after the onboarding,
   * never on the stable (`exp_welcome_due` answers false there). Escape and the
   * backdrop close it like « Compris »: nothing here is a choice, the features are already on.
   */
  import { focusTrap } from "../lib/focus-trap";
  import { t } from "../lib/i18n.svelte";
  import Icon from "./Icons.svelte";

  interface Props {
    /** `open`: the person asked to see the « À l'essai » tab. */
    onclose: (open: boolean) => void;
  }

  let { onclose }: Props = $props();

  // Keys written out (the catalogue guard reads `t("…")` calls, not a table of names).
  const FEATURES = $derived([
    { icon: "globe", title: t("trials.welcome.store.title"), what: t("trials.welcome.store.what"), where: t("trials.welcome.store.where") },
    { icon: "steam", title: t("trials.welcome.steam.title"), what: t("trials.welcome.steam.what"), where: t("trials.welcome.steam.where") },
    { icon: "patch", title: t("trials.welcome.patches.title"), what: t("trials.welcome.patches.what"), where: "" },
  ] as const);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div
  class="enter-fade fixed inset-0 z-[300] flex items-center justify-center lv-veil p-4"
  role="presentation"
  onclick={(e: MouseEvent) => { if (e.target === e.currentTarget) onclose(false); }}
>
  <div
    role="dialog"
    aria-modal="true"
    aria-labelledby="exp-welcome-title"
    tabindex="-1"
    onkeydown={(e) => { if (e.key === "Escape") onclose(false); }}
    use:focusTrap
    data-exp-welcome
    class="glass enter-fade flex max-h-full w-full max-w-lg flex-col rounded-xl2 p-6 shadow-xl"
  >
    <div class="mb-2 flex items-start gap-3">
      <span class="mt-0.5 shrink-0 text-lilac-deep"><Icon name="sparkle" size={22} /></span>
      <h2 id="exp-welcome-title" class="text-lg leading-tight font-semibold">
        {t("trials.welcome.title")}
      </h2>
    </div>
    <div class="min-h-0 overflow-y-auto">
      <p class="text-sm text-azure-900/80">{t("trials.welcome.intro")}</p>
      <ul class="mt-3 flex flex-col gap-2">
        {#each FEATURES as f (f.icon)}
          <li class="flex gap-3 rounded-xl bg-surface/55 px-4 py-3">
            <span class="mt-0.5 shrink-0"><Icon name={f.icon} size={16} /></span>
            <span class="flex min-w-0 flex-col gap-0.5">
              <span class="text-sm font-semibold">{f.title}</span>
              <span class="text-xs text-azure-900/70">{f.what}</span>
              {#if f.where}
                <span class="text-xs text-azure-900/55">{f.where}</span>
              {/if}
            </span>
          </li>
        {/each}
      </ul>
      <p class="mt-3 flex gap-2 rounded-xl bg-sky-soft/60 px-4 py-3 text-xs text-sky-deep" data-exp-welcome-restart>
        <Icon name="info" size={15} class="mt-0.5 shrink-0" />
        <span>{t("trials.welcome.restart")}</span>
      </p>
      <p class="mt-2 text-xs text-azure-900/55">{t("trials.welcome.where")}</p>
    </div>
    <div class="mt-5 flex flex-wrap justify-end gap-2">
      <button
        onclick={() => onclose(true)}
        data-exp-welcome-open
        class="rounded-lg border border-surface/70 bg-surface/60 px-3 py-1.5 text-xs font-medium text-azure-800 hover:bg-surface/90"
      >
        {t("trials.welcome.open")}
      </button>
      <button
        onclick={() => onclose(false)}
        data-exp-welcome-ok
        class="rounded-lg bg-azure-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-azure-600"
      >
        {t("trials.welcome.ok")}
      </button>
    </div>
  </div>
</div>

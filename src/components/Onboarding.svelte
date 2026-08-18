<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icons.svelte";
  import ActionButton from "./ActionButton.svelte";
  import ConfirmButton from "./ConfirmButton.svelte";
  import Rich from "./Rich.svelte";
  import StatusBadge from "./StatusBadge.svelte";
  import { appState } from "../lib/app-state.svelte";
  import { focusTrap } from "../lib/focus-trap";
  import { installSteam, installSteamtools, markOnboardingDone, setSteamDir } from "../lib/api";
  import { t } from "../lib/i18n.svelte";

  const report = $derived(appState.report);
  const steam = $derived(report?.steam ?? null);
  const st = $derived(report?.steamtools ?? null);
  let busy = $state<string | null>(null);
  let finishing = $state(false);

  async function run(id: string, action: () => Promise<string>) {
    busy = id;
    try {
      appState.toast("info", await action());
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function pickSteamFolder() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    await run("pick", async () => {
      appState.report = await setSteamDir(sel);
      return t("onboarding.steam.dir-saved");
    });
  }

  const doInstallSteam = () => run("steam", installSteam);

  async function doInstallSteamtools() {
    await run("st", installSteamtools);
    await new Promise((r) => setTimeout(r, 3000));
    await appState.refresh();
  }

  async function finish() {
    finishing = true;
    try {
      await markOnboardingDone();
      await appState.refresh();
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      finishing = false;
    }
  }
</script>

<div
  class="enter-fade fixed inset-0 z-50 flex items-center justify-center bg-azure-900/25 p-6 backdrop-blur-sm"
>
  <div
    class="glass-strong enter-pop flex max-h-[90vh] w-full max-w-2xl flex-col overflow-y-auto rounded-2xl p-7"
    use:focusTrap={{ initial: "container" }}
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-labelledby="onboarding-title"
  >
    <div class="mb-1 flex items-center gap-3">
      <div
        class="flex h-11 w-11 items-center justify-center rounded-xl bg-gradient-to-br from-azure-400 to-azure-600 text-sm font-bold text-white shadow-md"
      >
        LV
      </div>
      <div>
        <h1 id="onboarding-title" class="text-xl font-semibold">{t("onboarding.title")}</h1>
        <p class="text-sm text-azure-900/60">{t("onboarding.subtitle")}</p>
      </div>
    </div>

    <!-- Steam -->
    <section class="mt-5 rounded-xl border border-surface/60 bg-surface/45 p-4">
      <div class="flex flex-wrap items-center gap-2">
        <h2 class="flex items-center gap-2 font-semibold">
          <Icon name="steam" size={17} />
          Steam
        </h2>
        <StatusBadge
          label={steam ? t("onboarding.steam.detected") : t("onboarding.steam.missing")}
          icon={steam ? "check" : "alert"}
          tone={steam ? "good" : "action"}
          compact
          tip={steam ? steam.source : t("onboarding.steam.required")}
        />
      </div>
      {#if steam}
        <p class="mt-2 break-all text-sm text-azure-900/70">{steam.path}</p>
        <div class="mt-3">
          <ActionButton
            label={t("onboarding.steam.pick")}
            icon="edit"
            size="sm"
            disabled={busy !== null}
            onclick={pickSteamFolder}
          />
        </div>
      {:else}
        <p class="mt-2 text-sm text-azure-900/70">{t("onboarding.steam.absent")}</p>
        <div class="mt-3 flex flex-wrap gap-2">
          <ActionButton
            label={t("onboarding.steam.install")}
            icon="download"
            variant="primary"
            size="sm"
            disabled={busy !== null}
            busy={busy === "steam"}
            busyLabel={t("onboarding.steam.install.busy")}
            onclick={doInstallSteam}
          />
          <ActionButton
            label={t("onboarding.steam.have")}
            icon="folder"
            size="sm"
            disabled={busy !== null}
            onclick={pickSteamFolder}
          />
        </div>
      {/if}
    </section>

    <!-- SteamTools -->
    <section class="mt-3 rounded-xl border border-surface/60 bg-surface/45 p-4">
      <div class="flex flex-wrap items-center gap-2">
        <h2 class="flex items-center gap-2 font-semibold">
          <Icon name="tools" size={17} />
          SteamTools
        </h2>
        {#if st?.installed}
          <StatusBadge label={t("onboarding.st.installed")} icon="check" tone="good" compact tip="" />
        {:else if steam}
          <StatusBadge
            label={t("onboarding.st.absent")}
            icon="alert"
            tone="action"
            compact
            tip={t("onboarding.st.absent.tip")}
          />
        {/if}
      </div>
      {#if !steam}
        <p class="mt-2 text-sm text-azure-900/70">{t("onboarding.st.needs-steam")}</p>
      {:else if st?.installed}
        <p class="mt-2 text-sm text-azure-900/70">{t("onboarding.st.ready")}</p>
      {:else}
        <p class="mt-2 text-sm text-azure-900/70">{t("onboarding.st.explain")}</p>
        <div class="mt-3">
          <ConfirmButton
            label={t("onboarding.st.install")}
            confirmLabel={t("onboarding.st.install.confirm")}
            onconfirm={doInstallSteamtools}
            primary
          />
        </div>
      {/if}
    </section>

    <!-- How it works: sets expectations before the first download. -->
    <section class="mt-3 rounded-xl border border-sky/20 bg-sky-soft/40 p-4 text-sm text-sky-deep">
      <h2 class="flex items-center gap-2 font-semibold">
        <Icon name="info" size={17} />
        {t("onboarding.how.title")}
      </h2>
      <ol class="mt-2 flex list-inside list-decimal flex-col gap-1">
        <li>{t("onboarding.how.step1")}</li>
        <li><Rich text={t("onboarding.how.step2")} /></li>
        <li>{t("onboarding.how.step3")}</li>
      </ol>
    </section>

    <div class="mt-6 flex items-center justify-end gap-2">
      <ActionButton
        label={t("onboarding.finish")}
        icon="play"
        variant="primary"
        disabled={finishing}
        busy={finishing}
        busyLabel={t("onboarding.finish.busy")}
        onclick={finish}
      />
    </div>
  </div>
</div>

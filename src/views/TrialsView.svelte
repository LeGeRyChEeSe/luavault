<script lang="ts">
  // The « À l'essai » tab of the experimental edition: what it adds, each feature with its switch.
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import Icon from "../components/Icons.svelte";
  import ConfirmButton from "../components/ConfirmButton.svelte";
  import { appState } from "../lib/app-state.svelte";
  import { engageKillSwitch, getInSteam, getKillSwitch, releaseKillSwitch, setInSteam, setSteamDiscreet } from "../lib/api";
  import type { InSteam, KillSwitch } from "../lib/api";
  import { t } from "../lib/i18n.svelte";

  let inSteam = $state<InSteam | null>(null);
  let hidden = $state<KillSwitch | null>(null);
  let busy = $state(false);

  onMount(() => {
    void getInSteam()
      .then((v) => (inSteam = v))
      .catch(() => {});
    void getKillSwitch()
      .then((v) => (hidden = v))
      .catch(() => {});
    // Hidden from Steam's own panel while this tab is open.
    const off = listen<KillSwitch>("kill-switch://changed", (e) => (hidden = e.payload));
    return () => void off.then((f) => f());
  });

  async function hide() {
    try {
      hidden = await engageKillSwitch();
      appState.toast("info", t("trials.hide.done"));
    } catch (e) {
      appState.toast("error", String(e));
    }
  }

  async function putBack() {
    try {
      hidden = await releaseKillSwitch();
      appState.toast(hidden.engaged ? "error" : "info", t("trials.hide.released"));
    } catch (e) {
      appState.toast("error", String(e));
    }
  }

  /** Same rules as `toggle`: a refusal puts the box back and reads the state again. */
  async function toggleDiscreet(box: HTMLInputElement) {
    const on = box.checked;
    busy = true;
    try {
      inSteam = await setSteamDiscreet(on);
      appState.toast("info", on ? t("trials.steam.discreet.on") : t("trials.steam.discreet.off"));
    } catch (e) {
      box.checked = !on;
      appState.toast("error", String(e));
      inSteam = await getInSteam().catch(() => inSteam);
    } finally {
      busy = false;
    }
  }

  /** On a refusal the state is read again, never guessed, and the box is put back by hand: when
   *  the value Svelte holds did not change, it does not touch the box the click had ticked. */
  async function toggle(box: HTMLInputElement) {
    const on = box.checked;
    busy = true;
    try {
      inSteam = await setInSteam(on);
      appState.toast("info", on ? t("trials.steam.on") : t("trials.steam.off"));
    } catch (e) {
      box.checked = !on;
      appState.toast("error", String(e));
      inSteam = await getInSteam().catch(() => inSteam);
    } finally {
      busy = false;
    }
  }
</script>

<div class="flex h-full flex-col gap-4 overflow-y-auto" data-trials>
  <section class="glass enter-up rounded-xl2 p-5">
    <h2 class="flex items-center gap-2 text-lg font-semibold">
      <Icon name="sparkle" size={18} />
      {t("trials.title")}
    </h2>
    <p class="mt-1 text-sm text-azure-900/60">{t("trials.intro")}</p>
  </section>

  {#if inSteam && !inSteam.supported}
    <section class="glass enter-up rounded-xl2 p-5 text-sm text-azure-900/70" data-trials-unsupported>
      {t("trials.unsupported")}
    </section>
  {:else if inSteam}
    <section class="glass enter-up flex flex-col gap-3 rounded-xl2 p-5" data-trials-steam>
      <div class="flex flex-wrap items-center gap-2">
        <h3 class="flex items-center gap-2 text-sm font-semibold">
          <Icon name="steam" size={16} />
          {t("trials.steam.title")}
        </h3>
        <span class="rounded-full bg-surface/70 px-2 py-0.5 text-[10px] font-semibold text-azure-900/60 uppercase">
          {t("trials.badge.your_choice")}
        </span>
      </div>
      <p class="text-xs text-azure-900/60">{t("trials.steam.desc")}</p>
      <ul class="ml-4 list-disc text-xs text-azure-900/70">
        <li>{t("trials.steam.item.store")}</li>
        <li>{t("trials.steam.item.follow")}</li>
        <li>{t("trials.steam.item.pill")}</li>
        <li>{t("trials.steam.item.library")}</li>
        <li>{t("trials.steam.item.notice")}</li>
      </ul>
      <div class="flex gap-2 rounded-xl bg-peach-soft/60 px-4 py-3 text-xs text-peach-deep" data-trials-steam-risk>
        <Icon name="alert" size={15} class="mt-0.5 shrink-0" />
        <span>{t("trials.steam.risk")}</span>
      </div>
      <label class="flex cursor-pointer items-center gap-3 rounded-xl bg-surface/55 px-4 py-2.5 text-sm">
        <input
          type="checkbox"
          class="accent-azure-500"
          data-trials-steam-switch
          checked={inSteam.enabled}
          disabled={busy}
          onchange={(e) => void toggle(e.currentTarget)}
        />
        <span class="min-w-0 flex-1">{t("trials.steam.switch")}</span>
      </label>
      <p class="text-xs text-azure-900/45">{t("trials.steam.restart")}</p>
      {#if inSteam.enabled}
        <label class="flex cursor-pointer items-start gap-3 rounded-xl bg-surface/55 px-4 py-2.5 text-sm">
          <input
            type="checkbox"
            class="mt-1 accent-azure-500"
            data-trials-steam-discreet
            checked={inSteam.discreet}
            disabled={busy}
            onchange={(e) => void toggleDiscreet(e.currentTarget)}
          />
          <span class="flex min-w-0 flex-1 flex-col gap-0.5">
            <span>{t("trials.steam.discreet")}</span>
            <span class="text-xs text-azure-900/55">{t("trials.steam.discreet.desc")}</span>
          </span>
        </label>
      {/if}
    </section>
  {/if}

  {#if hidden && (hidden.engaged || inSteam?.enabled)}
    <section class="glass enter-up flex flex-col gap-3 rounded-xl2 p-5" data-trials-hide>
      <h3 class="flex items-center gap-2 text-sm font-semibold">
        <Icon name="shield" size={16} />
        {t("trials.hide.title")}
      </h3>
      <p class="text-xs text-azure-900/60">{t("trials.hide.desc")}</p>
      {#if hidden.engaged}
        <div class="rounded-xl bg-peach-soft/60 px-4 py-3 text-xs text-peach-deep" data-trials-hide-engaged>
          {t("trials.hide.engaged", { luas: hidden.luas, patches: hidden.patches })}
        </div>
        <div>
          <ConfirmButton label={t("trials.hide.release")} confirmLabel={t("trials.hide.release")} primary onconfirm={putBack} />
        </div>
      {:else}
        <div>
          <ConfirmButton label={t("trials.hide.button")} confirmLabel={t("trials.hide.confirm")} onconfirm={hide} />
        </div>
      {/if}
    </section>
  {/if}
</div>

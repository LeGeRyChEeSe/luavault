<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { confirm, open } from "@tauri-apps/plugin-dialog";
  import Icon from "../components/Icons.svelte";
  import ActionButton from "../components/ActionButton.svelte";
  import BulkProgress from "../components/BulkProgress.svelte";
  import ConfirmButton from "../components/ConfirmButton.svelte";
  import ContextMenu from "../components/ContextMenu.svelte";
  import TagEditor from "../components/TagEditor.svelte";
  import GameCard from "./GameCard.svelte";
  import { appState } from "../lib/app-state.svelte";
  import { t } from "../lib/i18n.svelte";
  import { menuItemsFor } from "../lib/game-menu";
  import {
    applyFixesToSelection,
    bulkPreflight,
    cancelBulk,
    copyToSteam,
    importLuaFile,
    importPatchArchive,
    installAllFixes,
    installGameViaSteam,
    readoptIndex,
    repairAllFixes,
    setLibraryHidden,
    setLibraryTags,
    syncLibraryToSteam,
    verifyOnlineFix,
  } from "../lib/api";
  import type { BulkPlan, BulkPlanItem, BulkProgressEvent, BulkReport, GameStage, GameStatus } from "../lib/api";
  import {
    libraryState,
    sortStatuses,
    allTags,
    filterByTags,
    eligibleForSelectionAction,
    purgeSelection,
    selectAllVisible,
    deselectVisible,
    withAddedTag,
    withRemovedTag,
  } from "../lib/library-state.svelte";
  import type { LibraryFilter, SelectionAction } from "../lib/library-state.svelte";
  import { virtualWindow } from "../lib/virtual-scroll";
  import { isPatchArchivePath, patchAppIdFromFilename, patchFailureFor } from "../lib/patch-import";
  import { openFolder } from "../lib/open-folder";

  const statuses = $derived(appState.statuses);
  const report = $derived(appState.report);
  /** Hidden games stay in the library but disappear from this view. */
  const visible = $derived(statuses.filter((s) => !s.hidden));

  /** Tag editor state. */
  let tagEditor = $state<{ status: GameStatus; trigger: HTMLElement | null } | null>(null);

  let busy = $state<string | null>(null);
  let isDraggingFiles = $state(false);

  /** Persisted across navigation — imported from the shared store. */
  const matches = $derived(
    visible.filter((s) => {
      const q = libraryState.search.trim().toLowerCase();
      if (!q) return true;
      return s.name.toLowerCase().includes(q) || s.app_id.includes(q);
    }),
  );

  const ATTENTION: GameStage[] = [
    "lua_not_in_steam",
    "needs_steam_install",
    "fix_damaged",
    "fix_game_moved",
  ];

  const tagged = $derived(filterByTags(matches, libraryState.selectedTags));
  const counts = $derived({
    all: tagged.length,
    attention: tagged.filter((s) => ATTENTION.includes(s.stage)).length,
    fix: tagged.filter((s) => s.fix_downloaded || s.stage === "fix_installed" || s.stage === "fix_damaged" || s.stage === "fix_game_moved").length,
    ready: tagged.filter((s) => s.stage === "ready" || s.stage === "fix_installed").length,
  });
  const shownTags = $derived(tagged.filter((s) => {
    if (libraryState.filter === "attention") return ATTENTION.includes(s.stage);
    if (libraryState.filter === "fix") return s.fix_downloaded || s.stage === "fix_installed" || s.stage === "fix_damaged" || s.stage === "fix_game_moved";
    if (libraryState.filter === "ready") return s.stage === "ready" || s.stage === "fix_installed";
    return true;
  }));

  const shown = $derived(
    sortStatuses(shownTags, libraryState.sort),
  );

  // ------------------------------------------------------- context menu
  /** Rendered at the view root: inside a card, the frosted-glass backdrop-filter
      would trap a fixed menu and clip it. */
  let ctxMenu = $state<{ status: GameStatus; x: number; y: number; trigger: HTMLElement } | null>(null);

  function openContextMenu(status: GameStatus, x: number, y: number, trigger: HTMLElement) {
    ctxMenu = { status, x, y, trigger };
  }

  function closeContextMenu() {
    ctxMenu = null;
  }

  /** Open the context menu — unified for right-click and kebab button.
   *  Captures the stable trigger element (card or kebab button) at click time
   *  so TagEditor restores focus to a live element, not a transient menu item. */
  function onCardContextMenu(status: GameStatus, event: MouseEvent) {
    if (busy !== null) return; // refuse to open while an action is running
    event.preventDefault();
    event.stopPropagation();
    if (libraryState.selectionMode) {
      // Right-click selects like left-click — the menu is a per-game tool,
      // it has no place while the view is in selection mode.
      toggleSelect(status.app_id);
      return;
    }
    openContextMenu(status, event.clientX, event.clientY, event.currentTarget as HTMLElement);
  }

  function onKebabClick(status: GameStatus, event: MouseEvent) {
    if (busy !== null) return; // refuse to open while an action is running
    event.stopPropagation();
    const trigger = event.currentTarget as HTMLElement;
    const rect = trigger.getBoundingClientRect();
    openContextMenu(status, rect.right, rect.bottom, trigger);
  }

  async function executeAction(item: { id: string; run: () => Promise<string | null> }) {
    if (item.id === "edit-tags") {
      const status = ctxMenu?.status;
      const trigger = ctxMenu?.trigger ?? null;
      if (status) {
        tagEditor = { status, trigger };
        ctxMenu = null;
      }
      return;
    }
    busy = item.id;
    try {
      const msg = await item.run();
      await appState.refreshLibrary();
      if (msg) appState.toast("success", msg);
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  /** Available tag pills — shown only when at least one tag exists. */
  const tagList = $derived(allTags(visible));

  /** Purge selectedTags of tags that no longer exist in tagList. */
  $effect(() => {
    const available = new Set(tagList);
    const kept = libraryState.selectedTags.filter((t) => available.has(t));
    if (kept.length === libraryState.selectedTags.length) return; // rien à purger
    libraryState.selectedTags = kept;
  });

  /** LOT-16 — purge the selection of AppIDs that left the library, the
      mirror image of the purge above. Hidden games keep their place: they
      are still in the library, only out of sight, and every action count
      already excludes them (it runs on what the view shows). */
  $effect(() => {
    const kept = purgeSelection(libraryState.selection, statuses);
    if (kept.length === libraryState.selection.length) return; // rien à purger
    libraryState.selection = kept;
  });

  function toggleTag(tag: string) {
    if (libraryState.selectedTags.includes(tag)) {
      libraryState.selectedTags = libraryState.selectedTags.filter((t) => t !== tag);
    } else {
      libraryState.selectedTags = [...libraryState.selectedTags, tag];
    }
  }

  function clearTagFilters() {
    libraryState.selectedTags = [];
  }

  const FILTERS: { id: LibraryFilter; icon: string }[] = [
    { id: "all", icon: "library" },
    { id: "attention", icon: "alert" },
    { id: "fix", icon: "globe" },
    { id: "ready", icon: "check" },
  ];

  async function run(id: string, action: () => Promise<string>) {
    busy = id;
    try {
      appState.toast("success", await action());
      await appState.refreshLibrary();
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      busy = null;
    }
  }

  async function importLuaPaths(paths: string[]) {
    const luaPaths = paths.filter((path) => path.toLowerCase().endsWith(".lua"));
    const rejected = paths.filter((path) => !path.toLowerCase().endsWith(".lua"));
    if (rejected.length > 0) {
      appState.toast("warning", t("library.import.ignored", { names: summarizedNames(rejected) }));
    }
    if (luaPaths.length === 0) return;

    busy = "import";
    const imported = [] as Awaited<ReturnType<typeof importLuaFile>>[];
    const failed: string[] = [];
    try {
      for (const path of luaPaths) {
        // Import serially: each write signs the shared library index.
        try {
          imported.push(await importLuaFile(path));
        } catch {
          failed.push(path);
        }
      }

      if (imported.length > 0) {
        const differentNames = imported
          .filter((result) => result.filename_differs)
          .map((result) => result.entry.name);
        appState.toast(
          differentNames.length > 0 ? "info" : "success",
          differentNames.length > 0
            ? t("library.import.done.mismatch", { count: imported.length, names: summarizedNames(differentNames) })
            : t("library.import.done", { count: imported.length }),
        );
      }
      if (failed.length > 0) {
        appState.toast("error", t("library.import.failed", { names: summarizedNames(failed) }));
      }
    } finally {
      try {
        await appState.refreshLibrary();
      } catch (e) {
        appState.toast("error", String(e));
      }
      busy = null;
    }
  }

  function summarizedNames(paths: string[]) {
    const names = paths.map((path) => path.split(/[\\/]/).pop() ?? path);
    const shown = names.slice(0, 3).join(", ");
    return names.length > 3
      ? `${shown} (${t("library.import.more", { count: names.length - 3 })})`
      : shown;
  }

  async function pickLuaFiles() {
    const selected = await open({
      title: t("library.import.dialog.title"),
      multiple: true,
      filters: [{ name: t("library.import.dialog.filter"), extensions: ["lua"] }],
    });
    const paths = Array.isArray(selected) ? selected : typeof selected === "string" ? [selected] : [];
    await importLuaPaths(paths);
  }

  async function importPatchPaths(paths: string[]) {
    const patchPaths = paths.filter(isPatchArchivePath);
    if (patchPaths.length === 0) return;

    busy = "import-patch";
    const imported: { entry: GameStatus }[] = [];
    const failures: string[] = [];
    try {
      for (const path of patchPaths) {
        const appId = patchAppIdFromFilename(path);
        if (!appId) {
          failures.push(t("library.patch.failed.no-app-id", { name: path.split(/[\\/]/).pop() ?? path }));
          continue;
        }
        const entry = statuses.find((status) => status.app_id === appId);
        if (!entry) {
          failures.push(t("library.patch.failed.unknown-app-id", { appId }));
          continue;
        }
        const accepted = await confirm(
          t("library.patch.confirm", { name: entry.name, appId: entry.app_id }),
          {
            title: t("library.patch.confirm.title"),
            kind: "warning",
            okLabel: t("library.patch.confirm.ok"),
            cancelLabel: t("library.patch.confirm.cancel"),
          },
        );
        if (!accepted) continue;
        try {
          await importPatchArchive(path, entry.app_id);
          imported.push({ entry });
        } catch (error) {
          const failure = patchFailureFor(error, summarizedNames([path]));
          failures.push(t(failure.key, failure.params));
        }
      }

      if (imported.length > 0) {
        appState.toast(
          "success",
          imported.length === 1
            ? t("library.patch.done", { name: imported[0].entry.name })
            : t("library.patch.done.many", { count: imported.length }),
        );
      }
      if (failures.length > 0) {
        appState.toast("error", failures.join("\n"));
      }
    } finally {
      try {
        await appState.refreshLibrary();
      } catch (e) {
        appState.toast("error", String(e));
      }
      busy = null;
    }
  }

  async function pickPatchFiles() {
    const selected = await open({
      title: t("library.patch.dialog.title"),
      multiple: true,
      filters: [{ name: t("library.patch.dialog.filter"), extensions: ["zip", "rar", "7z"] }],
    });
    const paths = Array.isArray(selected) ? selected : typeof selected === "string" ? [selected] : [];
    await importPatchPaths(paths);
  }

  async function importDroppedPaths(paths: string[]) {
    await importLuaPaths(paths.filter((path) => path.toLowerCase().endsWith(".lua")));
    await importPatchPaths(paths.filter(isPatchArchivePath));
  }

  onMount(() => {
    let unlistenDrop: (() => void) | undefined;
    let unlistenEnter: (() => void) | undefined;
    let unlistenLeave: (() => void) | undefined;
    void getCurrentWindow()
      .listen<{ paths?: string[] }>("tauri://drag-drop", (event) => {
        isDraggingFiles = false;
        void importDroppedPaths(event.payload.paths ?? []);
      })
      .then((unlisten) => (unlistenDrop = unlisten));
    void getCurrentWindow()
      .listen("tauri://drag-enter", () => (isDraggingFiles = true))
      .then((unlisten) => (unlistenEnter = unlisten));
    void getCurrentWindow()
      .listen("tauri://drag-leave", () => (isDraggingFiles = false))
      .then((unlisten) => (unlistenLeave = unlisten));
    return () => {
      unlistenDrop?.();
      unlistenEnter?.();
      unlistenLeave?.();
    };
  });

  const syncAll = () =>
    run("sync", async () => {
      const count = await syncLibraryToSteam();
      return t("library.sync.done", { count });
    });

  // ------------------------------------------------------- bulk install flow
  type BulkMode = "games" | "fixes" | "repair" | "all" | "selection";

  let bulkPlan = $state<BulkPlan | null>(null);
  let bulkPhase = $state<"confirm" | "running" | "done" | null>(null);
  let bulkMode = $state<BulkMode>("all");
  let bulkProgress = $state<BulkProgressEvent[]>([]);
  let bulkReport = $state<BulkReport | null>(null);
  let unlistenProgress: (() => void) | null = null;
  /** True while the games phase waits for the user to click "Jeu suivant". */
  let waitingForNext = $state(false);
  let bulkCancelled = false;
  let nextResolver: (() => void) | null = null;

  // ------------------------------------------------------- selection (LOT-16)
  /** Which selection action is about to run / running. */
  let selectionAction = $state<SelectionAction>("fixes");
  /** The tag for add/remove-tag actions, captured at click time. */
  let selectionRunTag = $state("");
  /** Tag being typed in the bar. */
  let selectionTagInput = $state("");
  /** Targets captured at click time — nothing invisible at that moment
      enters the pass, and the plan shows exactly this list. */
  let selectionRunTargets = $state<GameStatus[]>([]);

  /** Selected AND on screen right now. A selection a filter hides stays
      inert — never counted, never treated, never a lie on a button. */
  const selectedVisible = $derived(
    shown.filter((s) => libraryState.selection.includes(s.app_id)),
  );

  /** How many games each selection button would actually act on. The
      counts come from the same eligibility the passes run on — the bar
      never announces a game the pass won't treat. */
  const selectionCounts = $derived({
    fixes: eligibleForSelectionAction(selectedVisible, "fixes").length,
    verify: eligibleForSelectionAction(selectedVisible, "verify").length,
    copy: eligibleForSelectionAction(selectedVisible, "copy").length,
    addTag: eligibleForSelectionAction(selectedVisible, "add-tag", selectionTagInput).length,
    removeTag: eligibleForSelectionAction(selectedVisible, "remove-tag", selectionTagInput).length,
    hide: eligibleForSelectionAction(selectedVisible, "hide").length,
  });

  /** Selected games the current filter hides — named, so the gap between
      "10 sélectionnés" and the buttons' counts is never a mystery. */
  const selectedOutOfFilter = $derived(
    libraryState.selection.length - selectedVisible.length,
  );

  /** Le nom d'action backend de chaque mode de sélection. Les phrases, elles,
      vivent dans les catalogues sous `library.sel.<action>.*` : une const de
      module est évaluée au montage, et un libellé posé ici ne suivrait pas un
      changement de langue. */
  const SELECTION_ACTIONS: Record<SelectionAction, BulkPlanItem["action"]> = {
    fixes: "install_fix",
    verify: "verify_fix",
    copy: "copy_lua",
    "add-tag": "add_tag",
    "remove-tag": "remove_tag",
    hide: "hide",
  };

  /** Wording handed to the overlay while a selection action runs. */
  const selectionWording = $derived(
    bulkMode === "selection"
      ? {
          confirm: t(`library.sel.${selectionAction}.confirm`),
          running: t(`library.sel.${selectionAction}.running`),
          done: t(`library.sel.${selectionAction}.done`),
          interrupted: t(`library.sel.${selectionAction}.interrupted`),
          ok: t(`library.sel.${selectionAction}.ok`),
        }
      : undefined,
  );
  const selectionSectionLabel = $derived(
    bulkMode === "selection"
      ? t(`library.sel.${selectionAction}.section`) +
          (selectionRunTag ? t("library.sel.tag-suffix", { tag: selectionRunTag }) : "")
      : "",
  );

  function toggleSelectionMode() {
    libraryState.selectionMode = !libraryState.selectionMode;
    // Leaving the mode closes the gesture: a selection with no checkboxes on
    // screen would be the invisible-selection trap again.
    if (!libraryState.selectionMode) libraryState.selection = [];
  }

  function toggleSelect(appId: string) {
    libraryState.selection = libraryState.selection.includes(appId)
      ? libraryState.selection.filter((id) => id !== appId)
      : [...libraryState.selection, appId];
  }

  function selectAllShown() {
    libraryState.selection = selectAllVisible(shown, libraryState.selection);
  }

  function deselectAllShown() {
    libraryState.selection = deselectVisible(shown, libraryState.selection);
  }

  /** Fifth mode — capture the targets at click time, then open the usual
      confirm → running → done flow. The patch action asks the backend for
      its plan (the pass's own predicate); the local actions build theirs
      from the same eligibility the buttons count. */
  async function startSelection(action: SelectionAction) {
    const tag = selectionTagInput.trim();
    const targets = eligibleForSelectionAction(selectedVisible, action, tag);
    if (targets.length === 0) return;
    selectionAction = action;
    selectionRunTag = tag;
    selectionRunTargets = targets;
    bulkMode = "selection";
    bulkProgress = [];
    bulkReport = null;
    bulkCancelled = false;

    if (action === "fixes") {
      try {
        bulkPlan = await bulkPreflight(false, targets.map((t) => t.app_id));
        bulkPhase = "confirm";
      } catch (e) {
        appState.toast("error", t("library.preflight.error", { error: String(e) }));
      }
      return;
    }

    bulkPlan = {
      steam_detected: true,
      steam_running: true,
      games: [],
      fixes: [],
      selection: targets.map((target) => ({
        app_id: target.app_id,
        name: target.name,
        action: SELECTION_ACTIONS[action],
        label: t(`library.sel.${action}.label`) + (tag ? t("library.sel.tag-suffix", { tag }) : ""),
        warning: null,
      })),
      warnings: [],
    };
    bulkPhase = "confirm";
  }

  /** Filter the plan to match which button was clicked. */
  const filteredPlan = $derived.by((): BulkPlan => {
    if (!bulkPlan) return { steam_detected: false, steam_running: false, games: [], fixes: [], selection: [], warnings: [] };
    // The fifth mode's plan already covers exactly the chosen action.
    if (bulkMode === "selection") return bulkPlan;
    return {
      ...bulkPlan,
      games: bulkMode === "fixes" || bulkMode === "repair" ? [] : bulkPlan.games,
      fixes: bulkMode === "games" ? [] : bulkPlan.fixes,
    };
  });

  /** How many games each bulk button would actually act on. The counts
      mirror the backend's selection exactly — same stages, same
      fix_downloaded + fully_installed conditions, and hidden games excluded
      on BOTH sides (the passes skip index entries marked hidden) — so the
      number on a button is exactly the number of games the pass will treat. */
  const pending = $derived({
    games: visible.filter((s) => s.stage === "needs_steam_install" || s.stage === "lua_not_in_steam")
      .length,
    fixes: visible.filter(
      (s) =>
        s.game.fully_installed &&
        ["fix_downloaded", "fix_damaged", "fix_game_moved"].includes(s.stage),
    ).length,
    repair: visible.filter(
      (s) =>
        s.game.fully_installed &&
        ["fix_damaged", "fix_game_moved"].includes(s.stage),
    ).length,
    /** Repairable games whose archive is already in the library — the ones
        a repair can treat while offline. */
    repair_offline_ready: visible.filter(
      (s) =>
        s.game.fully_installed &&
        s.fix_downloaded &&
        ["fix_damaged", "fix_game_moved"].includes(s.stage),
    ).length,
  });

  async function startBulk(mode: BulkMode) {
    bulkMode = mode;
    bulkProgress = [];
    bulkReport = null;
    bulkCancelled = false;
    try {
      // A repair confirms only the broken installs — the same selection the
      // pass itself runs on.
      bulkPlan = await bulkPreflight(mode === "repair");
      bulkPhase = "confirm";
    } catch (e) {
      appState.toast("error", t("library.preflight.error", { error: String(e) }));
    }
  }

  function pushEvent(ev: BulkProgressEvent) {
    bulkProgress = [...bulkProgress, ev];
  }

  /** Wait until the user clicks "Jeu suivant". */
  function waitForNext(): Promise<void> {
    return new Promise((resolve) => {
      nextResolver = resolve;
    });
  }

  function onNext() {
    waitingForNext = false;
    nextResolver?.();
    nextResolver = null;
  }

  /** Games phase: one at a time, the user validates in Steam then clicks "next". */
  async function runGamesOneByOne(games: BulkPlanItem[]) {
    const total = games.length;
    for (let i = 0; i < total; i++) {
      if (bulkCancelled) break;
      const game = games[i];

      if (game.action === "copy_lua") {
        pushEvent({ phase: "games", current: i + 1, total, app_id: game.app_id, name: game.name, status: "working", detail: t("library.games.copy-lua"), cancelled: false });
        try {
          await copyToSteam(game.app_id);
        } catch { /* non-fatal: the install URI still works */ }
      }

      pushEvent({ phase: "games", current: i + 1, total, app_id: game.app_id, name: game.name, status: "working", detail: t("library.games.requested"), cancelled: false });
      try {
        await installGameViaSteam(game.app_id);
        pushEvent({ phase: "games", current: i + 1, total, app_id: game.app_id, name: game.name, status: "ok", detail: t("library.games.asked"), cancelled: false });
      } catch (e) {
        pushEvent({ phase: "games", current: i + 1, total, app_id: game.app_id, name: game.name, status: "error", detail: String(e), cancelled: false });
      }

      // Wait for the user before firing the next URI — Steam dialogs replace each other.
      if (i < total - 1 && !bulkCancelled) {
        waitingForNext = true;
        await waitForNext();
      }
    }
  }

  /** Fixes phase: backend-driven with progress events (no external dialogs).
      `launch` picks the pass — install every patchable fix, or repair only
      the broken ones; progress and cancellation are shared. */
  async function runFixesWithEvents(launch: () => Promise<BulkReport>) {
    const off = await listen<BulkProgressEvent>("bulk://progress", (event) => {
      pushEvent(event.payload);
    });
    unlistenProgress = off;
    try {
      const r = await launch();
      return r;
    } finally {
      unlistenProgress?.();
      unlistenProgress = null;
    }
  }

  async function confirmBulk() {
    if (!bulkPlan) return;
    bulkPhase = "running";
    bulkProgress = [];
    bulkCancelled = false;

    const games = filteredPlan.games;
    const fixes = filteredPlan.fixes;
    const report: BulkReport = { items: [], succeeded: 0, failed: 0, skipped: 0 };

    try {
      if (bulkMode === "selection" && selectionAction !== "fixes") {
        // Local selection actions: the view drives, one game at a time,
        // through the same events / cancellation / report machinery.
        await runSelectionLocal();
      } else {
        // Phase 1 — games: one at a time, user controls the pace.
        if (games.length > 0) {
          await runGamesOneByOne(games);
        }

        // Phase 2 — fixes: automatic, backend emits progress events.
        if (fixes.length > 0 && !bulkCancelled) {
          const launch =
            bulkMode === "selection"
              ? () => applyFixesToSelection(fixes.map((f) => f.app_id))
              : bulkMode === "repair" ? repairAllFixes : installAllFixes;
          const fixReport = await runFixesWithEvents(launch);
          report.items.push(...fixReport.items);
          report.succeeded += fixReport.succeeded;
          report.failed += fixReport.failed;
          report.skipped += fixReport.skipped;
        }
      }

      // Count game + local-selection results from the progress events.
      for (const ev of bulkProgress.filter(
        (e) => (e.phase === "games" || e.phase === "selection") && e.status !== "working",
      )) {
        if (ev.status === "ok") report.succeeded++;
        else if (ev.status === "error") report.failed++;
        else report.skipped++;
      }

      bulkReport = report;
    } catch (e) {
      appState.toast("error", String(e));
    } finally {
      waitingForNext = false;
      bulkPhase = "done";
      await appState.refreshLibrary();
      // Hidden games left the visible world: drop them from the selection
      // so the bar reflects what is still actionable.
      if (bulkMode === "selection" && selectionAction === "hide") {
        const hiddenIds = new Set(selectionRunTargets.map((t) => t.app_id));
        libraryState.selection = libraryState.selection.filter((id) => !hiddenIds.has(id));
      }
    }
  }

  /** Local selection actions (verify, copy, tag, hide): the view drives
      game by game — the same progress events, cancellation flag and report
      assembly as the games phase, no second machinery. */
  async function runSelectionLocal() {
    const targets = selectionRunTargets;
    const total = targets.length;
    for (let i = 0; i < total; i++) {
      const target = targets[i];
      if (bulkCancelled) {
        pushEvent({ phase: "selection", current: i + 1, total, app_id: target.app_id, name: target.name, status: "skipped", detail: t("library.run.cancelled"), cancelled: true });
        break;
      }
      pushEvent({ phase: "selection", current: i + 1, total, app_id: target.app_id, name: target.name, status: "working", detail: t(`library.sel.${selectionAction}.working`), cancelled: false });
      try {
        const detail = await runSelectionOp(target);
        pushEvent({ phase: "selection", current: i + 1, total, app_id: target.app_id, name: target.name, status: "ok", detail, cancelled: false });
      } catch (e) {
        pushEvent({ phase: "selection", current: i + 1, total, app_id: target.app_id, name: target.name, status: "error", detail: String(e), cancelled: false });
      }
    }
  }

  /** One game of a local selection action. Throws on failure so the event
      — and the final count — record it as an échec. */
  async function runSelectionOp(status: GameStatus): Promise<string> {
    switch (selectionAction) {
      case "verify": {
        const r = await verifyOnlineFix(status.app_id);
        if (r.health === "healthy") {
          return t("library.run.verify.ok", { count: r.file_count });
        }
        throw new Error(
          t("library.run.verify.failed", { missing: r.missing.length, modified: r.modified.length }),
        );
      }
      case "copy":
        await copyToSteam(status.app_id);
        return t("library.run.copy.ok");
      case "add-tag":
        await setLibraryTags(status.app_id, withAddedTag(status.tags, selectionRunTag));
        return t("library.run.add-tag.ok", { tag: selectionRunTag });
      case "remove-tag":
        await setLibraryTags(status.app_id, withRemovedTag(status.tags, selectionRunTag));
        return t("library.run.remove-tag.ok", { tag: selectionRunTag });
      case "hide":
        await setLibraryHidden(status.app_id, true);
        return t("library.run.hide.ok");
      default:
        throw new Error(t("library.run.unexpected"));
    }
  }

  function cancelBulkRun() {
    bulkCancelled = true;
    void cancelBulk();
    // Unblock the games loop if it's waiting for a "next" click.
    onNext();
  }

  function closeBulk() {
    bulkPhase = null;
    bulkPlan = null;
    bulkProgress = [];
    bulkReport = null;
    waitingForNext = false;
    bulkCancelled = false;
    nextResolver = null;
    unlistenProgress?.();
    unlistenProgress = null;
  }

  /** Label for the "next" button: last game → "Terminer", otherwise → "Jeu suivant". */
  const nextLabel = $derived.by(() => {
    const games = filteredPlan.games;
    const done = bulkProgress.filter((e) => e.phase === "games" && e.status !== "working").length;
    if (done >= games.length - 1 && filteredPlan.fixes.length === 0) return t("library.next.finish");
    if (done >= games.length - 1) return t("library.next.fixes");
    return t("library.next.game");
  });

  // Pick up .lua files dropped into the library folder while the app is open.
  onMount(() => void appState.adoptFromSteam(false));

  // ------------------------------------------------------- virtual scrolling
  /** Below this many cards the grid renders normally — no virtualisation cost. */
  const VIRTUAL_THRESHOLD = 100;
  const OVERSCAN_ROWS = 5;
  /** Card height + gap-3 (12 px) — refined by measurement after mount. */
  const ESTIMATED_ROW_HEIGHT = 170;

  let scrollEl = $state<HTMLElement | null>(null);
  let vScrollTop = $state(0);
  let vViewportH = $state(0);
  let vRowH = $state(ESTIMATED_ROW_HEIGHT);
  let vCols = $state(3);

  const useVirtual = $derived(shown.length > VIRTUAL_THRESHOLD);

  const vw = $derived(
    virtualWindow(vScrollTop, vViewportH, vRowH, shown.length, vCols, OVERSCAN_ROWS),
  );

  const visibleSlice = $derived(useVirtual ? shown.slice(vw.startIndex, vw.endIndex) : shown);

  const gridPadding = $derived(
    useVirtual ? `padding-top:${vw.offsetTop}px;padding-bottom:${vw.offsetBottom}px;` : "",
  );

  function onGridScroll() {
    // Below the threshold the grid renders normally — rewriting vScrollTop on
    // every frame would make vw recompute for nothing.
    if (!useVirtual) return;
    if (scrollEl) vScrollTop = scrollEl.scrollTop;
  }

  function measureCols() {
    // Tailwind's max-lg / max-xl variants switch at 1023.98 / 1279.98 px —
    // matchMedia must use the same bounds, or fractional viewport widths
    // (Windows scaling at 125%) get a vCols the CSS grid contradicts.
    if (window.matchMedia("(max-width: 1023.98px)").matches) vCols = 1;
    else if (window.matchMedia("(max-width: 1279.98px)").matches) vCols = 2;
    else vCols = 3;
  }

  function measureRowHeight() {
    if (!scrollEl) return;
    vViewportH = scrollEl.clientHeight;
    const grid = scrollEl.querySelector("[data-vgrid]");
    if (!grid || grid.children.length <= vCols) return;
    const first = grid.children[0] as HTMLElement;
    const nextRow = grid.children[vCols] as HTMLElement;
    if (first && nextRow) {
      const stride = nextRow.offsetTop - first.offsetTop;
      if (stride > 0) vRowH = stride;
    }
  }

  // The scroll container lives in the last branch of the {#if} chain: it is
  // destroyed and recreated whenever the list goes through zero results, and
  // could not exist yet at mount time (search state persists across
  // navigation). The instrumentation therefore follows the node through a
  // $effect instead of being installed once in onMount, and only above the
  // virtualisation threshold — below it the grid renders normally and none of
  // this may cost anything.
  $effect(() => {
    const el = scrollEl;
    if (!el || !useVirtual) return;
    vScrollTop = el.scrollTop;
    measureCols();
    measureRowHeight();
    const ro = new ResizeObserver(() => measureRowHeight());
    ro.observe(el);
    return () => ro.disconnect();
  });

  $effect(() => {
    if (!useVirtual) return;
    const mqLg = window.matchMedia("(max-width: 1023.98px)");
    const mqXl = window.matchMedia("(max-width: 1279.98px)");
    const onBreakpoint = () => {
      measureCols();
      measureRowHeight();
    };
    mqLg.addEventListener("change", onBreakpoint);
    mqXl.addEventListener("change", onBreakpoint);
    return () => {
      mqLg.removeEventListener("change", onBreakpoint);
      mqXl.removeEventListener("change", onBreakpoint);
    };
  });

  // Selection mode replaces the per-card action row with a hint line, so
  // the grid's stride changes — remeasure it for the virtual window.
  $effect(() => {
    libraryState.selectionMode;
    if (useVirtual) measureRowHeight();
  });
</script>

<div class="flex h-full flex-col gap-4 p-1 {isDraggingFiles ? 'ring-2 ring-inset ring-sky-400 bg-sky-soft/30' : ''}">
  <header class="glass enter-up flex flex-col gap-3 rounded-xl2 p-5">
    <div class="flex flex-wrap items-center gap-3">
      <div class="min-w-0 flex-1">
        <h2 class="flex items-center gap-2 text-lg font-semibold">
          <Icon name="library" size={20} />
          {t("library.title")}
        </h2>
        <p class="mt-0.5 truncate text-sm text-azure-900/60">
          {t("library.count", { n: matches.length })}
          {#if libraryState.search.trim() && matches.length !== visible.length}
            · {t("library.count.filtered", { total: visible.length })}
          {/if}
          {#if counts.attention > 0}
            · <span class="font-semibold text-peach-deep">{t("library.count.attention", { n: counts.attention })}</span>
          {/if}
          · {report?.library_dir ?? "…"}
        </p>
      </div>
      <ActionButton
        label={t("library.open-folder")}
        icon="folder"
        onclick={() => report && void openFolder(report.library_dir)}
        tip={t("library.open-folder.tip")}
      />
      <ActionButton
        label={t("library.import")}
        icon="plus"
        variant="primary"
        busy={busy === "import"}
        busyLabel={t("library.import.busy")}
        disabled={busy !== null}
        onclick={() => void pickLuaFiles()}
        tip={t("library.import.tip")}
      />
      <ActionButton
        label={t("library.patch.import")}
        icon="patch"
        busy={busy === "import-patch"}
        busyLabel={t("library.patch.import.busy")}
        disabled={busy !== null}
        onclick={() => void pickPatchFiles()}
        tip={t("library.patch.import.tip")}
      />
      <ActionButton
        label={t("library.refresh")}
        icon="refresh"
        busy={busy === "refresh"}
        busyLabel={t("library.refresh.busy")}
        disabled={busy !== null}
        onclick={() =>
          run("refresh", async () => {
            const r = await appState.adoptFromSteam(false);
            return r && r.imported.length > 0
              ? t("library.refresh.found", { n: r.imported.length })
              : t("library.refresh.uptodate");
          })}
        tip={t("library.refresh.tip")}
      />
      <ActionButton
        label={t("library.sync")}
        icon="copy"
        variant="primary"
        busy={busy === "sync"}
        busyLabel={t("library.sync.busy")}
        disabled={busy !== null || visible.length === 0}
        onclick={syncAll}
        tip={t("library.sync.tip")}
      />
    </div>

    {#if visible.length > 0}
      <div class="flex flex-wrap gap-2">
        <div class="relative min-w-0 flex-1">
          <Icon
            name="search"
            size={15}
            class="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 opacity-50"
          />
          <input
            type="text"
            bind:value={libraryState.search}
            placeholder={t("library.search.placeholder")}
            class="w-full rounded-xl border border-surface/70 bg-surface/60 py-2 pr-9 pl-9 text-sm outline-none backdrop-blur-md transition select-text placeholder:text-azure-900/40 focus:border-azure-300 focus:bg-surface/85 focus:ring-2 focus:ring-azure-300/40"
          />
          {#if libraryState.search}
            <button
              onclick={() => (libraryState.search = "")}
              aria-label={t("library.search.clear-aria")}
              class="lift absolute top-1/2 right-2 -translate-y-1/2 rounded-md p-1 text-azure-900/45 hover:bg-surface/70 hover:text-azure-900/70"
            >
              <Icon name="x" size={13} />
            </button>
          {/if}
        </div>
        <div class="relative shrink-0 basis-64">
          <Icon
            name="arrow-up-down"
            size={15}
            class="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 opacity-50"
          />
          <select
            bind:value={libraryState.sort}
            aria-label={t("library.sort.aria")}
            data-tip={t("library.sort.tip")}
            class="w-full rounded-xl border border-surface/70 bg-surface/60 py-2 pl-9 pr-7 text-sm outline-none backdrop-blur-md transition focus:border-azure-300 focus:bg-surface/85 focus:ring-2 focus:ring-azure-300/40"
          >
            <option value="added_desc">{t("library.sort.added_desc")}</option>
            <option value="added_asc">{t("library.sort.added_asc")}</option>
            <option value="name_asc">{t("library.sort.name_asc")}</option>
            <option value="name_desc">{t("library.sort.name_desc")}</option>
            <option value="stage">{t("library.sort.stage")}</option>
            <option value="fix_first">{t("library.sort.fix_first")}</option>
            <option value="playtime">{t("library.sort.playtime")}</option>
          </select>
        </div>
      </div>
    {/if}
  </header>

  {#if visible.length > 0}
    <!-- Bulk lane. Counts come from the same stages the cards use, so a button
         that says 0 is a button that would do nothing. -->
    <div class="glass enter-up flex flex-wrap items-center gap-2 rounded-xl2 px-4 py-3">
      <span class="mr-1 flex items-center gap-2 text-xs font-bold tracking-wide text-azure-900/45 uppercase">
        <Icon name="layers" size={15} />
        {t("library.bulk.title")}
      </span>
      <ActionButton
        label={t("library.bulk.games", { n: pending.games })}
        icon="steam"
        size="sm"
        disabled={busy !== null || bulkPhase !== null || pending.games === 0}
        onclick={() => void startBulk("games")}
        tip={pending.games === 0
          ? t("library.bulk.games.tip.none")
          : t("library.bulk.games.tip")}
      />
      <ActionButton
        label={t("library.bulk.fixes", { n: pending.fixes })}
        icon="patch"
        size="sm"
        disabled={busy !== null || bulkPhase !== null || pending.fixes === 0 || !appState.online}
        onclick={() => void startBulk("fixes")}
        tip={!appState.online
          ? t("library.bulk.offline.tip")
          : pending.fixes === 0
            ? t("library.bulk.fixes.tip.none")
            : t("library.bulk.fixes.tip")}
      />
      <!-- Exists only when something is broken: an action that would do
           nothing has no place on screen. -->
      {#if pending.repair > 0}
        <ActionButton
          label={t("library.bulk.repair", { n: pending.repair })}
          icon="wrench"
          variant="danger"
          size="sm"
          disabled={busy !== null || bulkPhase !== null}
          onclick={() => void startBulk("repair")}
          tip={!appState.online
            ? t("library.bulk.repair.offline.tip", { n: pending.repair_offline_ready })
            : t("library.bulk.repair.tip")}
        />
      {/if}
      <ActionButton
        label={t("library.bulk.all")}
        icon="sparkle"
        variant="primary"
        size="sm"
        disabled={busy !== null || bulkPhase !== null || (pending.games === 0 && pending.fixes === 0) || !appState.online}
        onclick={() => void startBulk("all")}
        tip={!appState.online
          ? t("library.bulk.offline.tip")
          : t("library.bulk.all.tip")}
      />
      <ActionButton
        label={libraryState.selectionMode ? t("library.bulk.select.on") : t("library.bulk.select.off")}
        icon="select"
        size="sm"
        disabled={busy !== null || bulkPhase !== null}
        onclick={toggleSelectionMode}
        tip={libraryState.selectionMode
          ? t("library.bulk.select.on.tip")
          : t("library.bulk.select.off.tip")}
      />
    </div>
  {/if}

  {#if libraryState.selectionMode}
    <!-- Selection lane — LOT-16. The checkboxes on the cards and this lane
         are one gesture: leaving the mode clears the selection, so no
         invisible selection ever survives to be processed. -->
    <div class="glass enter-up flex flex-col gap-2.5 rounded-xl2 px-4 py-3">
      <div class="flex flex-wrap items-center gap-2">
        <span class="mr-1 flex items-center gap-2 text-xs font-bold tracking-wide text-azure-900/45 uppercase">
          <Icon name="select" size={15} />
          {t("library.multi.title")}
        </span>
        <ActionButton
          label={t("library.multi.select-all", { n: shown.length })}
          icon="select"
          size="sm"
          disabled={busy !== null || bulkPhase !== null || shown.length === 0}
          onclick={selectAllShown}
          tip={t("library.multi.select-all.tip")}
        />
        <ActionButton
          label={t("library.multi.deselect-all")}
          icon="x"
          size="sm"
          disabled={busy !== null || bulkPhase !== null || selectedVisible.length === 0}
          onclick={deselectAllShown}
          tip={t("library.multi.deselect-all.tip")}
        />
        <ActionButton
          label={t("library.multi.quit")}
          icon="eye-off"
          size="sm"
          disabled={busy !== null || bulkPhase !== null}
          onclick={toggleSelectionMode}
          tip={t("library.bulk.select.on.tip")}
        />
        <span class="text-xs font-semibold text-azure-900/60">
          {t("library.multi.count", { n: libraryState.selection.length })}
        </span>
      </div>

      {#if libraryState.selection.length > 0}
        <!-- Actions bar — each button counts exactly the games its pass
             will treat among the selected games on screen. -->
        <div class="flex flex-wrap items-center gap-2">
          <ActionButton
            label={t("library.multi.fixes", { n: selectionCounts.fixes })}
            icon="patch"
            size="sm"
            disabled={busy !== null || bulkPhase !== null || selectionCounts.fixes === 0 || !appState.online}
            onclick={() => void startSelection("fixes")}
            tip={!appState.online
              ? t("library.bulk.offline.tip")
              : t("library.multi.fixes.tip")}
          />
          <ActionButton
            label={t("library.multi.verify", { n: selectionCounts.verify })}
            icon="check"
            size="sm"
            disabled={busy !== null || bulkPhase !== null || selectionCounts.verify === 0}
            onclick={() => void startSelection("verify")}
            tip={t("library.multi.verify.tip")}
          />
          <ActionButton
            label={t("library.multi.copy", { n: selectionCounts.copy })}
            icon="copy"
            size="sm"
            disabled={busy !== null || bulkPhase !== null || selectionCounts.copy === 0}
            onclick={() => void startSelection("copy")}
            tip={t("library.multi.copy.tip")}
          />
          <div class="flex items-center gap-1.5 rounded-xl border border-surface/70 bg-surface/60 px-2.5 py-1.5">
            <Icon name="tag" size={13} class="shrink-0 opacity-50" />
            <input
              type="text"
              bind:value={selectionTagInput}
              placeholder={t("library.multi.tag.placeholder")}
              aria-label={t("library.multi.tag.aria")}
              class="w-24 bg-transparent text-xs outline-none select-text placeholder:text-azure-900/40"
            />
            <ActionButton
              label={t("library.multi.add", { n: selectionCounts.addTag })}
              icon="plus"
              size="sm"
              disabled={busy !== null || bulkPhase !== null || selectionCounts.addTag === 0}
              onclick={() => void startSelection("add-tag")}
              tip={t("library.multi.add.tip")}
            />
            <ActionButton
              label={t("library.multi.remove", { n: selectionCounts.removeTag })}
              icon="tag"
              size="sm"
              disabled={busy !== null || bulkPhase !== null || selectionCounts.removeTag === 0}
              onclick={() => void startSelection("remove-tag")}
              tip={t("library.multi.remove.tip")}
            />
          </div>
          <ActionButton
            label={t("library.multi.hide", { n: selectionCounts.hide })}
            icon="eye-off"
            size="sm"
            disabled={busy !== null || bulkPhase !== null || selectionCounts.hide === 0}
            onclick={() => void startSelection("hide")}
            tip={t("library.multi.hide.tip")}
          />
        </div>
        {#if selectedOutOfFilter > 0}
          <p class="flex items-center gap-1.5 text-xs text-peach-deep">
            <Icon name="info" size={13} class="shrink-0" />
            {t("library.multi.out-of-filter", { n: selectedOutOfFilter })}
          </p>
        {/if}
      {/if}
    </div>
  {/if}

  {#if visible.length > 0}
    <div class="enter-fade flex flex-wrap gap-1.5">
      {#each FILTERS as item (item.id)}
        <button
          onclick={() => (libraryState.filter = item.id)}
          data-tip={t(`library.filter.${item.id}.tip`)}
          class="lift inline-flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs font-semibold {libraryState.filter ===
          item.id
            ? 'border-azure-300/60 bg-surface/85 text-azure-800 shadow-sm'
            : 'border-surface/60 bg-surface/45 text-azure-900/55 hover:bg-surface/70'}"
        >
          <Icon name={item.icon} size={13} />
          {t(`library.filter.${item.id}`)}
          <span
            class="rounded-full bg-azure-500/12 px-1.5 py-px text-[0.65rem] font-bold text-azure-700"
          >
            {counts[item.id]}
          </span>
        </button>
      {/each}
      {#if libraryState.selectedTags.length > 0}
        <button
          onclick={clearTagFilters}
          aria-label={t("library.tags.clear-aria")}
          data-tip={t("library.tags.clear.tip")}
          class="lift inline-flex items-center gap-1 rounded-full border border-rose-soft/60 bg-rose-soft/40 px-2.5 py-1.5 text-xs font-semibold text-rose-deep hover:bg-rose-soft/60"
        >
          <Icon name="x" size={11} />
        </button>
      {/if}
      {#if tagList.length > 0}
        {#each tagList as tag (tag)}
          <button
            onclick={() => toggleTag(tag)}
            aria-pressed={libraryState.selectedTags.includes(tag)}
            data-tip={libraryState.selectedTags.includes(tag)
              ? t("library.tags.remove.tip")
              : t("library.tags.add.tip")}
            class="lift inline-flex items-center gap-1 rounded-full border px-3 py-1.5 text-xs font-semibold transition {libraryState.selectedTags.includes(tag)
              ? 'border-mint/50 bg-mint-soft/50 text-mint-deep shadow-sm'
              : 'border-surface/60 bg-surface/45 text-azure-900/55 hover:bg-surface/70'}"
          >
            <Icon name="tag" size={11} />
            {tag}
          </button>
        {/each}
      {/if}
    </div>
  {/if}

  {#if appState.libraryError}
    <div class="glass enter-fade flex flex-1 items-center justify-center rounded-xl2 p-8">
      <div class="text-center text-rose-deep">
        <div class="mb-3 flex justify-center opacity-70">
          <Icon name="alert" size={40} />
        </div>
        <p class="text-sm font-medium">{t("library.error.title")}</p>
        <p class="mt-1 text-xs text-azure-900/60">{t("library.error.hidden")}</p>
        <p class="mt-1 text-xs text-azure-900/60">{t("library.error.restore")}</p>
        <ConfirmButton
          label={t("library.error.accept")}
          icon="check"
          confirmLabel={t("library.error.accept.confirm")}
          title={t("library.error.accept.title")}
          onconfirm={async () => {
            if (!report) return;
            try {
              await readoptIndex(report.library_dir);
              await appState.refreshLibrary();
            } catch (e) {
              appState.toast("error", String(e));
            }
          }}
        />
      </div>
    </div>
  {:else if visible.length === 0}
    <div class="glass enter-up flex flex-1 items-center justify-center rounded-xl2 p-8">
      <div class="text-center text-azure-900/55">
        <div class="mb-3 flex justify-center opacity-70">
          <Icon name={statuses.length === 0 ? "library" : "eye-off"} size={44} />
        </div>
        {#if statuses.length === 0}
          <p class="text-sm font-medium">{t("library.empty.none")}</p>
          <p class="mt-1 text-xs">
            {t("library.empty.none.hint")}
          </p>
        {:else}
          <p class="text-sm font-medium">{t("library.empty.hidden")}</p>
          <p class="mt-1 text-xs">
            {t("library.empty.hidden.hint")}
          </p>
        {/if}
      </div>
    </div>
  {:else if matches.length === 0}
    <div class="glass enter-fade flex flex-1 items-center justify-center rounded-xl2 p-8">
      <div class="text-center text-azure-900/55">
        <div class="mb-3 flex justify-center opacity-70">
          <Icon name="search" size={40} />
        </div>
        <p class="text-sm font-medium">{t("library.empty.search", { query: libraryState.search.trim() })}</p>
      </div>
    </div>
  {:else if shown.length === 0}
    <div class="glass enter-fade flex flex-1 items-center justify-center rounded-xl2 p-8">
      <div class="text-center text-azure-900/55">
        <div class="mb-3 flex justify-center opacity-70">
          <Icon name="check" size={40} />
        </div>
        {#if libraryState.selectedTags.length > 0}
          <p class="text-sm font-medium">{t("library.empty.tags")}</p>
          <button
            onclick={clearTagFilters}
            class="mt-2 lift rounded-lg border border-surface/60 px-3 py-1.5 text-xs font-medium text-azure-900/65 hover:bg-surface/70"
          >
            {t("library.empty.tags.clear")}
          </button>
        {:else}
          <p class="text-sm font-medium">{t("library.empty.category")}</p>
        {/if}
      </div>
    </div>
  {:else}
    <div class="min-h-0 flex-1 overflow-y-auto pr-1" bind:this={scrollEl} onscroll={onGridScroll}>
      <div
        data-vgrid
        class="grid grid-cols-3 gap-3 max-lg:grid-cols-1 max-xl:grid-cols-2"
        style={gridPadding}
      >
        {#each visibleSlice as status (status.app_id)}
          <GameCard
            {status}
            selecting={libraryState.selectionMode}
            selected={libraryState.selection.includes(status.app_id)}
            onToggleSelect={() => toggleSelect(status.app_id)}
            onContextMenu={onCardContextMenu}
            onKebabClick={onKebabClick}
          />
        {/each}
      </div>
    </div>
  {/if}
</div>

{#if ctxMenu && busy === null}
  <ContextMenu
    items={menuItemsFor(ctxMenu.status)}
    x={ctxMenu.x}
    y={ctxMenu.y}
    onclose={closeContextMenu}
    onaction={executeAction}
  />
{/if}

{#if tagEditor}
  <TagEditor
    status={tagEditor.status}
    allLibraryTags={tagList}
    trigger={tagEditor.trigger}
    onclose={() => (tagEditor = null)}
    onsave={() => (tagEditor = null)}
  />
{/if}

{#if bulkPhase && bulkPlan}
  <BulkProgress
    plan={filteredPlan}
    phase={bulkPhase}
    progress={bulkProgress}
    report={bulkReport}
    kind={bulkMode === "repair" ? "repair" : "install"}
    wording={selectionWording}
    selectionLabel={selectionSectionLabel}
    onconfirm={() => void confirmBulk()}
    oncancel={cancelBulkRun}
    onclose={closeBulk}
    onnext={waitingForNext ? onNext : undefined}
    {nextLabel}
  />
{/if}

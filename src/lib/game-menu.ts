import type { GameStatus } from "./api";
import {
  copyToSteam,
  installGameViaSteam,
  importPatchArchive,
  launchGame,
  removeLibraryEntry,
  removeLuaFromSteam,
  setLibraryHidden,
  uninstallOnlineFix,
  verifyOnlineFix,
} from "./api";
import { installFixWithRepair } from "./defender";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { t } from "./i18n.svelte";

export type MenuGroup = "action" | "utility" | "danger";

export type GameMenuItem = {
  id: string;
  label: string;
  /** Nom d'icône existant dans Icons.svelte. */
  icon: string;
  /** Infobulle courte, en français. */
  tip: string;
  /** Groupe auquel appartient cette entrée. */
  group: MenuGroup;
  /** Séparateur visuel AVANT cette entrée. */
  separated?: boolean;
  /** Action destructrice — rendue dans le ton `rose`. */
  danger?: boolean;
  /** Action réseau suspendue tant qu'LuaVault est injoignable (LOT-11). */
  disabled?: boolean;
  /** Infobulle de substitution quand l'entrée est désactivée. */
  disabledTip?: string;
  /** The status is captured by closure when `menuItemsFor` is called. */
  run: () => Promise<string | null>;
};

/** Entrées applicables à ce jeu, dans l'ordre d'affichage. */
export function menuItemsFor(status: GameStatus): GameMenuItem[] {
  const items: GameMenuItem[] = [];

  // Lancer le jeu
  if (status.game.fully_installed) {
    items.push({
      id: "launch",
      label: t("menu.launch.label"),
      icon: "play",
      tip: t("menu.launch.tip"),
      group: "action",
      run: async () => {
        await launchGame(status.app_id);
        return t("menu.launch.done", { name: status.name });
      },
    });
  }

  // Installer via Steam
  if (!status.game.installed) {
    items.push({
      id: "steam-install",
      label: t("menu.steam-install.label"),
      icon: "steam",
      tip: t("menu.steam-install.tip"),
      group: "action",
      run: async () => {
        await installGameViaSteam(status.app_id);
        return t("menu.steam-install.done", { name: status.name });
      },
    });
  }

  // Copier le .lua vers Steam
  if (status.in_library && !status.lua_in_steam) {
    items.push({
      id: "copy-lua",
      label: t("menu.copy-lua.label"),
      icon: "copy",
      tip: t("menu.copy-lua.tip"),
      group: "action",
      run: async () => {
        await copyToSteam(status.app_id);
        return t("menu.copy-lua.done", { name: status.name });
      },
    });
  }

  // Retirer le .lua de Steam
  if (status.lua_in_steam) {
    items.push({
      id: "remove-lua",
      label: t("menu.remove-lua.label"),
      icon: "x",
      tip: t("menu.remove-lua.tip"),
      group: "action",
      run: async () => {
        const removed = await removeLuaFromSteam(status.app_id);
        return removed
          ? t("menu.remove-lua.done", { name: status.name })
          : t("menu.remove-lua.none");
      },
    });
  }

  // Importer une archive locale pour ce jeu précis. Le menu de la carte est
  // l'étape de sélection explicite : aucun chiffre du nom de fichier n'est
  // utilisé ici pour décider quel jeu recevra le patch.
  if (status.in_library) {
    items.push({
      id: "import-patch",
      label: t("menu.import-patch.label"),
      icon: "patch",
      tip: t("menu.import-patch.tip"),
      group: "action",
      run: async () => {
        const selected = await open({
          title: t("library.patch.dialog.title"),
          multiple: false,
          filters: [{ name: t("library.patch.dialog.filter"), extensions: ["zip", "rar", "7z"] }],
        });
        if (typeof selected !== "string") return null;
        const ok = await confirm(
          t("library.patch.confirm", { name: status.name, appId: status.app_id }),
          {
            title: t("library.patch.confirm.title"),
            kind: "warning",
            okLabel: t("library.patch.confirm.ok"),
            cancelLabel: t("library.patch.confirm.cancel"),
          },
        );
        if (!ok) return null;
        await importPatchArchive(selected, status.app_id);
        return t("library.patch.done", { name: status.name });
      },
    });
  }

  // Installer / réparer le patch
  if (status.fix_downloaded && status.fix.health !== "healthy" && status.game.fully_installed && status.fix.foreign.length === 0) {
    items.push({
      id: "install-fix",
      label: status.fix.health === "damaged" || status.fix.health === "game_moved"
        ? t("menu.install-fix.repair.label")
        : t("menu.install-fix.install.label"),
      icon: "patch",
      tip: status.fix.health === "damaged" || status.fix.health === "game_moved"
        ? t("menu.install-fix.repair.tip")
        : t("menu.install-fix.install.tip"),
      group: "action",
      run: async () => {
        const report = await installFixWithRepair(status.app_id);
        return report.health === "healthy"
          ? t("menu.install-fix.done", { name: status.name, count: report.file_count })
          : t("menu.install-fix.unhealthy", { name: status.name });
      },
    });
  }

  // Vérifier le patch
  if (status.fix_downloaded && status.game.fully_installed) {
    items.push({
      id: "verify-fix",
      label: t("menu.verify-fix.label"),
      icon: "check",
      tip: t("menu.verify-fix.tip"),
      group: "action",
      run: async () => {
        const report = await verifyOnlineFix(status.app_id);
        return report.health === "healthy"
          ? t("menu.verify-fix.done", { name: status.name, count: report.file_count })
          : t("menu.verify-fix.broken", {
              name: status.name,
              missing: report.missing.length,
              modified: report.modified.length,
            });
      },
    });
  }

  // Désinstaller le patch
  if (status.fix.health === "healthy" || status.fix.health === "damaged") {
    items.push({
      id: "uninstall-fix",
      label: t("menu.uninstall-fix.label"),
      icon: "broom",
      tip: t("menu.uninstall-fix.tip"),
      group: "action",
      run: async () => {
        const report = await uninstallOnlineFix(status.app_id);
        return t("menu.uninstall-fix.done", {
          removed: report.removed,
          restored: report.restored,
        });
      },
    });
  }

  // Ouvrir le dossier du jeu
  if (status.game.install_dir || status.fix.game_dir) {
    const dir = status.fix.game_dir ?? status.game.install_dir!;
    items.push({
      id: "open-folder",
      label: t("menu.open-folder.label"),
      icon: "folder",
      tip: dir,
      group: "utility",
      run: async () => {
        const d = status.fix.game_dir ?? status.game.install_dir!;
        await revealItemInDir(d);
        return t("menu.open-folder.done", { name: status.name });
      },
    });
  }

  // Masquer de la bibliothèque
  items.push({
    id: "hide",
    label: t("menu.hide.label"),
    icon: "eye-off",
    tip: t("menu.hide.tip"),
    group: "utility",
    run: async () => {
      await setLibraryHidden(status.app_id, true);
      return t("menu.hide.done", { name: status.name });
    },
  });

  // Modifier les tags
  if (status.in_library) {
    items.push({
      id: "edit-tags",
      label: t("menu.edit-tags.label"),
      icon: "tag",
      tip: t("menu.edit-tags.tip"),
      group: "utility",
      run: async () => {
        return "edit-tags";
      },
    });
  }

  // Retirer de la bibliothèque
  if (status.in_library) {
    items.push({
      id: "remove",
      label: t("menu.remove.label"),
      icon: "trash",
      tip: t("menu.remove.tip"),
      group: "danger",
      danger: true,
      run: async () => {
        await removeLibraryEntry(status.app_id);
        return t("menu.remove.done", { name: status.name });
      },
    });
  }

  // Compute `separated` after the fact: the first visible entry of each group
  // determines where separators go. Because visibility depends on the game
  // (e.g. "open folder" only appears when a directory exists, which shifts
  // which entry opens the utility group), we cannot decide `separated` at
  // push-time. We walk the built array once and mark the first entry of
  // each group — except the very first menu entry overall.
  let lastGroup: MenuGroup | null = null;
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.group !== lastGroup) {
      item.separated = i > 0;
      lastGroup = item.group;
    }
  }

  return items;
}

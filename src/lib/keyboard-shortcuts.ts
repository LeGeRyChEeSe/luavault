/**
 * Pure keyboard-shortcut resolver — the single source of truth for which
 * key combinations trigger which action.
 *
 * Contract:
 *  - Ctrl+L → open the logs view.
 *  - ? (no Ctrl/Alt) → open the shortcuts help modal.
 *  - everything else → null.
 *
 * The resolver returns null when `editable` is true.
 * Letter comparisons are case-insensitive (Windows-only project).
 */

export type ShortcutAction =
  | "open-logs"
  | "open-help"
  | null;

/**
 * Check whether a DOM event target is an editable element.
 * Uses a structural contract testable without a full DOM:
 * `tagName` for INPUT/TEXTAREA/SELECT and `isContentEditable === true`.
 * In browsers the descendant inherits `isContentEditable` from an ancestor.
 */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!target || typeof target !== "object") return false;

  const el = target as { tagName?: string; isContentEditable?: boolean };

  // Direct editable elements
  if (String(el.tagName).toUpperCase() === "INPUT" ||
      String(el.tagName).toUpperCase() === "TEXTAREA" ||
      String(el.tagName).toUpperCase() === "SELECT") {
    return true;
  }

  // contenteditable ancestor (including the element itself)
  return el.isContentEditable === true;
}

/**
 * Resolve a keyboard event to a shortcut action.
 * Returns null when no action should be taken or when the target is editable.
 *
 * Pure: reads only its parameters, returns only an action or null.
 */
export function resolveShortcut({
  key,
  ctrlKey,
  altKey,
  editable,
}: {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  editable: boolean;
}): ShortcutAction {
  if (editable) return null;

  // Ctrl+L → open logs (case-insensitive letter)
  if (ctrlKey && !altKey && key.toLowerCase() === "l") {
    return "open-logs";
  }

  // ? (no Ctrl/Alt) → open help
  if (!ctrlKey && !altKey && key === "?") {
    return "open-help";
  }

  return null;
}

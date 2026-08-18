/**
 * The one focus-trap every modal surface uses (LOT-19, task 3). A Svelte
 * action: `use:focusTrap` on the dialog element.
 *
 * Contract:
 *  - on mount, remember what had the focus and move it inside — the first
 *    focusable control (or a `[data-autofocus]` descendant when present),
 *    or the container itself with `initial: "container"` for screens that
 *    read like documents;
 *  - Tab from the last control wraps to the first, Shift+Tab from the first
 *    wraps to the last, and focusable controls are re-queried at key time
 *    because `disabled` attributes and phases change while a dialog is open;
 *  - a dialog with no focusable descendant keeps the focus on the container;
 *  - on destroy, the previous focus comes back when it still exists;
 *  - Escape is never intercepted here — each component keeps its own close
 *    policy.
 *  - the `keydown` listener is anchored on `document` so that Tab from
 *    `<body>` (after the triggering control was destroyed) still bubbles
 *    into the topmost trap; the listener is removed in `destroy`.
 *
 * Nested dialogs (the screenshot viewer over the game card) go through a
 * module-level stack: only the topmost open trap answers Tab, and closing it
 * restores the focus to the element that opened it.
 */

export interface FocusTrapOptions {
  /** "first" (default): first focusable control, or `[data-autofocus]`.
      "container": the dialog element itself. */
  initial?: "first" | "container";
  /** Explicit element to restore focus to on destroy. When unset the trap
      falls back to the element that had focus at mount time — which may
      still be a transient element (e.g. a context-menu item) that was
      unmounted in the same Svelte tick. */
  returnFocus?: HTMLElement | null;
}

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Open traps, bottom → top. The last entry owns the Tab key. */
const stack: HTMLElement[] = [];

/** Single document-level listener, installed once, removed on last destroy. */
let docListener: ((e: KeyboardEvent) => void) | null = null;

function ensureDocListener() {
  if (docListener) return;
  docListener = (e: KeyboardEvent) => {
    if (e.key !== "Tab") return;
    const top = stack[stack.length - 1];
    if (!top) return;
    const items = focusables(top);
    if (items.length === 0) {
      e.preventDefault();
      top.tabIndex = -1;
      top.focus();
      return;
    }
    const active = document.activeElement;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey) {
      if (active === first || !top.contains(active as Node)) {
        e.preventDefault();
        last.focus();
      }
    } else if (active === last || !top.contains(active as Node)) {
      e.preventDefault();
      first.focus();
    }
  };
  document.addEventListener("keydown", docListener);
}

function removeDocListener() {
  if (docListener) {
    document.removeEventListener("keydown", docListener);
    docListener = null;
  }
}

function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => el.tabIndex >= 0,
  );
}

export function focusTrap(node: HTMLElement, options: FocusTrapOptions = {}) {
  const initial = options.initial ?? "first";
  const returnFocusTarget = options.returnFocus ?? null;
  const previous =
    document.activeElement instanceof HTMLElement && document.activeElement !== document.body
      ? document.activeElement
      : null;

  const wasEmpty = stack.length === 0;
  if (wasEmpty) ensureDocListener();
  stack.push(node);

  if (initial === "container") {
    node.tabIndex = -1;
    node.focus();
  } else {
    const target = node.querySelector<HTMLElement>("[data-autofocus]") ?? focusables(node)[0];
    if (target) {
      target.focus();
    } else {
      node.tabIndex = -1;
      node.focus();
    }
  }

  return {
    destroy() {
      const at = stack.indexOf(node);
      if (at !== -1) stack.splice(at, 1);
      if (stack.length === 0) removeDocListener();
      if (returnFocusTarget && document.contains(returnFocusTarget)) {
        returnFocusTarget.focus();
      } else if (previous && document.contains(previous)) {
        previous.focus();
      }
    },
  };
}

/**
 * Tooltips drawn in a single top-level layer.
 *
 * The old implementation was a `::after` on the hovered element, which meant
 * any ancestor with `overflow: hidden`, a stacking context or a higher sibling
 * could clip or cover it — and several of the app's panels do all three.
 * Here the bubble lives in `<body>` at the highest z-index in the document, is
 * positioned with fixed coordinates, and flips to whichever side has room.
 *
 * The markup contract is unchanged: any element with `data-tip` gets one.
 */

const OFFSET = 9;
const MARGIN = 8;
const SHOW_DELAY = 260;

let bubble: HTMLDivElement | null = null;
let arrow: HTMLDivElement | null = null;
let current: HTMLElement | null = null;
let timer: number | undefined;

function ensureBubble(): HTMLDivElement {
  if (bubble) return bubble;

  bubble = document.createElement("div");
  bubble.id = "lv-tooltip";
  bubble.setAttribute("role", "tooltip");
  bubble.style.cssText = [
    "position:fixed",
    "top:0",
    "left:0",
    // Above every panel, modal and overlay the app can create.
    "z-index:2147483000",
    "max-width:19rem",
    "padding:0.42rem 0.62rem",
    "border-radius:0.6rem",
    "font-size:0.72rem",
    "line-height:1.35",
    "font-weight:500",
    "text-align:center",
    "white-space:pre-line",
    "pointer-events:none",
    "opacity:0",
    "transform:translate3d(0,4px,0)",
    "transition:opacity 140ms cubic-bezier(0.22,1,0.36,1),transform 140ms cubic-bezier(0.22,1,0.36,1)",
    "box-shadow:0 10px 30px oklch(0 0 0 / 0.28)",
    "backdrop-filter:blur(8px)",
  ].join(";");

  arrow = document.createElement("div");
  arrow.style.cssText =
    "position:absolute;width:8px;height:8px;transform:rotate(45deg);border-radius:1px;";
  bubble.appendChild(arrow);

  document.body.appendChild(bubble);
  return bubble;
}

/** Read the palette from the document so the bubble follows the theme. */
function applyTheme(node: HTMLDivElement) {
  const styles = getComputedStyle(document.documentElement);
  const dark = document.documentElement.dataset.mode === "dark";
  const bg = dark
    ? styles.getPropertyValue("--color-azure-100").trim()
    : styles.getPropertyValue("--color-azure-900").trim();
  const fg = dark
    ? styles.getPropertyValue("--color-azure-900").trim()
    : "#fff";
  node.style.background = bg || "#0b3f6f";
  node.style.color = fg || "#fff";
  node.style.border = dark
    ? "1px solid oklch(1 0 0 / 0.12)"
    : "1px solid oklch(1 0 0 / 0.08)";
  if (arrow) arrow.style.background = bg || "#0b3f6f";
}

function place(target: HTMLElement, text: string) {
  const node = ensureBubble();
  // Text node first so measurement is right; the arrow stays as the last child.
  node.textContent = text;
  if (arrow) node.appendChild(arrow);
  applyTheme(node);

  node.style.opacity = "0";
  node.style.left = "0px";
  node.style.top = "0px";

  const anchor = target.getBoundingClientRect();
  const box = node.getBoundingClientRect();

  // Prefer above; drop below when the top edge is too close.
  const below = anchor.top - box.height - OFFSET < MARGIN;
  let top = below ? anchor.bottom + OFFSET : anchor.top - box.height - OFFSET;
  let left = anchor.left + anchor.width / 2 - box.width / 2;

  left = Math.min(Math.max(left, MARGIN), window.innerWidth - box.width - MARGIN);
  top = Math.min(Math.max(top, MARGIN), window.innerHeight - box.height - MARGIN);

  node.style.left = `${Math.round(left)}px`;
  node.style.top = `${Math.round(top)}px`;

  if (arrow) {
    const centre = anchor.left + anchor.width / 2 - left;
    arrow.style.left = `${Math.round(Math.min(Math.max(centre - 4, 10), box.width - 18))}px`;
    arrow.style.top = below ? "-4px" : `${Math.round(box.height - 4)}px`;
  }

  requestAnimationFrame(() => {
    node.style.opacity = "1";
    node.style.transform = "translate3d(0,0,0)";
  });
}

function hide() {
  window.clearTimeout(timer);
  current = null;
  if (!bubble) return;
  bubble.style.opacity = "0";
  bubble.style.transform = "translate3d(0,4px,0)";
}

function tipTarget(node: EventTarget | null): HTMLElement | null {
  if (!(node instanceof Element)) return null;
  const found = node.closest("[data-tip]");
  return found instanceof HTMLElement ? found : null;
}

function onOver(event: Event) {
  const target = tipTarget(event.target);
  if (!target || target === current) {
    if (!target) hide();
    return;
  }
  const text = target.getAttribute("data-tip");
  if (!text) {
    hide();
    return;
  }
  hide();
  current = target;
  // A short delay keeps the screen quiet while the pointer is just passing by.
  timer = window.setTimeout(() => {
    if (current === target && target.isConnected) place(target, text);
  }, SHOW_DELAY);
}

/** Install the global listeners. Call once, from the app shell. */
export function mountTooltips() {
  document.addEventListener("pointerover", onOver, true);
  document.addEventListener("pointerdown", hide, true);
  document.addEventListener("pointerleave", hide, true);
  // A tooltip anchored to a scrolled-away element would float over nothing.
  window.addEventListener("scroll", hide, true);
  window.addEventListener("blur", hide);
  window.addEventListener("resize", hide);

  return () => {
    document.removeEventListener("pointerover", onOver, true);
    document.removeEventListener("pointerdown", hide, true);
    document.removeEventListener("pointerleave", hide, true);
    window.removeEventListener("scroll", hide, true);
    window.removeEventListener("blur", hide);
    window.removeEventListener("resize", hide);
    bubble?.remove();
    bubble = null;
    arrow = null;
  };
}

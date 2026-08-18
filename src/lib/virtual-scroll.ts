/**
 * Pure virtual-scroll windowing — no DOM, no Svelte, fully testable.
 *
 * The library grid renders cards in rows of `cols` columns.  Given the
 * scroll position and the viewport size, this module computes which slice
 * of items to mount and how much spacer padding to add above and below so
 * the scrollbar stays honest.
 */

export interface VirtualWindow {
  /** First rendered row (overscan included). */
  startRow: number;
  /** Last rendered row, exclusive (overscan included). */
  endRow: number;
  /** Index of the first item to render. */
  startIndex: number;
  /** Index past the last item to render. */
  endIndex: number;
  /** Spacer height above the rendered slice (px). */
  offsetTop: number;
  /** Spacer height below the rendered slice (px). */
  offsetBottom: number;
  /** Total scrollable height (px) — what the scrollbar sees. */
  totalHeight: number;
}

/**
 * Compute the visible window for a uniform-row grid.
 *
 * @param scrollTop      Current scroll offset (px).
 * @param viewportHeight Visible height of the scroll container (px).
 * @param rowHeight      Row stride: card height + inter-row gap (px).
 * @param totalItems     Number of items in the list.
 * @param cols           Number of grid columns.
 * @param overscan       Extra rows rendered above and below the viewport.
 */
export function virtualWindow(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  totalItems: number,
  cols: number,
  overscan = 3,
): VirtualWindow {
  if (totalItems <= 0 || cols <= 0 || rowHeight <= 0) {
    return {
      startRow: 0,
      endRow: 0,
      startIndex: 0,
      endIndex: 0,
      offsetTop: 0,
      offsetBottom: 0,
      totalHeight: 0,
    };
  }

  const totalRows = Math.ceil(totalItems / cols);

  const firstVisible = Math.max(0, Math.floor(scrollTop / rowHeight));
  const visibleCount = Math.ceil(viewportHeight / rowHeight);
  const lastVisible = firstVisible + visibleCount;

  // Clamp the slice: once scrollTop overshoots the content (list shrunk by a
  // search, window widened so columns ate rows), an unbounded startRow inverts
  // the slice and pushes offsetTop past totalHeight — which inflates the
  // container and keeps the stale scroll position alive.
  // Clamp to a full screen from the bottom, not to the last row: bounding at
  // `totalRows - 1` still parks the window on a single row while the browser
  // walks scrollTop back, which shows as one frame of 3 cards instead of 24.
  const maxStartRow = Math.max(0, totalRows - visibleCount - overscan);
  const startRow = Math.min(maxStartRow, Math.max(0, firstVisible - overscan));
  const endRow = Math.min(totalRows, Math.max(startRow, lastVisible + overscan));

  const startIndex = startRow * cols;
  const endIndex = Math.min(totalItems, endRow * cols);

  const offsetTop = startRow * rowHeight;
  const totalHeight = totalRows * rowHeight;
  const offsetBottom = Math.max(0, totalHeight - endRow * rowHeight);

  return { startRow, endRow, startIndex, endIndex, offsetTop, offsetBottom, totalHeight };
}

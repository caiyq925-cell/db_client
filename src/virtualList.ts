/**
 * Windowing math for fixed-row-height virtual lists.
 *
 * Pure function so the clipping behaviour is unit-testable without a DOM:
 * given the scroll position and viewport size, return the inclusive-exclusive
 * slice [start, end) of items that should be rendered — the first visible row
 * anchored window with `overscan` rows rendered above and below (clamped to
 * the list bounds so no phantom rows appear at the top or bottom).
 */
export function visibleRange(
  scrollTop: number,
  viewportH: number,
  rowH: number,
  total: number,
  overscan = 10,
): { start: number; end: number } {
  if (rowH <= 0 || total <= 0) return { start: 0, end: 0 };
  const firstVisible = Math.floor(scrollTop / rowH);
  const start = Math.min(Math.max(0, firstVisible - overscan), total);
  const end = Math.min(total, firstVisible + Math.ceil(viewportH / rowH) + overscan);
  return { start, end: Math.max(end, start) };
}

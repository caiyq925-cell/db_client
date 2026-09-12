import { describe, expect, it } from 'vitest';
import { visibleRange } from '../virtualList';

describe('visibleRange', () => {
  it('renders the first window at scrollTop 0 (no phantom rows above)', () => {
    const r = visibleRange(0, 280, 28, 3000, 10);
    expect(r.start).toBe(0);
    // 10 visible rows + 10 overscan below (0 above — already at top)
    expect(r.end).toBe(20);
  });

  it('skips rows scrolled past above the viewport', () => {
    // scrollTop = 28 * 100 → first visible row is 100
    const r = visibleRange(2800, 280, 28, 3000, 10);
    expect(r.start).toBe(90); // 100 - 10 overscan above
    expect(r.end).toBe(120); // 100 + 10 visible + 10 overscan below
  });

  it('clamps to the total at the bottom', () => {
    const r = visibleRange(28 * 2990, 280, 28, 3000, 10);
    expect(r.start).toBe(2980);
    expect(r.end).toBe(3000);
  });

  it('handles fewer items than one viewport', () => {
    const r = visibleRange(0, 280, 28, 5, 10);
    expect(r.start).toBe(0);
    expect(r.end).toBe(5);
  });

  it('handles empty list', () => {
    expect(visibleRange(0, 280, 28, 0, 10)).toEqual({ start: 0, end: 0 });
  });

  it('handles zero-height viewport (not yet measured)', () => {
    const r = visibleRange(0, 0, 28, 100, 10);
    expect(r.start).toBe(0);
    // still renders overscan rows so the list appears once measured
    expect(r.end).toBe(10);
  });

  it('never returns an inverted or out-of-bounds range for extreme scroll', () => {
    // way past the end
    const r = visibleRange(28 * 99999, 280, 28, 3000, 10);
    expect(r.start).toBe(3000);
    expect(r.end).toBe(3000);
  });

  it('handles fractional scrollTop', () => {
    const r = visibleRange(28.5, 280, 28, 3000, 10);
    expect(r.start).toBe(0); // firstVisible=1, minus overscan clamps to 0
    expect(r.end).toBe(21); // 1 + 10 visible + 10 overscan
  });
});

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { visibleRange } from './virtualList';

interface VirtualListProps<T> {
  items: T[];
  /** Stable key per item (already unique within the list). */
  getKey: (item: T, index: number) => string;
  /** Fixed row height in px — the windowing math depends on it. */
  rowH?: number;
  overscan?: number;
  renderItem: (item: T, index: number) => ReactNode;
  /** Extra styles for the scroll container (e.g. flex fill). */
  style?: React.CSSProperties;
}

/**
 * Minimal fixed-row-height virtual list (no dependency): only the visible
 * window ± overscan rows are mounted. Used for schema/table lists that can
 * reach thousands of entries.
 */
export function VirtualizedList<T>({ items, getKey, rowH = 28, overscan = 10, renderItem, style }: VirtualListProps<T>) {
  const ref = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(0);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setHeight(el.clientHeight);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const onScroll = useCallback(() => {
    setScrollTop(ref.current?.scrollTop ?? 0);
  }, []);

  const { start, end } = visibleRange(scrollTop, height, rowH, items.length, overscan);
  const slice = items.slice(start, end);

  return (
    <div ref={ref} onScroll={onScroll} style={{ overflowY: 'auto', minHeight: 0, ...style }}>
      <div style={{ height: items.length * rowH, position: 'relative' }}>
        <div style={{ position: 'absolute', top: start * rowH, left: 0, right: 0 }}>
          {slice.map((item, i) => (
            <div key={getKey(item, start + i)} style={{ height: rowH }}>
              {renderItem(item, start + i)}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

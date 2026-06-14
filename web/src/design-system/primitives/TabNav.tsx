import { useRef, type KeyboardEvent } from 'react';
import { cn } from '@/lib/cn';
import { Badge } from '@/design-system/primitives/Badge';

/* TabNav — an accessible tablist usable as either a horizontal strip OR a
 * vertical rail. Roving tabindex, ARIA-correct (aria-orientation, aria-selected,
 * id/aria-controls), arrow-key navigation that follows the orientation
 * (Left/Right for horizontal, Up/Down for vertical), plus Home/End. The active
 * tab is marked by a cyan-gray accent indicator (underline in horizontal, 2px
 * left rail in vertical) AND brighter ink — never color alone. */

export interface TabSpec<T extends string> {
  id: T;
  label: string;
}

export function TabNav<T extends string>({
  tabs,
  active,
  onSelect,
  counts = {},
  ariaLabel = 'Panels',
  orientation = 'horizontal',
}: {
  tabs: readonly TabSpec<T>[];
  active: T;
  onSelect: (id: T) => void;
  counts?: Partial<Record<T, number>>;
  ariaLabel?: string;
  orientation?: 'horizontal' | 'vertical';
}) {
  const refs = useRef<Record<string, HTMLButtonElement | null>>({});
  const isVertical = orientation === 'vertical';

  function onKeyDown(e: KeyboardEvent, idx: number) {
    let next = idx;
    const prevKey = isVertical ? 'ArrowUp' : 'ArrowLeft';
    const nextKey = isVertical ? 'ArrowDown' : 'ArrowRight';
    if (e.key === nextKey) next = (idx + 1) % tabs.length;
    else if (e.key === prevKey) next = (idx - 1 + tabs.length) % tabs.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = tabs.length - 1;
    else return;
    e.preventDefault();
    const id = tabs[next].id;
    onSelect(id);
    refs.current[id]?.focus();
  }

  return (
    <nav
      role="tablist"
      aria-label={ariaLabel}
      aria-orientation={orientation}
      className={
        isVertical
          // Fixed-width rail keeps row positions stable when count badges
          // appear / disappear.
          ? 'flex w-32 shrink-0 flex-col overflow-y-auto border-r border-iron bg-charcoal min-h-0 prism-scrollbar'
          : 'sticky top-0 z-10 flex border-b border-iron bg-charcoal'
      }
    >
      {tabs.map((t, i) => {
        const selected = t.id === active;
        const count = counts[t.id];
        const horizontalCls = cn(
          'flex min-w-0 flex-1 items-center justify-center gap-1 rounded-none border-b-2 bg-transparent px-1 py-2.5 text-data-s transition-colors duration-150',
          selected ? 'border-cyan-gray text-bone' : 'border-transparent text-archive hover:text-bone',
        );
        // Vertical: a full-width row with a 2px cyan-gray rail on the LEFT (the
        // moral equivalent of the horizontal underline).
        const verticalCls = cn(
          'flex items-center justify-between gap-1.5 rounded-none border-l-2 bg-transparent px-3 py-2 text-left text-data-s transition-colors duration-150',
          selected ? 'border-cyan-gray text-bone' : 'border-transparent text-archive hover:text-bone',
        );
        return (
          <button
            key={t.id}
            ref={(el) => {
              refs.current[t.id] = el;
            }}
            role="tab"
            id={`tab-${t.id}`}
            aria-selected={selected}
            aria-controls={`tabpanel-${t.id}`}
            tabIndex={selected ? 0 : -1}
            title={t.label}
            onClick={() => onSelect(t.id)}
            onKeyDown={(e) => onKeyDown(e, i)}
            className={isVertical ? verticalCls : horizontalCls}
          >
            <span className="truncate">{t.label}</span>
            {count ? (
              <Badge tone="neutral" mono className="px-1 py-0">
                {count}
              </Badge>
            ) : null}
          </button>
        );
      })}
    </nav>
  );
}

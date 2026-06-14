/* TabNav — an accessible tablist usable as either a horizontal strip OR a
 * vertical rail. Roving tabindex, ARIA-correct (aria-orientation, aria-selected,
 * id/aria-controls), arrow-key navigation that follows the orientation
 * (Left/Right for horizontal, Up/Down for vertical), plus Home/End. The active
 * tab is marked by an accent indicator (underline in horizontal, left rail in
 * vertical) AND bolder ink — never colour alone. */

import { useRef, type KeyboardEvent } from 'react';
import { Badge } from './Badge';

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
          // appear / disappear; 128 px holds every current label including
          // "Duplicates" with a small count badge alongside.
          ? 'flex flex-col shrink-0 w-32 border-r border-line bg-panel min-h-0 overflow-y-auto prism-scrollbar'
          : 'flex border-b border-line bg-panel sticky top-0 z-10'
      }
    >
      {tabs.map((t, i) => {
        const selected = t.id === active;
        const count = counts[t.id];
        const horizontalCls = `flex-1 min-w-0 flex items-center justify-center gap-1 px-1 py-2.5 text-mini font-semibold border-b-2 bg-transparent rounded-none ${
          selected ? 'text-ink border-accent' : 'text-muted border-transparent hover:bg-panel-2 hover:text-ink'
        }`;
        // Vertical: a full-width row with a 2-px accent rail on the LEFT (the
        // moral equivalent of the horizontal underline). Active row gets a
        // subtle bg so the selection reads at a glance.
        const verticalCls = `flex items-center justify-between gap-1.5 px-3 py-2 text-mini font-semibold bg-transparent rounded-none border-l-2 text-left transition-colors ${
          selected ? 'text-ink border-accent bg-panel-2/60' : 'text-muted border-transparent hover:bg-panel-2 hover:text-ink'
        }`;
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
            <span className={isVertical ? 'truncate' : 'truncate'}>{t.label}</span>
            {count ? (
              <Badge tone={selected ? 'onAccent' : 'muted'} className="px-1 py-0 leading-[15px]">
                {count}
              </Badge>
            ) : null}
          </button>
        );
      })}
    </nav>
  );
}

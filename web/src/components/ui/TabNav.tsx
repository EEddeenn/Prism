/* TabNav — an accessible tablist for the side rail: roving tabindex, Arrow-key
 * navigation, aria-selected, and id/aria-controls wiring the shell lacked. The
 * active tab is shown by an accent underline AND bolder ink (not color alone). */

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
}: {
  tabs: readonly TabSpec<T>[];
  active: T;
  onSelect: (id: T) => void;
  counts?: Partial<Record<T, number>>;
  ariaLabel?: string;
}) {
  const refs = useRef<Record<string, HTMLButtonElement | null>>({});

  function onKeyDown(e: KeyboardEvent, idx: number) {
    let next = idx;
    if (e.key === 'ArrowRight') next = (idx + 1) % tabs.length;
    else if (e.key === 'ArrowLeft') next = (idx - 1 + tabs.length) % tabs.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = tabs.length - 1;
    else return;
    e.preventDefault();
    const id = tabs[next].id;
    onSelect(id);
    refs.current[id]?.focus();
  }

  return (
    <nav role="tablist" aria-label={ariaLabel} className="flex border-b border-line bg-panel sticky top-0 z-10">
      {tabs.map((t, i) => {
        const selected = t.id === active;
        const count = counts[t.id];
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
            className={`flex-1 min-w-0 flex items-center justify-center gap-1 px-1 py-2.5 text-mini font-semibold border-b-2 bg-transparent rounded-none ${
              selected ? 'text-ink border-accent' : 'text-muted border-transparent hover:bg-panel-2 hover:text-ink'
            }`}
          >
            <span className="truncate">{t.label}</span>
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

import { type CSSProperties } from 'react';
import { cn } from '@/lib/cn';
import { type Tone, toneFill } from '@/lib/tone';

export interface LegendItem {
  /** Tone-driven swatch (preferred). */
  tone?: Tone;
  /** Explicit swatch color when no tone applies (e.g. a graph-role/ramp hex). */
  color?: string;
  label: string;
}

export interface LegendProps {
  items: LegendItem[];
  className?: string;
}

/** Inline key — a small dot swatch + mono label per entry. The label carries the
 *  meaning so the legend is never color-only. */
export function Legend({ items, className }: LegendProps) {
  return (
    <ul role="list" className={cn('flex flex-wrap items-center gap-x-3 gap-y-1', className)}>
      {items.map((item, i) => {
        const style: CSSProperties | undefined =
          !item.tone && item.color ? { backgroundColor: item.color } : undefined;
        return (
          <li key={`${item.label}-${i}`} className="inline-flex items-center gap-1.5">
            <span
              aria-hidden
              style={style}
              className={cn('size-2 shrink-0 rounded-pill', item.tone && toneFill[item.tone])}
            />
            <span className="font-mono text-meta text-archive">{item.label}</span>
          </li>
        );
      })}
    </ul>
  );
}

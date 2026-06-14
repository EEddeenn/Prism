import { cn } from '@/lib/cn';

/* SegmentedControl — a single picker rendered as a "track + inner pill": the
 * outer flex bar carries the border + `rounded-control` corners and a base
 * charcoal surface; every inner segment is transparent until active, then it
 * lifts to bg-iron with its OWN `rounded-control` corners. Because each segment
 * shares the same shape regardless of position, a middle-segment activation no
 * longer pops as a square box flanked by half-rounded neighbours — only the
 * activation state varies. Each button carries aria-pressed (never color alone).*/

export interface SegOption<T extends string> {
  value: T;
  label: string;
  title?: string;
}

const sizes: Record<'sm' | 'md', string> = {
  sm: 'px-2 py-0.5 text-data-s',
  md: 'px-2.5 py-1 text-data',
};

export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  ariaLabel,
  size = 'md',
  className,
}: {
  options: SegOption<T>[];
  value: T;
  onChange: (v: T) => void;
  ariaLabel: string;
  size?: 'sm' | 'md';
  className?: string;
}) {
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      // Track = the visible bar: holds the border + radius + base surface;
      // p-0.5 keeps the active inner pill from clipping against the bar.
      className={cn('inline-flex items-center rounded-control border border-iron bg-charcoal p-0.5', className)}
    >
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            title={o.title}
            aria-pressed={active}
            onClick={() => onChange(o.value)}
            className={cn(
              'rounded-control border-0 bg-transparent transition-colors duration-150',
              sizes[size],
              active ? 'bg-iron text-bone' : 'text-archive hover:text-bone',
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

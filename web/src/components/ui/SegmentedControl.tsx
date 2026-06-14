/* SegmentedControl — a single picker rendered as a "container + inner pill":
 * the outer flex bar carries the border and `rounded-control` corners; every
 * inner segment is transparent until active, then it lifts to bg-panel-3 with
 * its OWN `rounded-control` corners. This retires the legacy shared-edge
 * pattern (rounded-l on first, rounded-none in the middle, rounded-r on last)
 * where the active middle segment popped as a perfectly square box flanked by
 * half-rounded inactive neighbours. Every segment now reads as the same shape
 * regardless of position, and only the activation state varies. */

export interface SegOption<T extends string> {
  value: T;
  label: string;
  title?: string;
}

export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  ariaLabel,
  size = 'md',
  className = '',
}: {
  options: SegOption<T>[];
  value: T;
  onChange: (v: T) => void;
  ariaLabel: string;
  size?: 'sm' | 'md';
  className?: string;
}) {
  const pad = size === 'sm' ? 'px-2 py-1 text-micro' : 'px-2.5 py-1 text-mini';
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      // Container = the visible bar. Holds the border + radius + base bg;
      // p-0.5 keeps the active inner pill from clipping against the bar.
      className={`inline-flex items-center rounded-control border border-line-2 bg-panel-2 p-0.5 ${className}`}
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
            // border-0 + bg-transparent override the global button reset
            // (globals.css applies bg-panel-2 + border-line-2 + rounded-control
            // to every <button>). Active gets its OWN rounded-control pill
            // (bg-panel-3), so a middle-segment activation no longer pops as
            // a different shape than an outer-segment one.
            className={`${pad} rounded-control border-0 bg-transparent transition-colors ${
              active
                ? 'bg-panel-3 text-ink font-semibold'
                : 'text-muted hover:text-ink'
            }`}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

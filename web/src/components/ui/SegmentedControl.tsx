/* SegmentedControl — encapsulates the duplicated rounded-l/rounded-r corner math
 * + active styling + aria-pressed re-typed in Topbar (View/Edge), Controls3D, and
 * Treemap. Active is shown by border + weight, not color alone. */

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
    <div role="group" aria-label={ariaLabel} className={`inline-flex ${className}`}>
      {options.map((o, i) => {
        const active = o.value === value;
        const corner =
          i === 0 ? 'rounded-l-md' : i === options.length - 1 ? 'rounded-r-md' : 'rounded-none';
        return (
          <button
            key={o.value}
            type="button"
            title={o.title}
            aria-pressed={active}
            onClick={() => onChange(o.value)}
            className={`${pad} ${corner} border -ml-px first:ml-0 ${
              active
                ? 'z-10 border-accent text-ink font-semibold bg-panel-3'
                : 'border-line-2 text-muted hover:text-ink hover:bg-panel-2'
            }`}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

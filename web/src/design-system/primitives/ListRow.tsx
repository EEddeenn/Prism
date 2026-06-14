import { forwardRef, type ComponentPropsWithoutRef, type ReactNode } from 'react';
import { cn } from '@/lib/cn';

export interface ListRowProps extends Omit<ComponentPropsWithoutRef<'button'>, 'onSelect'> {
  onSelect?: () => void;
  active?: boolean;
  ariaLabel?: string;
}

/** Accessible clickable list row. Active = cyan-gray border + a 2px left
 *  node-center bar (selection is never color-only) + aria-current. */
export const ListRow = forwardRef<HTMLButtonElement, ListRowProps>(
  ({ onSelect, active = false, disabled = false, ariaLabel, className, children, ...props }, ref) => (
    <button
      ref={ref}
      type="button"
      onClick={onSelect}
      disabled={disabled}
      aria-label={ariaLabel}
      aria-current={active || undefined}
      className={cn(
        'relative w-full rounded-control border px-2 py-1.5 text-left text-data-s transition-colors duration-150',
        active
          ? 'border-cyan-gray bg-charcoal text-bone'
          : 'border-transparent text-archive hover:border-iron hover:bg-charcoal hover:text-bone',
        disabled && 'cursor-not-allowed opacity-50',
        className,
      )}
      {...props}
    >
      {active && <span aria-hidden className="absolute inset-y-1 left-0 w-0.5 rounded-pill bg-node-center" />}
      {children}
    </button>
  ),
);
ListRow.displayName = 'ListRow';

export interface RowProps {
  label: ReactNode;
  value: ReactNode;
  /** Class string for the value (e.g. a tone text class). */
  valueTone?: string;
  className?: string;
}

/** Static key/value row — muted label, mono tabular-nums value. */
export function Row({ label, value, valueTone = 'text-bone', className }: RowProps) {
  return (
    <div className={cn('flex items-baseline justify-between gap-3 py-0.5 text-data-s', className)}>
      <span className="text-archive">{label}</span>
      <span className={cn('font-mono tabular-nums', valueTone)}>{value}</span>
    </div>
  );
}

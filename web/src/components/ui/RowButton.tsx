/* RowButton — the single keyboard-accessible clickable list row. Replaces the
 * non-semantic div/li/tr/span onClick handlers that wired the primary drill-in
 * action of nearly every panel (unreachable by keyboard / screen reader). A real
 * <button> with a focus-visible ring; `active` shows a left accent bar + tint so
 * selection is never conveyed by color alone. */

import type { ReactNode } from 'react';

export function RowButton({
  onSelect,
  active = false,
  disabled = false,
  ariaLabel,
  title,
  className = '',
  children,
}: {
  onSelect: () => void;
  active?: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  title?: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={ariaLabel}
      aria-current={active ? 'true' : undefined}
      disabled={disabled}
      onClick={disabled ? undefined : onSelect}
      className={`relative w-full flex items-center gap-2 text-left rounded-control border px-2 py-1.5 bg-panel-2 transition-colors ${
        active ? 'border-accent bg-panel-3' : 'border-transparent'
      } ${
        disabled
          ? 'opacity-60 cursor-default'
          : 'cursor-pointer hover:border-accent hover:bg-panel-3'
      } ${className}`}
    >
      {active && (
        <span aria-hidden className="absolute left-0 inset-y-1 w-0.5 rounded-pill bg-accent" />
      )}
      {children}
    </button>
  );
}

/** A non-interactive key/value row (static metadata), matching RowButton rhythm. */
export function Row({
  label,
  value,
  valueTone = '',
  className = '',
}: {
  label: ReactNode;
  value: ReactNode;
  /** Extra classes for the value (e.g. `text-warn` to flag a risk signal). */
  valueTone?: string;
  className?: string;
}) {
  return (
    <div className={`flex items-center justify-between gap-2.5 py-0.5 text-body ${className}`}>
      <span className="text-muted">{label}</span>
      <span className={`tabular-nums ${valueTone}`}>{value}</span>
    </div>
  );
}

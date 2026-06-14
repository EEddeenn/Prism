import { forwardRef, type ComponentPropsWithoutRef } from 'react';
import { cn } from '@/lib/cn';
import { shortLabel } from '@/lib/palette';

export interface MonoLabelProps extends Omit<ComponentPropsWithoutRef<'span'>, 'title'> {
  text: string;
  /** Full value for the native tooltip when `text` is truncated/abbreviated. */
  full?: string;
}

/** A single-line mono machine value (ID, version, path segment) that truncates,
 *  surfacing the full value via the native title tooltip. */
export const MonoLabel = forwardRef<HTMLSpanElement, MonoLabelProps>(
  ({ text, full, className, ...props }, ref) => (
    <span ref={ref} title={full ?? text} className={cn('font-mono truncate', className)} {...props}>
      {text}
    </span>
  ),
);
MonoLabel.displayName = 'MonoLabel';

export interface PurlLabelProps {
  purl: string;
  /** When provided, the label becomes a focusable button (e.g. select the node). */
  onSelect?: () => void;
  ariaLabel?: string;
  className?: string;
}

/** A package URL shown compactly via `shortLabel` (full purl in the title).
 *  Clickable (a <button>) when `onSelect` is given, else a static span. */
export const PurlLabel = forwardRef<HTMLElement, PurlLabelProps>(
  ({ purl, onSelect, ariaLabel, className }, ref) => {
    const display = shortLabel(purl);
    if (onSelect) {
      return (
        <button
          ref={ref as React.Ref<HTMLButtonElement>}
          type="button"
          onClick={onSelect}
          title={purl}
          aria-label={ariaLabel}
          className={cn(
            'font-mono truncate text-left transition-colors duration-150 hover:text-cyan-gray',
            className,
          )}
        >
          {display}
        </button>
      );
    }
    return (
      <span
        ref={ref as React.Ref<HTMLSpanElement>}
        title={purl}
        aria-label={ariaLabel}
        className={cn('font-mono truncate', className)}
      >
        {display}
      </span>
    );
  },
);
PurlLabel.displayName = 'PurlLabel';

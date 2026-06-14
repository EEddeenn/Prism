import { forwardRef, type ComponentPropsWithoutRef } from 'react';
import { ArrowUp, ArrowDown, Minus } from 'lucide-react';
import { cn } from '@/lib/cn';
import { deltaTone, toneText } from '@/lib/tone';

export interface DeltaTagProps extends Omit<ComponentPropsWithoutRef<'span'>, 'children'> {
  /** Absolute signed change (preferred when given). */
  value?: number;
  /** Signed percentage change; rendered with a `%` suffix when `value` is absent. */
  pct?: number;
  /** Whether an increase is the bad direction (e.g. risk up = bad). Default true. */
  moreIsWorse?: boolean;
  ariaLabel?: string;
}

/** Signed change indicator — direction arrow + formatted magnitude, mono.
 *  Tone comes from deltaTone(change, moreIsWorse); direction is carried by the
 *  arrow glyph AND the aria-label (never color alone). */
export const DeltaTag = forwardRef<HTMLSpanElement, DeltaTagProps>(
  ({ value, pct, moreIsWorse = true, ariaLabel, className, ...props }, ref) => {
    const change = value ?? pct ?? 0;
    const tone = deltaTone(change, moreIsWorse);
    const dir = Math.abs(change) < 0.5 ? 'flat' : change > 0 ? 'up' : 'down';
    const Icon = dir === 'up' ? ArrowUp : dir === 'down' ? ArrowDown : Minus;

    const magnitude = value ?? pct ?? 0;
    const sign = magnitude > 0 ? '+' : magnitude < 0 ? '−' : '';
    const num = Math.abs(magnitude);
    const text = value !== undefined ? `${sign}${num}` : `${sign}${num}%`;

    const word = dir === 'up' ? 'increased' : dir === 'down' ? 'decreased' : 'unchanged';
    const label = ariaLabel ?? `${word} by ${num}${value !== undefined ? '' : ' percent'}`;

    return (
      <span
        ref={ref}
        aria-label={label}
        className={cn('inline-flex items-center gap-0.5 font-mono tabular-nums text-data-s leading-none', toneText[tone], className)}
        {...props}
      >
        <Icon aria-hidden className="size-3" strokeWidth={1.8} />
        <span aria-hidden>{text}</span>
      </span>
    );
  },
);
DeltaTag.displayName = 'DeltaTag';

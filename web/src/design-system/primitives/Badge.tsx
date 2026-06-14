import { forwardRef, type ComponentPropsWithoutRef, type ReactNode } from 'react';
import type { LucideIcon } from 'lucide-react';
import { cn } from '@/lib/cn';
import { type Tone, toneText, toneBorder } from '@/lib/tone';

export interface BadgeProps extends Omit<ComponentPropsWithoutRef<'span'>, 'children'> {
  tone?: Tone;
  /** Optional leading lucide icon (reinforces the tone — never color alone). */
  icon?: LucideIcon;
  mono?: boolean;
  ariaLabel?: string;
  children?: ReactNode;
}

/** Outlined status/tag pill — color on border + text, never a filled background. */
export const Badge = forwardRef<HTMLSpanElement, BadgeProps>(
  ({ tone = 'neutral', icon: Icon, mono = false, ariaLabel, className, children, ...props }, ref) => (
    <span
      ref={ref}
      aria-label={ariaLabel}
      className={cn(
        'inline-flex items-center gap-1 whitespace-nowrap rounded-pill border px-2 py-px text-data-s leading-none',
        mono && 'font-mono tabular-nums',
        toneText[tone],
        toneBorder[tone],
        className,
      )}
      {...props}
    >
      {Icon && <Icon aria-hidden className="size-3" strokeWidth={1.8} />}
      {children}
    </span>
  ),
);
Badge.displayName = 'Badge';

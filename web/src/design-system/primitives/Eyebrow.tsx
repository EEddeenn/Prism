import { forwardRef, type ComponentPropsWithoutRef } from 'react';
import { cn } from '@/lib/cn';

export type EyebrowProps = ComponentPropsWithoutRef<'p'>;

/** Tiny uppercase-mono metadata label (kicker above a heading). */
export const Eyebrow = forwardRef<HTMLParagraphElement, EyebrowProps>(
  ({ className, ...props }, ref) => (
    <p ref={ref} className={cn('metadata-label', className)} {...props} />
  ),
);
Eyebrow.displayName = 'Eyebrow';

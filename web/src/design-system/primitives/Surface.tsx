import { forwardRef, type ComponentPropsWithoutRef } from 'react';
import { cn } from '@/lib/cn';

type SurfaceVariant = 'page' | 'panel' | 'deep';

const variants: Record<SurfaceVariant, string> = {
  page: 'bg-deep-field text-bone',
  panel: 'border border-iron bg-charcoal text-bone',
  deep: 'bg-model-black text-bone',
};

export interface SurfaceProps extends ComponentPropsWithoutRef<'div'> {
  variant?: SurfaceVariant;
}

/** Lowest-level themed background block — pairs bg + fg so contrast is coherent. */
export const Surface = forwardRef<HTMLDivElement, SurfaceProps>(
  ({ variant = 'page', className, ...props }, ref) => (
    <div ref={ref} className={cn(variants[variant], className)} {...props} />
  ),
);
Surface.displayName = 'Surface';

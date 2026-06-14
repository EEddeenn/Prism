import { forwardRef, type ComponentPropsWithoutRef } from 'react';
import { cn } from '@/lib/cn';

export interface CardProps extends ComponentPropsWithoutRef<'div'> {
  /** Add the corner-bracket instrument registration mark. */
  bracket?: boolean;
}

/** Flat bordered panel — charcoal surface, iron hairline border, 2px corners,
 *  NO shadow. The basic record/section container. */
export const Card = forwardRef<HTMLDivElement, CardProps>(
  ({ bracket = false, className, ...props }, ref) => (
    <div
      ref={ref}
      className={cn('rounded-control border border-iron bg-charcoal', bracket && 'ds-bracket', className)}
      {...props}
    />
  ),
);
Card.displayName = 'Card';

export type HudProps = ComponentPropsWithoutRef<'div'>;

/** Floating overlay card for on-canvas chrome (legends, controls, hover cards).
 *  Charcoal/95 + faint backdrop-blur for legibility over the graph; no shadow. */
export const Hud = forwardRef<HTMLDivElement, HudProps>(
  ({ className, ...props }, ref) => (
    <div
      ref={ref}
      className={cn('rounded-control border border-iron bg-charcoal/95 backdrop-blur-sm', className)}
      {...props}
    />
  ),
);
Hud.displayName = 'Hud';

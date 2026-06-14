import { forwardRef, type ComponentPropsWithoutRef } from 'react';
import { cn } from '@/lib/cn';

export type TextLinkProps = ComponentPropsWithoutRef<'a'>;

/** Inline link — visible underline (not color alone), bone text, dust→bone hover. */
export const TextLink = forwardRef<HTMLAnchorElement, TextLinkProps>(
  ({ className, ...props }, ref) => (
    <a
      ref={ref}
      className={cn(
        'font-medium text-bone underline decoration-dust decoration-1 underline-offset-2 transition-colors duration-150 hover:decoration-bone',
        className,
      )}
      {...props}
    />
  ),
);
TextLink.displayName = 'TextLink';

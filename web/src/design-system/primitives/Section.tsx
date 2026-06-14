import { forwardRef, type ComponentPropsWithoutRef, type ReactNode } from 'react';
import { cn } from '@/lib/cn';

export interface SectionHeaderProps extends Omit<ComponentPropsWithoutRef<'div'>, 'title'> {
  count?: number;
  right?: ReactNode;
  as?: 'h3' | 'h4';
}

/** Uppercase-mono section label + optional count + right-aligned action/legend slot. */
export const SectionHeader = forwardRef<HTMLDivElement, SectionHeaderProps>(
  ({ count, right, as = 'h3', className, children, ...props }, ref) => {
    const Heading = as;
    return (
      <div ref={ref} className={cn('mb-1.5 mt-4 flex items-center gap-2 first:mt-0', className)} {...props}>
        <Heading className="metadata-label flex items-center gap-1.5">
          <span>{children}</span>
          {typeof count === 'number' && <span className="font-mono tabular-nums text-dust">{count}</span>}
        </Heading>
        {right && <div className="ml-auto flex items-center gap-1.5">{right}</div>}
      </div>
    );
  },
);
SectionHeader.displayName = 'SectionHeader';

export interface SectionProps extends Omit<ComponentPropsWithoutRef<'section'>, 'title'> {
  title?: ReactNode;
  count?: number;
  right?: ReactNode;
}

/** A titled content block: SectionHeader + children. */
export const Section = forwardRef<HTMLElement, SectionProps>(
  ({ title, count, right, className, children, ...props }, ref) => (
    <section ref={ref} className={cn('mb-4', className)} {...props}>
      {title && (
        <SectionHeader count={count} right={right}>
          {title}
        </SectionHeader>
      )}
      {children}
    </section>
  ),
);
Section.displayName = 'Section';

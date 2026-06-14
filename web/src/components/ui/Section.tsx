/* SectionHeader — the one uppercase muted heading used across the rail, replacing
 * the ~20 inline `text-[11px] uppercase tracking-wide text-muted` labels at three
 * drifting sizes. A real heading element for screen readers. */

import type { ReactNode } from 'react';

export function SectionHeader({
  count,
  right,
  as: As = 'h3',
  className = '',
  children,
}: {
  count?: number;
  /** A trailing slot (legend, action) pushed to the right. */
  right?: ReactNode;
  as?: 'h3' | 'h4';
  className?: string;
  children: ReactNode;
}) {
  return (
    <As
      className={`mt-4 first:mt-0 mb-1.5 flex items-center gap-1.5 text-label font-semibold uppercase tracking-wide text-muted ${className}`}
    >
      <span>{children}</span>
      {count != null && <span className="font-normal text-muted">({count})</span>}
      {right && <span className="ml-auto font-normal normal-case tracking-normal">{right}</span>}
    </As>
  );
}

/** A titled block: a SectionHeader followed by its content. */
export function Section({
  title,
  count,
  right,
  className = '',
  children,
}: {
  title: ReactNode;
  count?: number;
  right?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section className={className}>
      <SectionHeader count={count} right={right}>
        {title}
      </SectionHeader>
      {children}
    </section>
  );
}

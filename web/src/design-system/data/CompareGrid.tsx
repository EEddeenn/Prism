import { Fragment, type ReactNode } from 'react';
import { cn } from '@/lib/cn';

export interface CompareRow {
  label: ReactNode;
  /** Baseline column — muted. */
  a: ReactNode;
  /** Comparison column — emphasized. */
  b: ReactNode;
  /** Optional change indicator (caller passes a <DeltaTag>). */
  delta?: ReactNode;
}

export interface CompareGridProps {
  rows: CompareRow[];
  aLabel?: string;
  bLabel?: string;
  className?: string;
}

/** Side-by-side A|B value grid (Overview compare, Diff): A muted, B emphasized,
 *  with an optional delta column. Drops the delta column when no row has one. */
export function CompareGrid({ rows, aLabel = 'A', bLabel = 'B', className }: CompareGridProps) {
  const hasDelta = rows.some((r) => r.delta != null);
  return (
    <div
      className={cn(
        'grid items-baseline gap-x-3 gap-y-1 text-data-s',
        hasDelta ? 'grid-cols-[1fr_auto_auto_auto]' : 'grid-cols-[1fr_auto_auto]',
        className,
      )}
    >
      <span aria-hidden />
      <span className="metadata-label text-right">{aLabel}</span>
      <span className="metadata-label text-right">{bLabel}</span>
      {hasDelta && <span aria-hidden />}
      {rows.map((row, i) => (
        <Fragment key={i}>
          <span className="text-archive">{row.label}</span>
          <span className="text-right font-mono tabular-nums text-archive">{row.a}</span>
          <span className="text-right font-mono tabular-nums text-bone">{row.b}</span>
          {hasDelta && <span className="text-right">{row.delta}</span>}
        </Fragment>
      ))}
    </div>
  );
}

import { cn } from '@/lib/cn';

export interface TruncationNoteProps {
  /** How many items are currently shown. */
  shown: number;
  /** The total available. */
  total: number;
  /** When given, render a terse "…+N more" instead of the full "Showing" line. */
  more?: number;
  className?: string;
}

/** Faint mono footnote for truncated lists — "…+N more" or "Showing N of M". */
export function TruncationNote({ shown, total, more, className }: TruncationNoteProps) {
  return (
    <p className={cn('font-mono tabular-nums text-meta text-dust', className)}>
      {more !== undefined ? `…+${more} more` : `Showing ${shown} of ${total}`}
    </p>
  );
}

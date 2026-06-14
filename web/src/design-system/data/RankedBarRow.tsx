import { type ReactNode } from 'react';
import { cn } from '@/lib/cn';
import { type Tone } from '@/lib/tone';
import { ListRow } from '@/design-system/primitives/ListRow';
import { StatBar } from './Meter';

export interface RankedBarRowProps {
  /** Row label — can be a PurlLabel or any node. Rendered on top. */
  label: ReactNode;
  value: number;
  /** Denominator the bar normalizes against (usually the leader's value). */
  max: number;
  tone?: Tone;
  /** Optional secondary value (e.g. a unit, share, or count). */
  sub?: ReactNode;
  /** Optional trailing badge (e.g. a tone tag). */
  badge?: ReactNode;
  onSelect?: () => void;
  active?: boolean;
  title?: string;
  ariaLabel?: string;
}

/** Leaderboard row — label over a normalized StatBar, mono value, optional sub +
 *  badge. Becomes an accessible ListRow when `onSelect` is given, else a static
 *  block with matching padding. */
export function RankedBarRow({
  label,
  value,
  max,
  tone = 'info',
  sub,
  badge,
  onSelect,
  active,
  title,
  ariaLabel,
}: RankedBarRowProps) {
  const body = (
    <div className="flex flex-col gap-1">
      <div className="min-w-0">{label}</div>
      <div className="flex items-center gap-2">
        <StatBar value={value} max={max} tone={tone} ariaLabel={ariaLabel} className="flex-1" />
        <span className="font-mono tabular-nums text-data-s text-bone">{value}</span>
        {sub !== undefined && <span className="font-mono tabular-nums text-data-s text-archive">{sub}</span>}
        {badge}
      </div>
    </div>
  );

  if (onSelect) {
    return (
      <ListRow onSelect={onSelect} active={active} ariaLabel={ariaLabel} title={title}>
        {body}
      </ListRow>
    );
  }

  return (
    <div className="rounded-control border border-transparent px-2 py-1.5" title={title}>
      {body}
    </div>
  );
}

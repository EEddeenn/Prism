import { type ReactNode } from 'react';
import { Badge } from '@/design-system/primitives/Badge';
import { type Tone } from '@/lib/tone';

export interface MetricPillProps {
  label: ReactNode;
  value: ReactNode;
  tone?: Tone;
  title?: string;
}

/** Outlined label+value chip — a muted mono caption beside a mono tabular value,
 *  bordered like a Badge (color on border + text, never filled). */
export function MetricPill({ label, value, tone = 'neutral', title }: MetricPillProps) {
  return (
    <Badge tone={tone} mono title={title}>
      <span className="text-archive">{label}</span>
      <span className="tabular-nums">{value}</span>
    </Badge>
  );
}

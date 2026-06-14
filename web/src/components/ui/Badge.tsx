/* Badge — one pill with a single radius/padding/size, replacing the divergent
 * rounded/rounded-xl/rounded-lg chips hand-rolled across every panel. Tone maps
 * to the semantic token classes (no more bg-[#2a1414] hex pairs in markup). */

import type { ReactNode } from 'react';

export type Tone = 'neutral' | 'muted' | 'ok' | 'warn' | 'bad' | 'accent' | 'onAccent';

const TONE: Record<Tone, string> = {
  neutral: 'text-ink border-line-2 bg-panel-2',
  muted: 'text-muted border-line-2 bg-panel-3',
  ok: 'text-ok border-ok-bd bg-ok-bg',
  warn: 'text-warn border-warn-bd bg-warn-bg',
  bad: 'text-bad border-bad-bd bg-bad-bg',
  accent: 'text-accent border-accent-bd bg-panel-3',
  onAccent: 'text-onAccent border-transparent bg-accent',
};

export function Badge({
  tone = 'neutral',
  mono = false,
  title,
  ariaLabel,
  className = '',
  children,
}: {
  tone?: Tone;
  mono?: boolean;
  title?: string;
  ariaLabel?: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <span
      title={title}
      aria-label={ariaLabel}
      className={`inline-flex items-center gap-1 rounded-pill border px-2 py-px text-micro tabular-nums whitespace-nowrap ${
        mono ? 'font-mono' : ''
      } ${TONE[tone]} ${className}`}
    >
      {children}
    </span>
  );
}

/** A labelled metric pill ("blast 42") — a Badge with a muted label prefix. */
export function MetricPill({
  label,
  value,
  tone = 'muted',
  title,
}: {
  label: string;
  value: ReactNode;
  tone?: Tone;
  title?: string;
}) {
  return (
    <Badge tone={tone} title={title} ariaLabel={`${label}: ${value}`}>
      <span className="text-muted">{label}</span>
      <span className="tabular-nums">{value}</span>
    </Badge>
  );
}

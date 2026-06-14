import { type ReactNode } from 'react';
import { cn } from '@/lib/cn';
import { type Tone, toneText } from '@/lib/tone';

const SIZE = { md: 'text-figure', lg: 'text-figure-l' } as const;

export interface HeroFigureProps {
  value: ReactNode;
  unit?: ReactNode;
  label?: ReactNode;
  tone?: Tone;
  sub?: ReactNode;
  size?: keyof typeof SIZE;
}

/** A headline metric — a large tone-ramped mono tabular number with an optional
 *  trailing unit, an uppercase-mono caption, and an optional sub line. For the
 *  risk score, topology distance, and other single-value readouts. */
export function HeroFigure({ value, unit, label, tone = 'accent', sub, size = 'md' }: HeroFigureProps) {
  return (
    <div>
      <div className="flex items-baseline gap-1">
        <span className={cn('font-mono tabular-nums leading-none', SIZE[size], toneText[tone])}>{value}</span>
        {unit && <span className="font-mono text-data-s text-dust">{unit}</span>}
      </div>
      {label && <div className="metadata-label mt-1">{label}</div>}
      {sub && <div className="mt-0.5 text-data-s text-archive">{sub}</div>}
    </div>
  );
}

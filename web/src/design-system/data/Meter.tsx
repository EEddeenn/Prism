import { cn } from '@/lib/cn';
import { type Tone, toneFill } from '@/lib/tone';

/* Accessible track+fill meters — one progressbar idiom for every bar in the app.
 * Each exposes the aria so the signal isn't invisible to assistive tech, and the
 * fill resolves through the single Tone vocabulary (toneFill) rather than a local
 * per-component color map. */

const HEIGHT = { sm: 'h-0.5', md: 'h-1.5', lg: 'h-3' } as const;

export interface StatBarProps {
  value: number;
  max: number;
  tone?: Tone;
  height?: keyof typeof HEIGHT;
  ariaLabel?: string;
  className?: string;
}

/** A single horizontal meter, `value` of `max`, filled in `tone`. */
export function StatBar({ value, max, tone = 'info', height = 'sm', ariaLabel, className }: StatBarProps) {
  const pct = max > 0 ? Math.max(0, Math.min(100, (value / max) * 100)) : 0;
  return (
    <div
      role="progressbar"
      aria-valuenow={Math.round(value)}
      aria-valuemin={0}
      aria-valuemax={Math.round(max)}
      aria-label={ariaLabel}
      className={cn(HEIGHT[height], 'overflow-hidden rounded-control bg-charcoal', className)}
    >
      <div className={cn('h-full rounded-control', toneFill[tone])} style={{ width: `${pct}%` }} />
    </div>
  );
}

export interface DivergingBarProps {
  a: number;
  b: number;
  leftTone?: Tone;
  rightTone?: Tone;
  ariaLabel?: string;
}

/** Two values diverging from a centre line (A grows left, B grows right). */
export function DivergingBar({ a, b, leftTone = 'neutral', rightTone = 'info', ariaLabel }: DivergingBarProps) {
  const max = Math.max(a, b, 1e-9);
  return (
    <div className="mt-1 flex h-1.5 items-center" aria-label={ariaLabel} title={`A ${a} | B ${b}`}>
      <div className="flex flex-1 justify-end overflow-hidden rounded-l-control bg-charcoal">
        <div className={cn('h-full rounded-l-control', toneFill[leftTone])} style={{ width: `${(a / max) * 100}%` }} />
      </div>
      <div className="h-full w-px bg-graphite" />
      <div className="flex-1 overflow-hidden rounded-r-control bg-charcoal">
        <div className={cn('h-full rounded-r-control', toneFill[rightTone])} style={{ width: `${(b / max) * 100}%` }} />
      </div>
    </div>
  );
}

export interface Segment {
  value: number;
  tone: Tone;
  label: string;
}

export interface SegmentBarProps {
  segments: Segment[];
  ariaLabel?: string;
  showLegend?: boolean;
}

/** A horizontal stacked proportion bar — each segment's width is its share of the
 *  total. Optional inline legend pairs a toneFill dot with a mono label + count so
 *  the breakdown reads without relying on color alone. */
export function SegmentBar({ segments, ariaLabel, showLegend = false }: SegmentBarProps) {
  const total = segments.reduce((sum, s) => sum + s.value, 0);
  return (
    <div>
      <div
        role="img"
        aria-label={ariaLabel}
        className="flex h-1.5 overflow-hidden rounded-control bg-charcoal"
      >
        {segments.map((s, i) => (
          <div
            key={i}
            className={cn('h-full', toneFill[s.tone])}
            style={{ width: `${total > 0 ? (s.value / total) * 100 : 0}%` }}
          />
        ))}
      </div>
      {showLegend && (
        <ul className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1">
          {segments.map((s, i) => (
            <li key={i} className="flex items-center gap-1.5 text-data-s">
              <span aria-hidden className={cn('size-2 shrink-0 rounded-pill', toneFill[s.tone])} />
              <span className="font-mono text-archive">{s.label}</span>
              <span className="font-mono tabular-nums text-bone">{s.value}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export interface SparklineProps {
  data: number[];
  tone?: Tone;
  ariaLabel?: string;
}

/** A compact column sparkline (e.g. per-depth counts). */
export function Sparkline({ data, tone = 'info', ariaLabel }: SparklineProps) {
  const max = Math.max(1e-9, ...data);
  return (
    <div className="flex h-6 items-end gap-px" aria-label={ariaLabel} role="img">
      {data.map((d, i) => (
        <div
          key={i}
          className={cn('flex-1 rounded-control', toneFill[tone])}
          style={{ height: `${Math.max(6, (d / max) * 100)}%` }}
        />
      ))}
    </div>
  );
}

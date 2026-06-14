/* StatBar / DivergingBar / Sparkline — accessible track+fill meters replacing the
 * five hand-rolled bars with mismatched heights. Each exposes a progressbar role
 * and aria values so the signal isn't invisible to assistive tech. */

export type MeterTone = 'accent' | 'ok' | 'warn' | 'bad' | 'muted';

const FILL: Record<MeterTone, string> = {
  accent: 'bg-accent',
  ok: 'bg-ok',
  warn: 'bg-warn',
  bad: 'bg-bad',
  muted: 'bg-line-2',
};

const HEIGHT = { sm: 'h-1', md: 'h-1.5', lg: 'h-3' } as const;

/** A single horizontal meter, `value` of `max`, filled in `tone`. */
export function StatBar({
  value,
  max,
  tone = 'accent',
  height = 'md',
  ariaLabel,
  className = '',
}: {
  value: number;
  max: number;
  tone?: MeterTone;
  height?: keyof typeof HEIGHT;
  ariaLabel?: string;
  className?: string;
}) {
  const pct = max > 0 ? Math.max(0, Math.min(100, (value / max) * 100)) : 0;
  return (
    <div
      role="progressbar"
      aria-valuenow={Math.round(value)}
      aria-valuemin={0}
      aria-valuemax={Math.round(max)}
      aria-label={ariaLabel}
      className={`${HEIGHT[height]} bg-panel-3 rounded-badge overflow-hidden ${className}`}
    >
      <div className={`h-full rounded-badge ${FILL[tone]}`} style={{ width: `${pct}%` }} />
    </div>
  );
}

/** Two values diverging from a centre line (A grows left, B grows right). */
export function DivergingBar({
  a,
  b,
  leftTone = 'muted',
  rightTone = 'accent',
  ariaLabel,
}: {
  a: number;
  b: number;
  leftTone?: MeterTone;
  rightTone?: MeterTone;
  ariaLabel?: string;
}) {
  const max = Math.max(a, b, 1e-9);
  return (
    <div className="flex items-center h-1.5 mt-1" aria-label={ariaLabel} title={`A ${a} | B ${b}`}>
      <div className="flex-1 flex justify-end overflow-hidden bg-panel-3 rounded-l-control">
        <div className={`h-full rounded-l-control ${FILL[leftTone]}`} style={{ width: `${(a / max) * 100}%` }} />
      </div>
      <div className="w-px h-full bg-line-2" />
      <div className="flex-1 overflow-hidden bg-panel-3 rounded-r-control">
        <div className={`h-full rounded-r-control ${FILL[rightTone]}`} style={{ width: `${(b / max) * 100}%` }} />
      </div>
    </div>
  );
}

/** A compact column sparkline (e.g. per-depth counts). */
export function Sparkline({
  data,
  tone = 'accent',
  ariaLabel,
}: {
  data: number[];
  tone?: MeterTone;
  ariaLabel?: string;
}) {
  const max = Math.max(1e-9, ...data);
  return (
    <div className="flex items-end gap-px h-6" aria-label={ariaLabel} role="img">
      {data.map((d, i) => (
        <div
          key={i}
          className={`flex-1 rounded-badge ${FILL[tone]}`}
          style={{ height: `${Math.max(6, (d / max) * 100)}%` }}
        />
      ))}
    </div>
  );
}

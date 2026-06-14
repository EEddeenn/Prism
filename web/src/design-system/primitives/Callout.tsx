import { forwardRef, type ComponentPropsWithoutRef } from 'react';
import { Info, CheckCircle2, AlertTriangle, XCircle, type LucideIcon } from 'lucide-react';
import { cn } from '@/lib/cn';
import { type Tone, toneText, toneBorder, toneSurface } from '@/lib/tone';

type CalloutTone = Extract<Tone, 'info' | 'good' | 'warn' | 'bad' | 'neutral'>;

const CONFIG: Record<CalloutTone, { icon: LucideIcon; word: string; role: 'status' | 'alert' }> = {
  info: { icon: Info, word: 'Note', role: 'status' },
  good: { icon: CheckCircle2, word: 'OK', role: 'status' },
  warn: { icon: AlertTriangle, word: 'Warning', role: 'status' },
  bad: { icon: XCircle, word: 'Problem', role: 'alert' },
  neutral: { icon: Info, word: 'Note', role: 'status' },
};

export interface CalloutProps extends ComponentPropsWithoutRef<'div'> {
  tone?: CalloutTone;
  /** Render the tone word for screen readers (meaning is never color-only). */
  showWord?: boolean;
}

/** Tinted banner — glyph + (sr-only word) + tinted surface, tone-colored text. */
export const Callout = forwardRef<HTMLDivElement, CalloutProps>(
  ({ tone = 'info', role, showWord = false, className, children, ...props }, ref) => {
    const cfg = CONFIG[tone];
    const Icon = cfg.icon;
    return (
      <div
        ref={ref}
        role={role ?? cfg.role}
        className={cn(
          'flex items-start gap-1.5 rounded-control border px-2.5 py-1.5 text-data-s',
          toneText[tone],
          toneBorder[tone],
          toneSurface[tone],
          className,
        )}
        {...props}
      >
        <Icon aria-hidden className="mt-px size-3.5 shrink-0" strokeWidth={1.8} />
        {showWord && <span className="sr-only">{cfg.word}: </span>}
        <span className="min-w-0 flex-1">{children}</span>
      </div>
    );
  },
);
Callout.displayName = 'Callout';

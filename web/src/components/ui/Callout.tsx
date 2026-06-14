/* Callout — a tone-aware tinted banner replacing the ~39 copy-pasted hex callout
 * boxes. Meaning is never color-only: each tone leads with a glyph + word. */

import type { ReactNode } from 'react';

export type CalloutTone = 'info' | 'ok' | 'warn' | 'bad';

const TONE: Record<CalloutTone, { cls: string; glyph: string; word: string }> = {
  info: { cls: 'text-accent border-accent-bd bg-panel-2', glyph: 'ⓘ', word: 'Note' },
  ok: { cls: 'text-ok border-ok-bd bg-ok-bg', glyph: '✓', word: 'OK' },
  warn: { cls: 'text-warn border-warn-bd bg-warn-bg', glyph: '⚠', word: 'Heads up' },
  bad: { cls: 'text-bad border-bad-bd bg-bad-bg', glyph: '✕', word: 'Problem' },
};

export function Callout({
  tone = 'info',
  role,
  showWord = false,
  className = '',
  children,
}: {
  tone?: CalloutTone;
  /** Defaults to `status`; `bad` defaults to `alert`. Override when needed. */
  role?: 'status' | 'alert';
  /** Prefix the body with the tone word (for very terse callouts). */
  showWord?: boolean;
  className?: string;
  children: ReactNode;
}) {
  const t = TONE[tone];
  return (
    <div
      role={role ?? (tone === 'bad' ? 'alert' : 'status')}
      className={`flex items-start gap-1.5 rounded-lg border px-2.5 py-1.5 text-mini ${t.cls} ${className}`}
    >
      <span aria-hidden className="shrink-0 leading-tight pt-px">
        {t.glyph}
      </span>
      <span className="sr-only">{t.word}: </span>
      <div className="min-w-0">
        {showWord && <b className="font-semibold">{t.word} — </b>}
        {children}
      </div>
    </div>
  );
}

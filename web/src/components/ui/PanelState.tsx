/* PanelState — the one wrapper for the four panel states the reviews found
 * conflated: prompt (no run / no selection), loading, loaded-empty, and error.
 * Replaces the bare `<p className="text-muted text-sm">` messages and stops
 * panels showing a success/empty state before any run is analyzed. */

import type { ReactNode } from 'react';
import { Callout } from './Callout';

export function PanelState({
  kind,
  icon,
  cta,
  children,
}: {
  kind: 'prompt' | 'empty' | 'loading' | 'error';
  icon?: ReactNode;
  cta?: ReactNode;
  children: ReactNode;
}) {
  if (kind === 'loading') {
    return (
      <div className="flex flex-col gap-2 animate-pulse" aria-busy="true" aria-live="polite">
        <div className="h-2.5 w-1/2 rounded bg-panel-3" />
        <div className="h-2.5 w-3/4 rounded bg-panel-2" />
        <div className="h-2.5 w-2/3 rounded bg-panel-2" />
        <span className="sr-only">Loading…</span>
      </div>
    );
  }
  if (kind === 'error') {
    return <Callout tone="bad" role="alert">{children}</Callout>;
  }
  // prompt / empty: a quiet, centered hint.
  return (
    <div className="flex flex-col items-start gap-1 text-muted text-mini py-1">
      <div className="flex items-center gap-1.5">
        {icon && <span aria-hidden>{icon}</span>}
        <span>{children}</span>
      </div>
      {cta}
    </div>
  );
}

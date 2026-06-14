import { type ReactNode } from 'react';
import { cn } from '@/lib/cn';
import { Callout } from './Callout';

export interface PanelStateProps {
  kind: 'prompt' | 'empty' | 'loading' | 'error';
  icon?: ReactNode;
  cta?: ReactNode;
  className?: string;
  children?: ReactNode;
}

/** The four panel states: prompt/empty (quiet hint), loading (skeleton pulse),
 *  error (delegates to a bad Callout). */
export function PanelState({ kind, icon, cta, className, children }: PanelStateProps) {
  if (kind === 'loading') {
    return (
      <div className={cn('space-y-1.5', className)} aria-busy="true" aria-live="polite">
        {[0, 1, 2, 3].map((i) => (
          <div key={i} className="h-3 animate-pulse rounded-control bg-iron" style={{ width: `${90 - i * 14}%` }} />
        ))}
      </div>
    );
  }
  if (kind === 'error') {
    return (
      <Callout tone="bad" role="alert" showWord className={className}>
        {children}
      </Callout>
    );
  }
  return (
    <div className={cn('flex flex-col items-center gap-2 py-6 text-center text-data-s text-archive', className)}>
      {icon && (
        <div className="text-dust" aria-hidden>
          {icon}
        </div>
      )}
      <div className="max-w-[34ch]">{children}</div>
      {cta && <div className="mt-1">{cta}</div>}
    </div>
  );
}

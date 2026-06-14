import { type ReactNode } from 'react';
import { cn } from '@/lib/cn';
import { Card } from '@/design-system/primitives/Card';

export interface DataCardProps {
  title?: ReactNode;
  /** Uppercase-mono kicker above the title. */
  eyebrow?: ReactNode;
  /** Record count, rendered as a mono tabular-nums figure next to the title. */
  count?: number;
  /** Right-aligned header slot (actions, badges, legend). */
  right?: ReactNode;
  /** Optional nested sub-panel rendered below the body on the deepest surface. */
  footer?: ReactNode;
  /** Pass the corner-bracket registration mark through to the Card. */
  bracket?: boolean;
  className?: string;
  children?: ReactNode;
}

/** A titled record card (Cycles / Duplicates / Diff): bracketed Card with an
 *  eyebrow + title header (count + right slot), a padded body, and an optional
 *  nested footer sub-panel. */
export function DataCard({ title, eyebrow, count, right, footer, bracket, className, children }: DataCardProps) {
  const hasHeader = eyebrow != null || title != null || typeof count === 'number' || right != null;
  return (
    <Card bracket={bracket} className={className}>
      {hasHeader && (
        <div className="flex items-start justify-between gap-3 border-b border-graphite p-2">
          <div className="min-w-0">
            {eyebrow != null && <p className="metadata-label">{eyebrow}</p>}
            {(title != null || typeof count === 'number') && (
              <h4 className="flex items-baseline gap-1.5 text-h4 text-bone">
                {title}
                {typeof count === 'number' && (
                  <span className="font-mono tabular-nums text-dust">{count}</span>
                )}
              </h4>
            )}
          </div>
          {right != null && <div className="flex shrink-0 items-center gap-1.5">{right}</div>}
        </div>
      )}
      <div className="p-2">
        {children}
        {footer != null && <div className="mt-2 rounded-control bg-model-black p-2">{footer}</div>}
      </div>
    </Card>
  );
}

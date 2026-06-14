import { Fragment } from 'react';
import { cn } from '@/lib/cn';

export interface PathTrailItem {
  label: string;
  /** Full package URL for the native title tooltip on this segment. */
  purl?: string;
  onSelect?: () => void;
}

export interface PathTrailProps {
  items: PathTrailItem[];
  /** Separator glyph between segments (decorative, aria-hidden). */
  sep?: string;
  /** Index of the terminal/current segment. Defaults to the last item. */
  currentIndex?: number;
  ariaLabel?: string;
  className?: string;
}

/** A mono breadcrumb / chain (node path, why-path, cycle member chain). Each
 *  segment is a <button> when `onSelect` is set (else a span), joined by the
 *  separator glyph; the current segment carries aria-current="page". */
export function PathTrail({ items, sep = '›', currentIndex, ariaLabel, className }: PathTrailProps) {
  const current = currentIndex ?? items.length - 1;
  return (
    <nav aria-label={ariaLabel} className={cn('text-data-s font-mono', className)}>
      <ol className="flex flex-wrap items-center gap-1">
        {items.map((item, i) => {
          const isCurrent = i === current;
          return (
            <Fragment key={i}>
              {i > 0 && (
                <li aria-hidden className="text-dust">
                  {sep}
                </li>
              )}
              <li>
                {item.onSelect ? (
                  <button
                    type="button"
                    onClick={item.onSelect}
                    title={item.purl}
                    aria-current={isCurrent ? 'page' : undefined}
                    className={cn(
                      'truncate transition-colors duration-150 hover:text-cyan-gray',
                      isCurrent ? 'text-bone' : 'text-archive',
                    )}
                  >
                    {item.label}
                  </button>
                ) : (
                  <span
                    title={item.purl}
                    aria-current={isCurrent ? 'page' : undefined}
                    className={cn('truncate', isCurrent ? 'text-bone' : 'text-archive')}
                  >
                    {item.label}
                  </span>
                )}
              </li>
            </Fragment>
          );
        })}
      </ol>
    </nav>
  );
}

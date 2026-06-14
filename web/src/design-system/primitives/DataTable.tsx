import { type ReactNode } from 'react';
import { ChevronDown, ChevronUp } from 'lucide-react';
import { cn } from '@/lib/cn';

export interface DataTableColumn {
  key: string;
  header: ReactNode;
  align?: 'left' | 'right';
  sortable?: boolean;
}
export interface DataTableRow {
  key: string;
  cells: Record<string, ReactNode>;
  onSelect?: () => void;
  active?: boolean;
}
export type SortDir = 'asc' | 'desc';
export interface DataTableProps {
  caption: string;
  columns: DataTableColumn[];
  rows: DataTableRow[];
  sort?: { key: string; dir: SortDir };
  onSort?: (key: string) => void;
  captionVisible?: boolean;
  className?: string;
}

/** Schema-driven, accessible, sortable table. Header on the deepest surface,
 *  hairline row dividers (no zebra), numeric columns right-aligned mono
 *  tabular-nums, keyboard-focusable rows, sortable headers with aria-sort. */
export function DataTable({ caption, columns, rows, sort, onSort, captionVisible = false, className }: DataTableProps) {
  return (
    <div className={cn('overflow-x-auto', className)}>
      <table className="w-full border-collapse text-data-s">
        <caption className={cn('metadata-label mb-1 text-left', !captionVisible && 'sr-only')}>{caption}</caption>
        <thead>
          <tr className="bg-model-black">
            {columns.map((col) => {
              const isSorted = sort?.key === col.key;
              const ariaSort = isSorted
                ? sort!.dir === 'asc'
                  ? 'ascending'
                  : 'descending'
                : col.sortable
                  ? 'none'
                  : undefined;
              return (
                <th
                  key={col.key}
                  scope="col"
                  aria-sort={ariaSort}
                  className={cn(
                    'border-b border-iron px-2 py-1 text-meta font-medium text-archive',
                    col.align === 'right' ? 'text-right' : 'text-left',
                  )}
                >
                  {col.sortable && onSort ? (
                    <button
                      type="button"
                      onClick={() => onSort(col.key)}
                      className={cn('inline-flex items-center gap-1 hover:text-bone', col.align === 'right' && 'flex-row-reverse')}
                    >
                      {col.header}
                      {isSorted &&
                        (sort!.dir === 'asc' ? (
                          <ChevronUp aria-hidden className="size-3" />
                        ) : (
                          <ChevronDown aria-hidden className="size-3" />
                        ))}
                    </button>
                  ) : (
                    col.header
                  )}
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const interactive = !!row.onSelect;
            return (
              <tr
                key={row.key}
                tabIndex={interactive ? 0 : undefined}
                aria-current={row.active || undefined}
                onClick={row.onSelect}
                onKeyDown={
                  interactive
                    ? (e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault();
                          row.onSelect!();
                        }
                      }
                    : undefined
                }
                className={cn(
                  'border-b border-graphite',
                  interactive && 'cursor-pointer outline-offset-[-2px] hover:bg-charcoal',
                  row.active && 'bg-charcoal',
                )}
              >
                {columns.map((col) => (
                  <td
                    key={`${row.key}-${col.key}`}
                    className={cn(
                      'px-2 py-1 align-top',
                      col.align === 'right' ? 'text-right font-mono tabular-nums text-bone' : 'text-archive',
                    )}
                  >
                    {row.cells[col.key]}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

'use client';

import { useMemo } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { diffRuns, type FamilyDiff, type DiffStatus } from '@/lib/diff';

const ROW_CAP = 50;

// Per-section accent: added reads as ok/green, removed as bad/red, changed as warn/amber.
const SECTION: Record<DiffStatus, { title: string; tone: string; border: string; bar: string }> = {
  added: { title: 'Added', tone: 'text-ok', border: 'border-l-ok', bar: 'bg-ok' },
  removed: { title: 'Removed', tone: 'text-bad', border: 'border-l-bad', bar: 'bg-bad' },
  changed: { title: 'Changed', tone: 'text-warn', border: 'border-l-warn', bar: 'bg-warn' },
};

/** One family row: monospace name, version delta, blast tag. Click highlights its A-run ids. */
function DiffRow({ row, onPick }: { row: FamilyDiff; onPick: (r: FamilyDiff) => void }) {
  const onlyInB = row.status === 'added' || row.ids.length === 0;
  const clickable = !onlyInB;

  // Version delta: changed shows aVersions → bVersions with a direction arrow; added/removed
  // show just the side that exists.
  const delta =
    row.status === 'changed' ? (
      <span className="font-mono text-[11px] text-muted truncate">
        <span className="text-ink">{row.aVersions.join(', ')}</span>
        {row.direction > 0 ? (
          <span className="text-ok mx-1" title="version increased">{'▲ →'}</span>
        ) : row.direction < 0 ? (
          <span className="text-warn mx-1" title="version decreased">{'▼ →'}</span>
        ) : (
          <span className="text-muted mx-1" title="version set changed">{'↔ →'}</span>
        )}
        <span className="text-ink">{row.bVersions.join(', ')}</span>
      </span>
    ) : row.status === 'added' ? (
      <span className="font-mono text-[11px] text-ok truncate">{row.bVersions.join(', ')}</span>
    ) : (
      <span className="font-mono text-[11px] text-bad truncate">{row.aVersions.join(', ')}</span>
    );

  return (
    <div
      onClick={clickable ? () => onPick(row) : undefined}
      title={clickable ? 'Highlight these nodes in the active run' : 'This family exists only in the compare run'}
      className={`flex items-center gap-2 px-2.5 py-1.5 border-t border-line ${
        clickable ? 'cursor-pointer hover:bg-panel-3' : 'cursor-default'
      }`}
    >
      <div className="min-w-0 flex-1">
        <div className="font-mono text-xs truncate text-ink">{row.name}</div>
        <div className="mt-0.5 flex items-center gap-1.5">
          {delta}
          {onlyInB && <span className="text-[10px] text-muted shrink-0">(only in B)</span>}
        </div>
      </div>
      <span
        title="max transitive dependents (blast radius)"
        className="shrink-0 text-[10.5px] tabular-nums text-muted rounded-xl px-2 py-px border border-line-2 bg-panel-3"
      >
        blast {row.blast}
      </span>
    </div>
  );
}

function Section({
  status,
  rows,
  onPick,
}: {
  status: DiffStatus;
  rows: FamilyDiff[];
  onPick: (r: FamilyDiff) => void;
}) {
  const cfg = SECTION[status];
  const shown = rows.slice(0, ROW_CAP);
  const more = rows.length - shown.length;

  return (
    <div className={`mb-3 rounded-lg overflow-hidden bg-panel-2 border border-line border-l-2 ${cfg.border}`}>
      <div className="flex items-center gap-2 px-2.5 py-2">
        <span className={`text-[11px] uppercase tracking-wide font-semibold ${cfg.tone}`}>{cfg.title}</span>
        <span className="ml-auto text-[11px] tabular-nums text-muted">{rows.length}</span>
      </div>
      {rows.length === 0 ? (
        <div className="px-2.5 pb-2.5 pt-1 text-muted text-xs border-t border-line">None.</div>
      ) : (
        <>
          {shown.map((r) => (
            <DiffRow key={r.key} row={r} onPick={onPick} />
          ))}
          {more > 0 && (
            <div className="px-2.5 py-1.5 border-t border-line text-[11px] text-muted">… {more} more</div>
          )}
        </>
      )}
    </div>
  );
}

export default function DiffTab() {
  const nodes = useGraphStore((s) => s.nodes);
  const index = useGraphStore((s) => s.index);
  const compareData = useGraphStore((s) => s.compareData);
  const compareRun = useGraphStore((s) => s.compareRun);
  const highlightSet = useGraphStore((s) => s.highlightSet);

  const diff = useMemo(() => {
    if (!index || !compareData) return null;
    return diffRuns(nodes, index, compareData.nodes, compareData.index);
  }, [nodes, index, compareData]);

  if (!compareData || !diff) {
    return (
      <p className="text-muted text-sm">
        Pick a &lsquo;vs&rsquo; run in the top bar to diff two resolutions.
      </p>
    );
  }

  const addedN = diff.added.length;
  const removedN = diff.removed.length;
  const changedN = diff.changed.length;

  // changes-by-depth sparkline, normalized to the busiest depth.
  const depthRows = [...diff.byDepth.entries()].sort((a, b) => a[0] - b[0]);
  const maxDepthCount = Math.max(1, ...depthRows.map(([, c]) => c));

  const onPick = (r: FamilyDiff) => {
    if (r.ids.length) highlightSet(r.ids, r.name);
  };

  return (
    <div>
      {/* Summary header */}
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-sm mb-3">
        <span className="text-ok font-semibold tabular-nums">+{addedN} added</span>
        <span className="text-muted">·</span>
        <span className="text-bad font-semibold tabular-nums">−{removedN} removed</span>
        <span className="text-muted">·</span>
        <span className="text-warn font-semibold tabular-nums">~{changedN} changed</span>
        <span className="text-muted">·</span>
        <span className="text-muted tabular-nums">{diff.unchangedCount} unchanged</span>
      </div>

      <p className="text-muted text-xs mb-3">
        Active run vs. run <span className="font-mono text-ink">{compareRun ?? compareData.run}</span>. Click a
        changed/removed family to highlight its nodes in the active run.
      </p>

      {/* Changes by depth */}
      {depthRows.length > 0 && (
        <div className="mb-3 rounded-lg bg-panel-2 border border-line px-2.5 py-2">
          <div className="text-[11px] uppercase tracking-wide text-muted mb-1.5">Changes by depth</div>
          <div className="flex items-end gap-1 h-12">
            {depthRows.map(([depth, count]) => (
              <div key={depth} className="flex-1 min-w-0 flex flex-col items-center gap-0.5" title={`depth ${depth}: ${count} change${count === 1 ? '' : 's'}`}>
                <div className="w-full flex items-end h-9">
                  <div
                    className="w-full rounded-sm bg-accent"
                    style={{ height: `${Math.max(6, (count / maxDepthCount) * 100)}%` }}
                  />
                </div>
                <div className="text-[9px] text-muted tabular-nums leading-none">{depth}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Sections, most impactful first within each (rows are pre-sorted by blast). */}
      <Section status="added" rows={diff.added} onPick={onPick} />
      <Section status="removed" rows={diff.removed} onPick={onPick} />
      <Section status="changed" rows={diff.changed} onPick={onPick} />
    </div>
  );
}

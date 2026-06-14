'use client';

import { useMemo, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import FidelityBanner from '@/components/FidelityBanner';
import { shortLabel } from '@/lib/palette';
import { signalValue } from '@/lib/layout3d';
import { Callout, PanelState, RowButton, StatBar } from '@/components/ui';

/** A numeric column of the leaderboard. `key` indexes a Row's value bag. */
type ColKey = 'revReach' | 'fwdReach' | 'inDeg' | 'outDeg' | 'famSize' | 'depth' | 'score';

interface Col {
  key: ColKey;
  label: string;
  title: string;
}

const COLS: Col[] = [
  { key: 'revReach', label: 'blast', title: 'revReach — transitive dependents (blast radius)' },
  { key: 'fwdReach', label: 'reach', title: 'fwdReach — transitive dependencies' },
  { key: 'inDeg', label: 'in', title: 'inDeg — direct dependents' },
  { key: 'outDeg', label: 'out', title: 'outDeg — direct dependencies' },
  { key: 'famSize', label: 'fam', title: 'famSize — distinct versions of this package family' },
  { key: 'depth', label: 'depth', title: 'depth — distance from a root' },
  { key: 'score', label: 'risk', title: 'risk — blast × duplicate × depth composite' },
];

interface Row {
  id: number;
  label: string;
  purl: string;
  revReach: number;
  fwdReach: number;
  inDeg: number;
  outDeg: number;
  famSize: number;
  depth: number;
  score: number;
}

const CAP = 40;
const fmtScore = (x: number) => (x >= 100 ? String(Math.round(x)) : String(Math.round(x * 10) / 10));

/** Shared grid template: a flexible pkg column followed by seven numeric columns. */
const GRID = 'grid items-center gap-1.5 grid-cols-[minmax(0,1fr)_repeat(7,minmax(2.5rem,1fr))]';

export default function RiskTab() {
  const nodes = useGraphStore((s) => s.nodes);
  const index = useGraphStore((s) => s.index);
  const sel = useGraphStore((s) => s.sel);
  const selectNode = useGraphStore((s) => s.selectNode);
  const requestFocus = useGraphStore((s) => s.requestFocus);

  const [sortKey, setSortKey] = useState<ColKey>('revReach');

  const rows = useMemo<Row[]>(() => {
    if (!index) return [];
    const out: Row[] = [];
    for (const n of nodes) {
      const m = index.metrics.get(n.id);
      if (!m) continue;
      out.push({
        id: n.id,
        label: shortLabel(n.purl),
        purl: n.purl,
        revReach: m.revReach,
        fwdReach: m.fwdReach,
        inDeg: m.inDeg,
        outDeg: m.outDeg,
        famSize: m.famSize,
        depth: m.depth,
        score: signalValue(m, 'risk'),
      });
    }
    return out;
  }, [nodes, index]);

  // Per-column maxima for mini-bar normalisation (over the full set, not the page).
  const maxes = useMemo<Record<ColKey, number>>(() => {
    const acc: Record<ColKey, number> = {
      revReach: 0, fwdReach: 0, inDeg: 0, outDeg: 0, famSize: 0, depth: 0, score: 0,
    };
    for (const r of rows) for (const c of COLS) acc[c.key] = Math.max(acc[c.key], r[c.key]);
    return acc;
  }, [rows]);

  const ranked = useMemo(() => {
    const sorted = [...rows].sort(
      (a, b) => b[sortKey] - a[sortKey] || b.revReach - a.revReach || a.label.localeCompare(b.label),
    );
    return sorted.slice(0, CAP);
  }, [rows, sortKey]);

  if (!index) {
    return <PanelState kind="prompt">Pick a run to rank packages by blast radius.</PanelState>;
  }
  if (!rows.length) {
    return (
      <div>
        <FidelityBanner />
        <PanelState kind="empty">No nodes to rank.</PanelState>
      </div>
    );
  }

  return (
    <div>
      <FidelityBanner />

      <p className="text-muted text-xs mb-2">
        Packages ranked by <b className="text-ink">blast radius</b> — how much of the graph breaks if
        this one does. Click a header to re-sort, a row to inspect it.
      </p>

      {index.approximate && (
        <Callout tone="warn" className="mb-2">
          approximate (degree proxy on large graphs)
        </Callout>
      )}

      <div className="border border-line rounded-lg overflow-hidden bg-panel-2">
        {/* Column headers — a real header row whose sort controls are <button>s. */}
        <div role="row" className={`${GRID} bg-panel-3 text-muted font-mono text-label font-semibold px-2 py-1.5`}>
          <div role="columnheader" className="text-left">pkg</div>
          {COLS.map((c) => {
            const active = c.key === sortKey;
            return (
              <div
                key={c.key}
                role="columnheader"
                aria-sort={active ? 'descending' : 'none'}
                className="text-right"
              >
                <button
                  type="button"
                  title={c.title}
                  onClick={() => setSortKey(c.key)}
                  aria-pressed={active}
                  className={`inline-flex items-center justify-end gap-0.5 whitespace-nowrap select-none cursor-pointer hover:text-ink focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent rounded-sm ${
                    active ? 'text-accent font-bold' : 'font-semibold'
                  }`}
                >
                  {c.label}
                  <span aria-hidden className="inline-block w-1.5 text-accent">{active ? '▾' : ''}</span>
                </button>
              </div>
            );
          })}
        </div>

        {/* Data rows — each is a keyboard-actionable drill-in button. */}
        {ranked.map((r, i) => {
          const selected = r.id === sel;
          return (
            <RowButton
              key={r.id}
              active={selected}
              ariaLabel={`${r.label}: blast ${r.revReach}, reach ${r.fwdReach}, risk ${fmtScore(r.score)}`}
              title={r.purl}
              onSelect={() => {
                void selectNode(r.id);
                requestFocus(r.id);
              }}
              className={`${GRID} !rounded-none !border-x-0 !border-t border-line font-mono text-label`}
            >
              <span className="min-w-0 flex items-center">
                <span className="text-muted mr-1 tabular-nums">{i + 1}.</span>
                <span className={`truncate ${selected ? 'text-accent-2' : 'text-ink'}`}>{r.label}</span>
              </span>
              {COLS.map((c) => {
                const v = r[c.key];
                const active = c.key === sortKey;
                return (
                  <span key={c.key} className="flex flex-col items-end gap-0.5">
                    <span
                      className={`tabular-nums leading-none ${
                        active ? 'text-accent font-bold' : 'text-ink'
                      }`}
                    >
                      {c.key === 'score' ? fmtScore(v) : v}
                    </span>
                    <span aria-hidden className="block w-full">
                      <StatBar
                        value={v}
                        max={maxes[c.key] || 1}
                        tone={active ? 'accent' : 'muted'}
                        height="sm"
                        className="w-full !h-[2px]"
                      />
                    </span>
                  </span>
                );
              })}
            </RowButton>
          );
        })}
      </div>

      {rows.length > CAP && (
        <p className="text-muted text-label mt-2">
          Showing the top {CAP} of {rows.length} by {COLS.find((c) => c.key === sortKey)?.label}.
        </p>
      )}
    </div>
  );
}

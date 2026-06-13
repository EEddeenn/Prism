'use client';

import { useMemo, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import FidelityBanner from '@/components/FidelityBanner';
import { shortLabel } from '@/lib/palette';
import { signalValue } from '@/lib/layout3d';

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
    return <p className="text-muted text-sm">Pick a run.</p>;
  }
  if (!rows.length) {
    return (
      <div>
        <FidelityBanner />
        <p className="text-muted text-sm">No nodes to rank.</p>
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
        <div className="px-2.5 py-1.5 rounded-lg mb-2 text-[11px] bg-[#20180c] border border-[#4a3a1e] text-warn">
          approximate (degree proxy on large graphs)
        </div>
      )}

      <div className="border border-line rounded-lg overflow-hidden bg-panel-2">
        <table className="w-full border-collapse font-mono text-[11px]">
          <thead>
            <tr className="bg-panel-3 text-muted">
              <th className="text-left font-semibold px-2 py-1.5 sticky left-0 bg-panel-3">pkg</th>
              {COLS.map((c) => {
                const active = c.key === sortKey;
                return (
                  <th
                    key={c.key}
                    title={c.title}
                    onClick={() => setSortKey(c.key)}
                    className={`text-right font-semibold px-1.5 py-1.5 cursor-pointer select-none whitespace-nowrap hover:text-ink ${
                      active ? 'text-accent' : ''
                    }`}
                  >
                    {c.label}
                    <span className="ml-0.5 inline-block w-1.5 text-accent">{active ? '▾' : ''}</span>
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {ranked.map((r, i) => {
              const selected = r.id === sel;
              return (
                <tr
                  key={r.id}
                  title={r.purl}
                  onClick={() => {
                    void selectNode(r.id);
                    requestFocus(r.id);
                  }}
                  className={`cursor-pointer border-t border-line ${
                    selected ? 'bg-panel-3' : 'hover:bg-panel-3'
                  }`}
                >
                  <td className={`px-2 py-1 max-w-[120px] truncate sticky left-0 ${selected ? 'bg-panel-3' : 'bg-panel-2'}`}>
                    <span className="text-muted mr-1 tabular-nums">{i + 1}.</span>
                    <span className={selected ? 'text-accent-2' : 'text-ink'}>{r.label}</span>
                  </td>
                  {COLS.map((c) => {
                    const v = r[c.key];
                    const max = maxes[c.key] || 1;
                    const pct = Math.max(0, Math.min(100, (v / max) * 100));
                    const active = c.key === sortKey;
                    return (
                      <td key={c.key} className="px-1.5 py-1 align-middle">
                        <div className="flex flex-col items-end gap-0.5">
                          <span className={`tabular-nums leading-none ${active ? 'text-accent' : 'text-ink'}`}>
                            {c.key === 'score' ? fmtScore(v) : v}
                          </span>
                          <span className="block w-full h-[2px] rounded-full bg-line-2 overflow-hidden">
                            <span
                              className={`block h-full rounded-full ${active ? 'bg-accent' : 'bg-muted'}`}
                              style={{ width: `${pct}%` }}
                            />
                          </span>
                        </div>
                      </td>
                    );
                  })}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {rows.length > CAP && (
        <p className="text-muted text-[11px] mt-2">
          Showing the top {CAP} of {rows.length} by {COLS.find((c) => c.key === sortKey)?.label}.
        </p>
      )}
    </div>
  );
}

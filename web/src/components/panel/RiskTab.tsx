'use client';

import { useMemo, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import FidelityBanner from '@/components/FidelityBanner';
import { shortLabel } from '@/lib/palette';
import { signalValue } from '@/lib/layout3d';
import {
  Callout,
  DataTable,
  Eyebrow,
  PanelState,
  StatBar,
  TruncationNote,
  type DataTableColumn,
  type DataTableRow,
} from '@/design-system';
import { severity, type Tone } from '@/lib/tone';

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

/** Tone for a numeric cell's mini-bar: the risk column always rides its own
 *  severity ramp; the active sort column reads `info` (cyan-gray); the rest stay
 *  neutral. The sort arrow + bold value carry the active signal too (never color
 *  alone). */
function cellTone(key: ColKey, value: number, sortKey: ColKey): Tone {
  if (key === 'score') return severity(value);
  return key === sortKey ? 'info' : 'neutral';
}

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

  // Schema-driven columns: a left pkg column plus the seven sortable numerics.
  const columns = useMemo<DataTableColumn[]>(
    () => [
      { key: 'pkg', header: 'pkg', align: 'left' },
      ...COLS.map((c): DataTableColumn => ({
        key: c.key,
        header: (
          <span title={c.title} className="whitespace-nowrap">
            {c.label}
          </span>
        ),
        align: 'right',
        sortable: true,
      })),
    ],
    [],
  );

  const tableRows = useMemo<DataTableRow[]>(
    () =>
      ranked.map((r, i) => {
        const selected = r.id === sel;
        return {
          key: String(r.id),
          active: selected,
          onSelect: () => {
            void selectNode(r.id);
            requestFocus(r.id);
          },
          cells: {
            pkg: (
              <span className="flex min-w-0 items-center" title={r.purl}>
                <span className="mr-1 font-mono tabular-nums text-dust">{i + 1}.</span>
                <span className={`truncate font-mono ${selected ? 'text-node-center' : 'text-bone'}`}>
                  {r.label}
                </span>
              </span>
            ),
            ...Object.fromEntries(
              COLS.map((c) => {
                const v = r[c.key];
                const active = c.key === sortKey;
                return [
                  c.key,
                  <span key={c.key} className="flex flex-col items-end gap-0.5">
                    <span
                      className={`leading-none tabular-nums ${active ? 'font-bold text-bone' : 'text-bone'}`}
                    >
                      {c.key === 'score' ? fmtScore(v) : v}
                    </span>
                    <StatBar
                      value={v}
                      max={maxes[c.key] || 1}
                      tone={cellTone(c.key, v, sortKey)}
                      height="sm"
                      ariaLabel={`${c.label} ${c.key === 'score' ? fmtScore(v) : v}`}
                      className="w-full"
                    />
                  </span>,
                ];
              }),
            ),
          },
        };
      }),
    [ranked, sel, sortKey, maxes, selectNode, requestFocus],
  );

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

      <Eyebrow className="mb-1">Blast radius</Eyebrow>
      <p className="mb-2 text-data-s text-archive">
        Packages ranked by <b className="text-bone">blast radius</b> — how much of the graph breaks
        if this one does. Click a header to re-sort, a row to inspect it.
      </p>

      {index.approximate && (
        <Callout tone="warn" showWord className="mb-2">
          approximate (degree proxy on large graphs)
        </Callout>
      )}

      <DataTable
        caption="Packages ranked by blast radius and related metrics"
        columns={columns}
        rows={tableRows}
        sort={{ key: sortKey, dir: 'desc' }}
        onSort={(key) => setSortKey(key as ColKey)}
        className="border border-iron"
      />

      {rows.length > CAP && (
        <div className="mt-2 flex flex-wrap items-baseline gap-x-1">
          <TruncationNote shown={CAP} total={rows.length} />
          <span className="font-mono text-meta text-dust">
            by {COLS.find((c) => c.key === sortKey)?.label}
          </span>
        </div>
      )}
    </div>
  );
}

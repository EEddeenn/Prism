'use client';

import { useEffect, useMemo, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import FidelityBanner from '@/components/FidelityBanner';
import {
  Badge,
  MetricPill,
  ListRow,
  Section,
  SectionHeader,
  PanelState,
  Card,
  Legend,
  SearchField,
  DataTable,
  TruncationNote,
} from '@/design-system';
import { api, ApiError } from '@/lib/api';
import { CHART_COLORS, PALETTE, shortLabel } from '@/lib/palette';
import type { TdaPair, H2View, FiltrationSummary } from '@/lib/types';

const fmt = (x: number | null) => (x == null ? '?' : String(Math.round(x * 10) / 10));

// H0 = connected components (chart dim0) · H1 = loops (chart dim1 / amber) ·
// H2 = voids (chart dim2 / violet). The persistence diagram + barcode only plot
// s.pairs (H0/H1); H2 lives in its own flag-complex section below.
const DIM = {
  0: { stroke: CHART_COLORS.dim0, label: 'H0' },
  1: { stroke: CHART_COLORS.dim1, label: 'H1' },
  2: { stroke: CHART_COLORS.dim2, label: 'H2' },
} as const;
const dimColor = (dim: number) => (dim === 2 ? DIM[2].stroke : dim === 1 ? DIM[1].stroke : DIM[0].stroke);

const lifetime = (p: TdaPair) =>
  p.persistence ?? (p.death != null ? p.death - p.birth : Number.POSITIVE_INFINITY);

// ---------------------------------------------------------------------------
// (a) PERSISTENCE DIAGRAM — scatter each pair at (birth, death); y=x diagonal;
//     essentials pinned to a dashed ∞ rail at the top.
// ---------------------------------------------------------------------------
function PersistenceDiagram({
  pairs,
  maxDepth,
  activePair,
  onPick,
}: {
  pairs: TdaPair[];
  maxDepth: number;
  activePair: number | null;
  onPick: (p: TdaPair) => void;
}) {
  const [hover, setHover] = useState<number | null>(null);

  // Plot geometry in user-space px; integer filtration-depth units on both axes.
  const W = 232;
  const H = 200;
  const m = { l: 26, r: 10, t: 22, b: 22 }; // t leaves room for the ∞ rail
  const innerW = W - m.l - m.r;
  const innerH = H - m.t - m.b;

  // x/y both range [0, maxDepth] in integer filtration units.
  const sx = (v: number) => m.l + (innerW * v) / maxDepth;
  // y grows downward in SVG; map low depth -> bottom, high depth -> top.
  const sy = (v: number) => m.t + innerH - (innerH * v) / maxDepth;

  // Integer gridlines (cap tick count so dense graphs stay readable).
  const step = Math.max(1, Math.ceil(maxDepth / 8));
  const ticks: number[] = [];
  for (let d = 0; d <= maxDepth; d += step) ticks.push(d);

  const railY = m.t - 8; // dashed ∞ rail above the plot box.
  const hoverPair = hover != null ? pairs.find((p) => p.pair === hover) ?? null : null;

  return (
    <div className="relative">
      <svg viewBox={`0 0 ${W} ${H}`} className="w-full block" aria-hidden>
        {/* plot frame */}
        <rect x={m.l} y={m.t} width={innerW} height={innerH} fill={CHART_COLORS.frame} stroke={CHART_COLORS.grid} />

        {/* hairline integer grid */}
        {ticks.map((d) => (
          <g key={d}>
            <line x1={sx(d)} y1={m.t} x2={sx(d)} y2={m.t + innerH} stroke={CHART_COLORS.rail} strokeWidth={0.5} />
            <line x1={m.l} y1={sy(d)} x2={m.l + innerW} y2={sy(d)} stroke={CHART_COLORS.rail} strokeWidth={0.5} />
            <text x={sx(d)} y={m.t + innerH + 11} textAnchor="middle" className="fill-archive" fontSize={7}>
              {d}
            </text>
            <text x={m.l - 4} y={sy(d) + 2.5} textAnchor="end" className="fill-archive" fontSize={7}>
              {d}
            </text>
          </g>
        ))}

        {/* y = x diagonal — nothing dies before it is born */}
        <line x1={sx(0)} y1={sy(0)} x2={sx(maxDepth)} y2={sy(maxDepth)} stroke={CHART_COLORS.diagonal} strokeWidth={0.75} strokeDasharray="2 2" />

        {/* dashed ∞ rail for essentials */}
        <line x1={m.l} y1={railY} x2={m.l + innerW} y2={railY} stroke={CHART_COLORS.diagonal} strokeWidth={0.75} strokeDasharray="3 2" />
        <text x={m.l + innerW} y={railY - 3} textAnchor="end" className="fill-archive" fontSize={8}>
          ∞
        </text>

        {/* axis labels */}
        <text x={m.l} y={H - 4} className="fill-archive" fontSize={7}>
          birth →
        </text>
        <text x={m.l - 22} y={m.t + 6} className="fill-archive" fontSize={7}>
          death
        </text>

        {/* dots — draw essentials/active last so they sit on top */}
        {[...pairs]
          .sort((a, b) => Number(a.pair === activePair) - Number(b.pair === activePair))
          .map((p) => {
            const isActive = p.pair === activePair;
            const isHover = p.pair === hover;
            const cx = sx(p.birth);
            const cy = p.essential ? railY : sy(p.death ?? p.birth);
            const c = dimColor(p.dim);
            const r = isActive ? 4 : isHover ? 3.4 : 2.6;
            return (
              <circle
                key={p.pair}
                cx={cx}
                cy={cy}
                r={r}
                fill={c}
                fillOpacity={p.essential ? 0.95 : 0.8}
                stroke={isActive ? PALETTE.ink : isHover ? c : 'none'}
                strokeWidth={isActive ? 1.4 : 1}
                className="cursor-pointer"
                onMouseEnter={() => setHover(p.pair)}
                onMouseLeave={() => setHover((h) => (h === p.pair ? null : h))}
                onClick={() => onPick(p)}
              />
            );
          })}
      </svg>

      {hoverPair && (
        <div className="absolute top-0 right-0 px-2 py-1 rounded-control bg-charcoal/95 border border-iron backdrop-blur-sm text-meta font-mono tabular-nums text-bone pointer-events-none whitespace-nowrap">
          <span className="font-bold" style={{ color: dimColor(hoverPair.dim) }}>
            {DIM[hoverPair.dim === 1 ? 1 : 0].label}
          </span>{' '}
          birth d{fmt(hoverPair.birth)} ·{' '}
          {hoverPair.essential ? 'death ∞' : `death d${fmt(hoverPair.death)}`} · life{' '}
          {hoverPair.essential ? '∞' : fmt(hoverPair.persistence ?? lifetime(hoverPair))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// (b) BARCODE — one thin lane per pair spanning [birth, death]; sorted by
//     persistence desc within H0 then H1; essentials run to the right edge
//     with an arrow; persistence<=1 dimmed as noise.
// ---------------------------------------------------------------------------
function Barcode({
  pairs,
  maxDepth,
  activePair,
  onPick,
}: {
  pairs: TdaPair[];
  maxDepth: number;
  activePair: number | null;
  onPick: (p: TdaPair) => void;
}) {
  const lanes = useMemo(() => {
    const byPersist = (a: TdaPair, b: TdaPair) =>
      lifetime(b) - lifetime(a) || a.birth - b.birth || a.pair - b.pair;
    return [
      ...pairs.filter((p) => p.dim === 0).sort(byPersist),
      ...pairs.filter((p) => p.dim === 1).sort(byPersist),
    ];
  }, [pairs]);

  const W = 232;
  const m = { l: 6, r: 12, t: 6, b: 16 };
  const innerW = W - m.l - m.r;
  const laneH = 4;
  const gap = 1.5;
  const H = m.t + m.b + lanes.length * (laneH + gap);

  const sx = (v: number) => m.l + (innerW * v) / maxDepth;

  const step = Math.max(1, Math.ceil(maxDepth / 8));
  const ticks: number[] = [];
  for (let d = 0; d <= maxDepth; d += step) ticks.push(d);

  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full block" aria-hidden>
      {/* faint vertical depth gridlines + labels */}
      {ticks.map((d) => (
        <g key={d}>
          <line x1={sx(d)} y1={m.t} x2={sx(d)} y2={H - m.b} stroke={CHART_COLORS.rail} strokeWidth={0.5} />
          <text x={sx(d)} y={H - 5} textAnchor="middle" className="fill-archive" fontSize={7}>
            {d}
          </text>
        </g>
      ))}

      {lanes.map((p, i) => {
        const y = m.t + i * (laneH + gap);
        const isActive = p.pair === activePair;
        const noise = !p.essential && lifetime(p) <= 1;
        const c = dimColor(p.dim);
        const x0 = sx(p.birth);
        const x1 = p.essential ? m.l + innerW : sx(p.death ?? p.birth);
        const w = Math.max(1, x1 - x0);
        return (
          <g
            key={p.pair}
            className="cursor-pointer"
            onClick={() => onPick(p)}
            opacity={noise && !isActive ? 0.32 : 1}
          >
            {/* full-width hit target keeps thin lanes clickable */}
            <rect x={m.l} y={y - 0.5} width={innerW} height={laneH + 1} fill="transparent" />
            <rect
              x={x0}
              y={y}
              width={w}
              height={laneH}
              rx={1}
              fill={c}
              fillOpacity={isActive ? 1 : 0.85}
              stroke={isActive ? PALETTE.ink : 'none'}
              strokeWidth={isActive ? 1 : 0}
            />
            {p.essential && (
              // arrowhead at the right edge: never dies
              <path
                d={`M ${m.l + innerW} ${y + laneH / 2} l -3.5 -2.6 v5.2 z`}
                fill={c}
                fillOpacity={isActive ? 1 : 0.85}
              />
            )}
          </g>
        );
      })}
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Searchable / sorted feature list (preserved below the visuals).
// ---------------------------------------------------------------------------
function FeatureRow({ p }: { p: TdaPair }) {
  const feature = useGraphStore((s) => s.feature);
  const selectFeature = useGraphStore((s) => s.selectFeature);
  // A void (dim 2) reuses the `feature.pair` slot as its own array index — never
  // a barcode pair id — so it must not mark an H0/H1 row active.
  const active = feature != null && feature.dim !== 2 && feature.pair === p.pair;
  const desc = p.essential
    ? `born d${fmt(p.birth)} · never dies`
    : `d${fmt(p.birth)} → d${fmt(p.death)} · life ${fmt(p.persistence)}`;
  return (
    <ListRow
      onSelect={() => selectFeature(p.pair, p.dim)}
      active={active}
      ariaLabel={`H${p.dim} feature ${p.pair}, ${desc}, ${p.essential ? 'essential' : 'finite'}`}
      className="my-1 flex items-center gap-2"
    >
      <span className="shrink-0 font-mono font-bold text-data-s min-w-[22px]" style={{ color: dimColor(p.dim) }}>
        H{p.dim}
      </span>
      <span className="flex-1 font-mono tabular-nums text-data-s">{desc}</span>
      {p.essential ? (
        <Badge tone="warn">essential</Badge>
      ) : (
        <Badge tone="neutral">finite</Badge>
      )}
    </ListRow>
  );
}

function Group({ title, list }: { title: string; list: TdaPair[] }) {
  if (!list.length) return null;
  const sorted = [...list].sort(
    (a, b) => Number(b.essential) - Number(a.essential) || (b.persistence ?? 0) - (a.persistence ?? 0) || a.pair - b.pair,
  );
  const CAP = 40;
  return (
    <div>
      <SectionHeader as="h4" count={list.length}>
        {title}
      </SectionHeader>
      {sorted.slice(0, CAP).map((p) => (
        <FeatureRow key={p.pair} p={p} />
      ))}
      {sorted.length > CAP && (
        <p className="mt-1.5 font-mono tabular-nums text-meta text-dust">
          …+{sorted.length - CAP} more (showing the {CAP} longest-lived).
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// (c) VOIDS (H₂) — flag-complex homology, computed independently of the
//     filtration barcode. Lazily fetched per run; each void is a cycle of
//     package nodes bounding a 2-dimensional hole in the dependency graph.
// ---------------------------------------------------------------------------
function VoidsSection({ run }: { run: number }) {
  const selectVoid = useGraphStore((s) => s.selectVoid);
  const [h2, setH2] = useState<H2View | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true); // we always fetch on mount

  useEffect(() => {
    let live = true;
    setLoading(true);
    setError(null);
    setH2(null);
    api
      .h2(run)
      .then((v) => {
        if (!live) return;
        setH2(v);
      })
      .catch((e) => {
        if (!live) return;
        setError(e instanceof ApiError ? e.message : 'failed to load flag-complex homology');
      })
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [run]);

  const CAP = 24;

  return (
    <Section className="mt-4">
      <SectionHeader right={<Badge tone="neutral">experimental</Badge>}>Voids (H₂)</SectionHeader>
      <p className="mb-1.5 text-data-s text-archive">
        Flag-complex homology — computed independently of the filtration barcode above, so its H₁ may differ.
      </p>

      {error ? (
        <PanelState kind="error">{error}</PanelState>
      ) : loading && !h2 ? (
        <PanelState kind="loading">Computing flag-complex homology…</PanelState>
      ) : h2 ? (
        <>
          <div className="flex flex-wrap items-center gap-1.5 mb-2">
            <MetricPill label="H₀" value={h2.betti0} tone="info" title="Connected components (flag complex)" />
            <MetricPill label="H₁" value={h2.betti1} tone="warn" title="Independent loops (flag complex)" />
            <MetricPill
              label="H₂ voids"
              value={h2.betti2}
              tone={h2.betti2 > 0 ? 'accent' : 'neutral'}
              title="2-dimensional voids — hollow shells in the dependency graph"
            />
            <MetricPill label="triangles" value={h2.triangles} tone="neutral" title="2-simplices in the flag complex" />
          </div>

          {h2.betti2 === 0 || h2.voids.length === 0 ? (
            <PanelState kind="empty">No 2-dimensional voids — no hollow shells in the dependency graph.</PanelState>
          ) : (
            <ul className="flex flex-col gap-1">
              {h2.voids.slice(0, CAP).map((members, i) => {
                const preview = members.slice(0, 3).map((n) => shortLabel(n.purl)).join(' · ');
                return (
                  <li key={i}>
                    <ListRow
                      onSelect={() => selectVoid(i, members)}
                      ariaLabel={`Void ${i + 1}, ${members.length} packages: ${preview}`}
                      className="flex items-center gap-2"
                    >
                      <span
                        className="shrink-0 font-mono font-bold text-data-s min-w-[22px]"
                        style={{ color: DIM[2].stroke }}
                      >
                        H₂
                      </span>
                      <span className="shrink-0 font-mono tabular-nums text-data-s text-archive min-w-[28px]">
                        {members.length}
                      </span>
                      <span className="flex-1 font-mono text-data-s truncate">{preview}</span>
                    </ListRow>
                  </li>
                );
              })}
              {h2.voids.length > CAP && (
                <li>
                  <TruncationNote className="mt-0.5" shown={CAP} total={h2.voids.length} more={h2.voids.length - CAP} />
                </li>
              )}
            </ul>
          )}
        </>
      ) : (
        <PanelState kind="empty">No flag-complex homology for this run.</PanelState>
      )}
    </Section>
  );
}

// ---------------------------------------------------------------------------
// (d) FILTRATION COMPARISON — Betti numbers are invariant across lenses; only
//     the persistence (how long-lived features are) differs. Lazily fetched.
// ---------------------------------------------------------------------------
function FiltrationComparison({ run }: { run: number }) {
  const [rows, setRows] = useState<FiltrationSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let live = true;
    setLoading(true);
    setError(null);
    setRows([]);
    api
      .tdaCompare(run)
      .then((r) => {
        if (!live) return;
        setRows(r);
      })
      .catch((e) => {
        if (!live) return;
        setError(e instanceof ApiError ? e.message : 'failed to load filtration comparison');
      })
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [run]);

  return (
    <Section className="mt-4">
      <SectionHeader>Filtration comparison</SectionHeader>

      {error ? (
        <PanelState kind="error">{error}</PanelState>
      ) : loading && rows.length === 0 ? (
        <PanelState kind="loading">Comparing filtration lenses…</PanelState>
      ) : rows.length === 0 ? (
        <PanelState kind="empty">No filtration comparison available for this run.</PanelState>
      ) : (
        <>
          <DataTable
            caption="Filtration comparison — Betti numbers and persistence per lens"
            columns={[
              { key: 'filtration', header: 'filtration', align: 'left' },
              { key: 'h0', header: 'H₀', align: 'right' },
              { key: 'h1', header: 'H₁', align: 'right' },
              { key: 'finite', header: 'finite bars', align: 'right' },
              { key: 'sum', header: 'Σ persistence', align: 'right' },
              { key: 'max', header: 'max persistence', align: 'right' },
            ]}
            rows={rows.map((r) => ({
              key: r.filtration,
              cells: {
                filtration: <span className="font-mono">{r.filtration}</span>,
                h0: r.h0_components,
                h1: r.h1_loops,
                finite: r.finite_bars,
                sum: fmt(r.total_persistence),
                max: fmt(r.max_persistence),
              },
            }))}
          />
          <p className="mt-1 text-meta text-archive">
            H₀/H₁ Betti numbers are invariant across lenses; persistence shows how long-lived features are under each
            filtration (depth / centrality / scope).
          </p>
        </>
      )}
    </Section>
  );
}

export default function TdaTab() {
  const barcode = useGraphStore((s) => s.barcode);
  const pairs = useGraphStore((s) => s.pairs);
  const run = useGraphStore((s) => s.run);
  const stats = useGraphStore((s) => s.stats);
  const feature = useGraphStore((s) => s.feature);
  const selectFeature = useGraphStore((s) => s.selectFeature);
  const statusText = useGraphStore((s) => s.statusText);
  const statusError = useGraphStore((s) => s.statusError);

  const [q, setQ] = useState('');

  // x/y bound: integer filtration-depth units. Take the max of the graph depth
  // AND every birth/finite-death so no glyph ever falls outside the plot box
  // (a feature can be born or die deeper than the graph's reported max depth).
  const maxDepth = useMemo(() => {
    const dataMax = pairs.reduce((mx, p) => Math.max(mx, p.birth, p.death ?? 0), 0);
    return Math.max(1, stats?.max_depth ?? 0, dataMax) + 1;
  }, [stats?.max_depth, pairs]);

  // Only an H0/H1 generator marks a barcode/diagram glyph active; a void (dim 2)
  // carries an array index here, not a pair id, so it never lights a plotted pair.
  const activePair = feature != null && feature.dim !== 2 ? feature.pair : null;

  const filtered = useMemo(() => {
    const t = q.trim().toLowerCase();
    if (!t) return pairs;
    return pairs.filter((p) => {
      const tag = `h${p.dim}`;
      const kind = p.essential ? 'essential infinite' : 'finite';
      return (
        tag.includes(t) ||
        kind.includes(t) ||
        String(p.pair).includes(t) ||
        `d${fmt(p.birth)}`.includes(t) ||
        `d${fmt(p.death)}`.includes(t)
      );
    });
  }, [pairs, q]);

  // No run picked yet → prompt; a run is selected but its homology is still
  // streaming in → loading. Distinguish both from a run that genuinely has no
  // persistent homology (the !barcode empty branch below).
  if (run == null) {
    return (
      <div>
        <FidelityBanner />
        <PanelState kind="prompt">Pick a run to compute its persistent homology.</PanelState>
      </div>
    );
  }

  if (!barcode) {
    const loading = !statusError && /load/i.test(statusText);
    if (loading) {
      return (
        <div>
          <FidelityBanner />
          <PanelState kind="loading">Computing persistent homology…</PanelState>
        </div>
      );
    }
    return (
      <div>
        <FidelityBanner />
        <PanelState
          kind="empty"
          cta={
            <code className="bg-charcoal px-1.5 rounded-control font-mono text-data-s">prism tda run --run {run}</code>
          }
        >
          No persistent homology for this run yet.
        </PanelState>
      </div>
    );
  }

  const h0 = pairs.filter((p) => p.dim === 0);
  const h1 = pairs.filter((p) => p.dim === 1);

  return (
    <div>
      <FidelityBanner />

      {/* H0/H1 component/cycle summary counts */}
      <div className="flex flex-wrap items-center gap-1.5 mb-2">
        <MetricPill
          label="H₀ components"
          value={barcode.component_count}
          tone="info"
          title="Connected components (H₀)"
        />
        <MetricPill
          label="H₁ cycles"
          value={barcode.cycle_count}
          tone="warn"
          title="Independent loops (H₁)"
        />
      </div>

      {pairs.length > 0 ? (
        <>
          <SectionHeader>Persistence diagram</SectionHeader>
          <Card className="p-1.5">
            <PersistenceDiagram pairs={pairs} maxDepth={maxDepth} activePair={activePair} onPick={(p) => selectFeature(p.pair, p.dim)} />
          </Card>
          <p className="mt-1 text-data-s text-archive">
            Each dot is a feature at (birth, death); distance above the diagonal is persistence.
            Dots on the <span className="font-mono">∞</span> rail never die.
          </p>

          <SectionHeader>Barcode</SectionHeader>
          <Card className="p-1.5">
            <Barcode pairs={pairs} maxDepth={maxDepth} activePair={activePair} onPick={(p) => selectFeature(p.pair, p.dim)} />
          </Card>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5">
            <Legend
              items={[
                { color: DIM[0].stroke, label: 'H0 components' },
                { color: DIM[1].stroke, label: 'H1 loops' },
              ]}
            />
            <span className="font-mono text-meta text-archive">→ never dies</span>
            <span className="font-mono text-meta text-archive">dim = noise (life ≤ 1)</span>
          </div>
        </>
      ) : (
        <PanelState kind="empty">No individual feature pairs returned for this run.</PanelState>
      )}

      {pairs.length > 0 && (
        <>
          <SectionHeader
            className="mt-4"
            right={
              <SearchField
                value={q}
                onChange={(e) => setQ(e.target.value)}
                placeholder="search H0 / H1 / d3 / essential…"
                aria-label="Search features"
                className="w-[180px] text-data-s"
              />
            }
          >
            Features
          </SectionHeader>
          <p className="mb-1 text-data-s text-archive">Click a feature to light up the packages that carry it (opens 3D · TDA).</p>

          {q.trim() ? (
            filtered.length ? (
              <Group title="Matches" list={filtered} />
            ) : (
              <PanelState kind="empty">No features match “{q.trim()}”.</PanelState>
            )
          ) : (
            <>
              <Group title="H₀ — connected components" list={h0} />
              <Group title="H₁ — loops" list={h1} />
            </>
          )}
        </>
      )}

      <VoidsSection run={run} />
      <FiltrationComparison run={run} />
    </div>
  );
}

'use client';

import { useGraphStore } from '@/store/useGraphStore';
import type { CompareData } from '@/store/useGraphStore';
import type { Barcode, GraphStats, RunMeta } from '@/lib/types';
import { Badge, DivergingBar, PanelState, SectionHeader, type Tone } from '@/components/ui';

/** Fidelity tier + caveats for one run, rendered as a single chip row. */
function FidelityChips({ meta }: { meta: RunMeta | null }) {
  if (!meta) return <span className="text-muted text-mini">—</span>;
  return (
    <div className="flex flex-wrap gap-1.5">
      <Badge tone={meta.fidelity_tier === 'lockfile-exact' ? 'ok' : 'warn'}>
        <b className="font-semibold">fidelity</b>
        {meta.fidelity_tier}
      </Badge>
      {meta.fidelity_caveats.map((c) => (
        <Badge key={c} tone="warn">
          {c}
        </Badge>
      ))}
    </div>
  );
}

// Metrics where a larger value reads as worse (more risk). Others are neutral.
const MORE_IS_WORSE = new Set(['edges', 'scc_count', 'max_depth', 'duplicate_version_count', 'cycle_count']);

type MetricRow = {
  key: string;
  label: string;
  a: number | null;
  b: number | null;
};

function numericRows(
  aStats: GraphStats | null,
  bStats: GraphStats | null,
  aBar: Barcode | null,
  bBar: Barcode | null,
): MetricRow[] {
  return [
    { key: 'nodes', label: 'nodes', a: aStats?.nodes ?? null, b: bStats?.nodes ?? null },
    { key: 'edges', label: 'edges', a: aStats?.edges ?? null, b: bStats?.edges ?? null },
    { key: 'scc_count', label: 'SCCs', a: aStats?.scc_count ?? null, b: bStats?.scc_count ?? null },
    { key: 'max_depth', label: 'max depth', a: aStats?.max_depth ?? null, b: bStats?.max_depth ?? null },
    {
      key: 'duplicate_version_count',
      label: 'duplicate families',
      a: aStats?.duplicate_version_count ?? null,
      b: bStats?.duplicate_version_count ?? null,
    },
    { key: 'component_count', label: 'components', a: aBar?.component_count ?? null, b: bBar?.component_count ?? null },
    { key: 'cycle_count', label: 'cycles', a: aBar?.cycle_count ?? null, b: bBar?.cycle_count ?? null },
  ];
}

/** Signed delta badge (B − A) with a direction tint (more = worse for risk metrics). */
function DeltaBadge({ delta, worseUp }: { delta: number; worseUp: boolean }) {
  if (delta === 0) {
    return <Badge tone="muted">±0</Badge>;
  }
  const sign = delta > 0 ? '+' : '−';
  const mag = Math.abs(delta);
  // For risk metrics an increase is bad; for neutral metrics any change stays muted-accent.
  const bad = worseUp && delta > 0;
  const good = worseUp && delta < 0;
  const tone: Tone = bad ? 'bad' : good ? 'ok' : 'accent';
  return (
    <Badge tone={tone}>
      {sign}
      {mag}
    </Badge>
  );
}

function CompareView({
  meta,
  stats,
  barcode,
  compare,
}: {
  meta: RunMeta | null;
  stats: GraphStats | null;
  barcode: Barcode | null;
  compare: CompareData;
}) {
  const aRun = stats?.run ?? meta?.run;
  const bRun = compare.run;
  const rows = numericRows(stats, compare.stats, barcode, compare.barcode);

  const aCycle = stats?.has_cycle ?? null;
  const bCycle = compare.stats?.has_cycle ?? null;

  return (
    <div>
      {/* A | B column headers */}
      <div className="grid grid-cols-[1fr_auto_auto] gap-x-3 items-baseline mb-2">
        <div />
        <div className="text-right text-label uppercase tracking-wide text-muted font-semibold">
          A {aRun != null ? `· run ${aRun}` : ''}
        </div>
        <div className="text-right text-label uppercase tracking-wide text-accent font-semibold">
          B {bRun != null ? `· run ${bRun}` : ''}
        </div>
      </div>

      <SectionHeader>Graph · A | B</SectionHeader>
      <div className="space-y-2">
        {rows.map((r) => {
          const worseUp = MORE_IS_WORSE.has(r.key);
          const hasBoth = r.a != null && r.b != null;
          const delta = hasBoth ? (r.b as number) - (r.a as number) : null;
          return (
            <div key={r.key}>
              <div className="grid grid-cols-[1fr_auto_auto] gap-x-3 items-center text-body">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="text-muted truncate">{r.label}</span>
                  {delta != null && <DeltaBadge delta={delta} worseUp={worseUp} />}
                </div>
                <div className="text-right tabular-nums font-semibold text-ink">
                  {r.a != null ? r.a : '—'}
                </div>
                <div className="text-right tabular-nums font-semibold text-accent">
                  {r.b != null ? r.b : '—'}
                </div>
              </div>
              {hasBoth && (
                <DivergingBar
                  a={r.a as number}
                  b={r.b as number}
                  leftTone={worseUp && (r.a as number) > (r.b as number) ? 'warn' : 'muted'}
                  rightTone={worseUp && (r.b as number) > (r.a as number) ? 'warn' : 'accent'}
                  ariaLabel={`${r.label}: A ${r.a}, B ${r.b}`}
                />
              )}
            </div>
          );
        })}

        {/* has_cycle: yes/no per run, not a numeric delta. */}
        <div className="grid grid-cols-[1fr_auto_auto] gap-x-3 items-center text-body">
          <div className="flex items-center gap-2 min-w-0">
            <span className="text-muted truncate">has cycle</span>
            {aCycle != null && bCycle != null && aCycle !== bCycle && (
              <Badge tone={bCycle && !aCycle ? 'bad' : 'ok'}>
                {bCycle && !aCycle ? 'gained' : 'resolved'}
              </Badge>
            )}
          </div>
          <div className={`text-right font-semibold ${aCycle ? 'text-warn' : 'text-ink'}`}>
            {aCycle == null ? '—' : aCycle ? 'yes' : 'no'}
          </div>
          <div className={`text-right font-semibold ${bCycle ? 'text-warn' : 'text-accent'}`}>
            {bCycle == null ? '—' : bCycle ? 'yes' : 'no'}
          </div>
        </div>
      </div>

      {/* Fidelity for BOTH runs as two chip rows — a downgrade in B is obvious. */}
      <SectionHeader>Fidelity · A | B</SectionHeader>
      <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 items-start">
        <span className="text-muted font-semibold text-label uppercase tracking-wide pt-1">A</span>
        <FidelityChips meta={meta} />
        <span className="text-accent font-semibold text-label uppercase tracking-wide pt-1">B</span>
        <FidelityChips meta={compare.runMeta} />
      </div>
    </div>
  );
}

export default function OverviewTab() {
  const meta = useGraphStore((s) => s.runMeta);
  const stats = useGraphStore((s) => s.stats);
  const barcode = useGraphStore((s) => s.barcode);
  const compareData = useGraphStore((s) => s.compareData);

  const compareMode = compareData != null;

  return (
    <div>
      {meta && (
        <div className="flex flex-wrap gap-1.5 mb-2">
          <Badge tone="accent">
            <b className="font-semibold">ecosystem</b>
            {meta.ecosystem}
          </Badge>
          {meta.resolver && (
            <Badge>
              <b className="font-semibold">resolver</b>
              {meta.resolver}
            </Badge>
          )}
          {/* Fidelity tier + caveats only outside compare mode — CompareView
              shows them per-run (A | B) so a downgrade in B stays obvious. */}
          {!compareMode && (
            <>
              <Badge tone={meta.fidelity_tier === 'lockfile-exact' ? 'ok' : 'warn'}>
                <b className="font-semibold">fidelity</b>
                {meta.fidelity_tier}
              </Badge>
              {meta.fidelity_caveats.map((c) => (
                <Badge key={c} tone="warn">
                  {c}
                </Badge>
              ))}
            </>
          )}
        </div>
      )}

      {compareMode ? (
        <CompareView meta={meta} stats={stats} barcode={barcode} compare={compareData} />
      ) : (
        <>
          <SectionHeader>Graph</SectionHeader>
          {stats ? (
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-body">
              {(
                [
                  ['nodes', String(stats.nodes), false],
                  ['edges', String(stats.edges), false],
                  ['SCCs', String(stats.scc_count), false],
                  ['has cycle', stats.has_cycle ? 'yes' : 'no', stats.has_cycle],
                  ['max depth', String(stats.max_depth), false],
                  ['duplicate families', String(stats.duplicate_version_count), stats.duplicate_version_count > 0],
                ] as [string, string, boolean][]
              ).map(([k, v, flag]) => (
                <div key={k} className="contents">
                  <dt className="text-muted">{k}</dt>
                  <dd className={`text-right tabular-nums font-semibold ${flag ? 'text-warn' : ''}`}>{v}</dd>
                </div>
              ))}
            </dl>
          ) : (
            <PanelState kind="prompt">Pick a run to see its graph stats.</PanelState>
          )}
        </>
      )}

      <SectionHeader>How to read it</SectionHeader>
      <ul className="list-disc pl-4 text-muted text-xs space-y-1">
        <li>Click a node to trace what it <b className="text-ink">depends on</b> and what <b className="text-ink">depends on it</b>.</li>
        <li><b className="text-ink">2D</b> is a layered graph; <b className="text-ink">3D</b> is a spatial topology.</li>
        <li>In 3D: <b className="text-ink">Layered</b> stacks by depth, <b className="text-ink">Fiber</b> by version, <b className="text-ink">Terrain</b> by a signal, <b className="text-ink">TDA</b> lights up a feature.</li>
      </ul>
    </div>
  );
}

'use client';

import { useGraphStore } from '@/store/useGraphStore';
import type { CompareData } from '@/store/useGraphStore';
import type { Barcode, GraphStats, RunMeta } from '@/lib/types';
import {
  Badge,
  CompareGrid,
  DeltaTag,
  DivergingBar,
  PanelState,
  Row,
  SectionHeader,
} from '@/design-system';

/** Fidelity tier + caveats for one run, rendered as a single chip row. */
function FidelityChips({ meta }: { meta: RunMeta | null }) {
  if (!meta) return <span className="text-archive text-data-s">—</span>;
  return (
    <div className="flex flex-wrap gap-1.5">
      <Badge tone={meta.fidelity_tier === 'lockfile-exact' ? 'good' : 'warn'}>
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

/** Signed delta (B − A). Risk metrics carry good/bad sentiment via deltaTone;
 *  neutral metrics keep the direction arrow but stay colorless (no sentiment). */
function DeltaCell({ delta, worseUp }: { delta: number; worseUp: boolean }) {
  // For neutral metrics any change reads as informational, not good/bad.
  return <DeltaTag value={delta} moreIsWorse={worseUp} className={worseUp ? undefined : 'text-archive'} />;
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

  const aLabel = `A${aRun != null ? ` · run ${aRun}` : ''}`;
  const bLabel = `B${bRun != null ? ` · run ${bRun}` : ''}`;

  // Rows with both sides present — drive the value grid + the magnitude bars.
  const pairedRows = rows.filter((r) => r.a != null && r.b != null);

  const gridRows = rows.map((r) => {
    const worseUp = MORE_IS_WORSE.has(r.key);
    const hasBoth = r.a != null && r.b != null;
    const delta = hasBoth ? (r.b as number) - (r.a as number) : null;
    return {
      label: r.label,
      a: r.a != null ? r.a : '—',
      b: r.b != null ? r.b : '—',
      delta: delta != null ? <DeltaCell delta={delta} worseUp={worseUp} /> : undefined,
    };
  });

  // has_cycle: yes/no per run (not a numeric delta) → gained/resolved badge.
  const cycleChanged = aCycle != null && bCycle != null && aCycle !== bCycle;
  gridRows.push({
    label: 'has cycle',
    a: aCycle == null ? '—' : aCycle ? 'yes' : 'no',
    b: bCycle == null ? '—' : bCycle ? 'yes' : 'no',
    delta: cycleChanged ? (
      <Badge tone={bCycle && !aCycle ? 'bad' : 'good'}>{bCycle && !aCycle ? 'gained' : 'resolved'}</Badge>
    ) : undefined,
  });

  return (
    <div>
      <SectionHeader as="h3">Graph · A | B</SectionHeader>
      <CompareGrid rows={gridRows} aLabel={aLabel} bLabel={bLabel} />

      {pairedRows.length > 0 && (
        <>
          <SectionHeader as="h3">Magnitude · A | B</SectionHeader>
          <div className="space-y-2">
            {pairedRows.map((r) => {
              const worseUp = MORE_IS_WORSE.has(r.key);
              return (
                <div key={r.key}>
                  <span className="text-archive text-data-s">{r.label}</span>
                  <DivergingBar
                    a={r.a as number}
                    b={r.b as number}
                    leftTone={worseUp && (r.a as number) > (r.b as number) ? 'warn' : 'neutral'}
                    rightTone={worseUp && (r.b as number) > (r.a as number) ? 'warn' : 'info'}
                    ariaLabel={`${r.label}: A ${r.a}, B ${r.b}`}
                  />
                </div>
              );
            })}
          </div>
        </>
      )}

      {/* Fidelity for BOTH runs as two chip rows — a downgrade in B is obvious. */}
      <SectionHeader as="h3">Fidelity · A | B</SectionHeader>
      <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 items-start">
        <span className="metadata-label pt-1">A</span>
        <FidelityChips meta={meta} />
        <span className="metadata-label pt-1">B</span>
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
          <Badge tone="info">
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
              <Badge tone={meta.fidelity_tier === 'lockfile-exact' ? 'good' : 'warn'}>
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
          <SectionHeader as="h3">Graph</SectionHeader>
          {stats ? (
            <div>
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
                <Row key={k} label={k} value={v} valueTone={flag ? 'text-warn' : 'text-bone'} />
              ))}
            </div>
          ) : (
            <PanelState kind="prompt">Pick a run to see its graph stats.</PanelState>
          )}
        </>
      )}

      <SectionHeader as="h3">How to read it</SectionHeader>
      <ul className="list-disc pl-4 text-archive text-data-s space-y-1">
        <li>Click a node to trace what it <b className="text-bone">depends on</b> and what <b className="text-bone">depends on it</b>.</li>
        <li><b className="text-bone">2D</b> is a layered graph; <b className="text-bone">3D</b> is a spatial topology.</li>
        <li>In 3D: <b className="text-bone">Layered</b> stacks by depth, <b className="text-bone">Fiber</b> by version, <b className="text-bone">Terrain</b> by a signal, <b className="text-bone">TDA</b> lights up a feature.</li>
      </ul>
    </div>
  );
}

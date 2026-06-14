'use client';

import { useMemo } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { verCmp, verKey } from '@/lib/version';
import { shortLabel } from '@/lib/palette';
import FidelityBanner from '@/components/FidelityBanner';
import { Badge, MetricPill, Callout, StatBar, RowButton, PanelState, type Tone, type MeterTone } from '@/components/ui';
import type { DuplicateCluster } from '@/lib/types';

type Drift = 'major' | 'minor' | 'patch';

const DRIFT_WEIGHT: Record<Drift, number> = { major: 3, minor: 2, patch: 1 };

/** Classify the semver span between the lowest and highest version in a cluster. */
function classifyDrift(versions: { purl: string }[]): Drift {
  if (versions.length < 2) return 'patch';
  const sorted = [...versions].sort((a, b) => verCmp(a.purl, b.purl));
  const lo = verKey(sorted[0].purl).core;
  const hi = verKey(sorted[sorted.length - 1].purl).core;
  if ((lo[0] || 0) !== (hi[0] || 0)) return 'major';
  if ((lo[1] || 0) !== (hi[1] || 0)) return 'minor';
  return 'patch';
}

// Drift tone: major reads as bad, minor as warn, patch as muted.
const DRIFT_TONE: Record<Drift, Tone> = { major: 'bad', minor: 'warn', patch: 'muted' };
// Severity meter tone mirrors drift, but patch falls back to accent for visibility.
const DRIFT_BAR_TONE: Record<Drift, MeterTone> = { major: 'bad', minor: 'warn', patch: 'accent' };

interface Ranked {
  cluster: DuplicateCluster;
  ids: number[];
  drift: Drift;
  blast: number;
  score: number;
}

export default function DuplicatesTab() {
  const run = useGraphStore((s) => s.run);
  const clusters = useGraphStore((s) => s.duplicates);
  const nodes = useGraphStore((s) => s.nodes);
  const index = useGraphStore((s) => s.index);
  const highlightSet = useGraphStore((s) => s.highlightSet);

  const purlToId = useMemo(() => {
    const m = new Map<string, number>();
    for (const n of nodes) m.set(n.purl, n.id);
    return m;
  }, [nodes]);

  const ranked = useMemo<Ranked[]>(() => {
    const metrics = index?.metrics;
    const rows = clusters.map((c) => {
      const ids = c.versions
        .map((v) => purlToId.get(v.purl))
        .filter((x): x is number => x != null);
      const drift = classifyDrift(c.versions);
      let blast = 0;
      if (metrics) {
        for (const id of ids) blast = Math.max(blast, metrics.get(id)?.revReach ?? 0);
      }
      const score =
        c.version_count *
        DRIFT_WEIGHT[drift] *
        Math.log2(2 + c.occurrence_count) *
        (1 + blast);
      return { cluster: c, ids, drift, blast, score };
    });
    rows.sort((a, b) => b.score - a.score);
    return rows;
  }, [clusters, purlToId, index]);

  if (!clusters.length) {
    return (
      <div>
        <FidelityBanner />
        {run == null ? (
          <PanelState kind="prompt">Pick a run to surface duplicate package families.</PanelState>
        ) : (
          <PanelState kind="empty" icon={<span aria-hidden>🎉</span>}>
            No package family appears at more than one version.
          </PanelState>
        )}
      </div>
    );
  }

  const maxScore = Math.max(1e-9, ...ranked.map((r) => r.score));

  return (
    <div>
      <FidelityBanner />
      {index?.approximate && (
        <Callout tone="warn" className="mb-2.5">
          Blast radius is approximate on this run (graph exceeds the transitive cap) — severity uses degree proxies.
        </Callout>
      )}
      <p className="text-muted text-sm mb-2">
        {clusters.length} famil{clusters.length === 1 ? 'y' : 'ies'} present at multiple versions, ranked by severity.
      </p>
      {ranked.map(({ cluster: c, ids, drift, blast, score }) => (
        <div key={c.package_id} className="border border-line rounded-lg mb-2.5 bg-panel-2 overflow-hidden">
          <RowButton
            onSelect={() => highlightSet(ids, c.package_purl)}
            ariaLabel={`Highlight ${shortLabel(c.package_purl)} (${c.version_count} versions)`}
            className="rounded-none border-x-0 border-t-0 border-b-line bg-panel-2 px-2.5 py-2"
          >
            <span className="font-mono text-body truncate">{shortLabel(c.package_purl)}</span>
            <Badge tone={DRIFT_TONE[drift]} title={`${drift} version drift`} className="ml-auto uppercase tracking-wide">
              {drift}
            </Badge>
            <MetricPill label="versions" value={c.version_count} tone="warn" />
          </RowButton>

          {/* Severity bar, normalized to the most severe cluster. */}
          <div className="px-2.5">
            <StatBar
              value={score}
              max={maxScore}
              tone={DRIFT_BAR_TONE[drift]}
              ariaLabel={`Severity for ${shortLabel(c.package_purl)}`}
            />
          </div>

          <div className="flex flex-wrap items-center gap-1.5 px-2.5 pt-1.5">
            <MetricPill
              label="occurrences"
              value={c.occurrence_count}
              title="how many nodes in the graph carry one of these versions"
            />
            <Badge
              mono
              title="shallowest → deepest placement in the dependency tree"
            >
              d{c.min_depth}..{c.max_depth}
            </Badge>
            <MetricPill
              label="blast"
              value={blast}
              title="max transitive dependents (blast radius) across these versions"
            />
          </div>

          <div className="px-2.5 pb-2.5 pt-1.5">
            {c.versions.map((v) => (
              <div key={v.version_id} className="font-mono text-mini text-muted py-0.5">
                {v.purl.split('@')[1] ?? v.purl}
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

'use client';

import { useMemo } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { verCmp, verKey } from '@/lib/version';
import FidelityBanner from '@/components/FidelityBanner';
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

// Drift badge tone: major reads as bad, minor as warn, patch as muted.
const DRIFT_STYLE: Record<Drift, string> = {
  major: 'text-bad border-[#5a2424] bg-[#2a1414]',
  minor: 'text-warn border-[#4a3a1e] bg-[#20180c]',
  patch: 'text-muted border-line-2 bg-panel-3',
};

interface Ranked {
  cluster: DuplicateCluster;
  ids: number[];
  drift: Drift;
  blast: number;
  score: number;
}

export default function DuplicatesTab() {
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
        <p className="text-muted text-sm">No package family appears at more than one version. 🎉</p>
      </div>
    );
  }

  const maxScore = Math.max(1e-9, ...ranked.map((r) => r.score));

  return (
    <div>
      <FidelityBanner />
      {index?.approximate && (
        <div className="px-2.5 py-2 rounded-lg text-[12px] mb-2.5 bg-[#20180c] border border-[#4a3a1e] text-warn">
          Blast radius is approximate on this run (graph exceeds the transitive cap) — severity uses degree proxies.
        </div>
      )}
      <p className="text-muted text-sm mb-2">
        {clusters.length} famil{clusters.length === 1 ? 'y' : 'ies'} present at multiple versions, ranked by severity.
      </p>
      {ranked.map(({ cluster: c, ids, drift, blast, score }) => {
        const pct = Math.round((score / maxScore) * 100);
        return (
          <div key={c.package_id} className="border border-line rounded-lg mb-2.5 bg-panel-2 overflow-hidden">
            <div
              onClick={() => highlightSet(ids, c.package_purl)}
              className="flex items-center gap-2 px-2.5 py-2 cursor-pointer hover:bg-panel-3"
            >
              <span className="font-mono text-xs truncate">{c.package_purl.replace(/^pkg:[^/]+\//, '')}</span>
              <span
                title={`${drift} version drift`}
                className={`ml-auto shrink-0 text-[10px] uppercase tracking-wide rounded-xl px-2 py-px border ${DRIFT_STYLE[drift]}`}
              >
                {drift}
              </span>
              <span className="shrink-0 text-[11px] text-warn bg-[#20180c] border border-[#4a3a1e] rounded-xl px-2 py-px">
                {c.version_count} versions
              </span>
            </div>

            {/* Severity bar, normalized to the most severe cluster. */}
            <div className="px-2.5">
              <div className="h-1.5 bg-panel-3 rounded overflow-hidden">
                <div
                  className={`h-full rounded ${drift === 'major' ? 'bg-bad' : drift === 'minor' ? 'bg-warn' : 'bg-accent'}`}
                  style={{ width: `${pct}%` }}
                />
              </div>
            </div>

            <div className="flex items-center gap-1.5 px-2.5 pt-1.5 text-[10.5px] text-muted">
              <span
                title="how many nodes in the graph carry one of these versions"
                className="rounded-xl px-2 py-px border border-line-2 bg-panel-3 tabular-nums"
              >
                occurrences: {c.occurrence_count}
              </span>
              <span
                title="shallowest → deepest placement in the dependency tree"
                className="font-mono rounded-xl px-2 py-px border border-line-2 bg-panel-3"
              >
                d{c.min_depth}..{c.max_depth}
              </span>
              <span
                title="max transitive dependents (blast radius) across these versions"
                className="rounded-xl px-2 py-px border border-line-2 bg-panel-3 tabular-nums"
              >
                blast {blast}
              </span>
            </div>

            <div className="px-2.5 pb-2.5 pt-1">
              {c.versions.map((v) => (
                <div key={v.version_id} className="font-mono text-[11px] text-muted py-0.5">
                  {v.purl.split('@')[1] ?? v.purl}
                </div>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}

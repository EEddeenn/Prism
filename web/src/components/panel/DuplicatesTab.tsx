'use client';

import { useEffect, useMemo, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { api, ApiError } from '@/lib/api';
import { verCmp, verKey } from '@/lib/version';
import { shortLabel } from '@/lib/palette';
import FidelityBanner from '@/components/FidelityBanner';
import {
  Badge,
  MetricPill,
  Callout,
  StatBar,
  PanelState,
  DataCard,
  PurlLabel,
  MonoLabel,
  TruncationNote,
} from '@/design-system';
import { driftTone, pickTone, type Tone } from '@/lib/tone';
import type { DuplicateCluster, DuplicateRootCause } from '@/lib/types';

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

// At most this many introducers per version before collapsing into "… +N more".
const MAX_INTRODUCERS = 4;

/** Server convergence verdict → shared tone. The verdict set is
 *  `compatible | major-drift | unknown`; anything else reads neutral. */
const CONVERGENCE_TONE: Record<string, Tone> = {
  'major-drift': 'bad',
  compatible: 'neutral',
  unknown: 'neutral',
};
function convergenceTone(verdict: string): Tone {
  return pickTone(CONVERGENCE_TONE, verdict.toLowerCase(), 'neutral');
}

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
  const selectNode = useGraphStore((s) => s.selectNode);

  const purlToId = useMemo(() => {
    const m = new Map<string, number>();
    for (const n of nodes) m.set(n.purl, n.id);
    return m;
  }, [nodes]);

  // Server-computed root cause, fetched lazily per run. Additive: a card renders
  // fine without it, and we never block the existing ranking on this request.
  const [causes, setCauses] = useState<DuplicateRootCause[]>([]);
  const [causeError, setCauseError] = useState<string | null>(null);

  useEffect(() => {
    if (run == null) {
      setCauses([]);
      setCauseError(null);
      return;
    }
    let live = true;
    setCauseError(null);
    setCauses([]); // drop the prior run's causes so a slow fetch can't paint stale root cause
    api
      .dupCauses(run)
      .then((rows) => { if (live) setCauses(rows); })
      .catch((e) => {
        if (!live) return;
        setCauses([]);
        setCauseError(e instanceof ApiError ? e.message : 'failed to load root-cause analysis');
      });
    return () => { live = false; };
  }, [run]);

  // package_id → root cause, matching DuplicateCluster.package_id.
  const causeById = useMemo(() => {
    const m = new Map<number, DuplicateRootCause>();
    for (const rc of causes) m.set(rc.package_id, rc);
    return m;
  }, [causes]);

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
    <div className="space-y-2.5">
      <FidelityBanner />
      {index?.approximate && (
        <Callout tone="warn" showWord>
          Blast radius is approximate on this run (graph exceeds the transitive cap) — severity uses degree proxies.
        </Callout>
      )}
      {causeError && (
        <Callout tone="bad" role="alert" showWord>
          Root-cause analysis unavailable: {causeError}
        </Callout>
      )}
      <p className="text-archive text-data-s">
        <span className="font-mono tabular-nums">{clusters.length}</span> famil
        {clusters.length === 1 ? 'y' : 'ies'} present at multiple versions, ranked by severity.
      </p>
      {ranked.map(({ cluster: c, ids, drift, blast, score }) => {
        const rc = causeById.get(c.package_id);
        return (
          <DataCard
            key={c.package_id}
            eyebrow="Duplicate family"
            title={<PurlLabel purl={c.package_purl} />}
            right={
              <>
                <Badge tone={driftTone(drift)} title={`${drift} version drift`} mono>
                  {drift}
                </Badge>
                <MetricPill label="versions" value={c.version_count} tone="warn" />
              </>
            }
            footer={
              rc ? (
                <div>
                  <p className="metadata-label mb-1.5">Root cause</p>
                  <div className="flex flex-wrap items-center gap-1.5">
                    <Badge tone={convergenceTone(rc.convergence)} title="convergence verdict from the server">
                      {rc.convergence}
                    </Badge>
                    <Badge
                      tone={rc.distinct_constraints > 1 ? 'warn' : 'neutral'}
                      title="distinct version constraints competing across introducers"
                      mono
                    >
                      {rc.distinct_constraints} competing constraint{rc.distinct_constraints === 1 ? '' : 's'}
                    </Badge>
                    <MetricPill
                      label="blast"
                      value={rc.blast_radius}
                      title="transitive dependents reachable through this family"
                    />
                    <MetricPill
                      label="risk"
                      value={rc.risk_score.toFixed(1)}
                      tone={convergenceTone(rc.convergence)}
                      title="server risk score for this duplicate family"
                    />
                  </div>

                  <div className="mt-2 flex flex-col gap-1.5">
                    {rc.versions.map((vc) => {
                      const shown = vc.introducers.slice(0, MAX_INTRODUCERS);
                      const extra = vc.introducers.length - shown.length;
                      return (
                        <div key={vc.version_id}>
                          <div className="flex flex-wrap items-baseline gap-x-1.5 gap-y-0.5">
                            <MonoLabel
                              className="text-data-s text-bone"
                              // version = everything after the LAST '@' so a scoped
                              // "@scope/name@1.0.0" isn't split at the scope marker.
                              text={vc.purl.slice(vc.purl.lastIndexOf('@') + 1) || shortLabel(vc.purl)}
                              full={vc.purl}
                            />
                            <span className="text-meta text-archive font-mono tabular-nums">
                              {vc.occurrences} occurrence{vc.occurrences === 1 ? '' : 's'}
                            </span>
                          </div>
                          {shown.length === 0 ? (
                            <div className="text-meta text-archive pl-2.5">no recorded introducer</div>
                          ) : (
                            <ul className="pl-2.5">
                              {shown.map((intro, i) => {
                                const nodeId = purlToId.get(intro.introducer_purl);
                                const label = shortLabel(intro.introducer_purl);
                                const constraint = intro.constraint || 'unconstrained';
                                return (
                                  <li key={`${intro.introducer_purl}@${i}`} className="text-meta py-0.5">
                                    {nodeId != null ? (
                                      <button
                                        type="button"
                                        onClick={() => void selectNode(nodeId)}
                                        className="font-mono text-cyan-gray hover:underline"
                                        title={`Select ${label}`}
                                      >
                                        {label}
                                      </button>
                                    ) : (
                                      <span className="font-mono text-archive">{label}</span>
                                    )}
                                    <span className="text-archive"> needs </span>
                                    <span className={intro.constraint ? 'font-mono text-bone' : 'text-archive italic'}>
                                      {constraint}
                                    </span>
                                  </li>
                                );
                              })}
                              {extra > 0 && (
                                <li className="py-0.5">
                                  <TruncationNote shown={shown.length} total={vc.introducers.length} more={extra} />
                                </li>
                              )}
                            </ul>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </div>
              ) : undefined
            }
          >
            {/* Severity bar, normalized to the most severe cluster. */}
            <StatBar
              value={score}
              max={maxScore}
              tone={driftTone(drift)}
              ariaLabel={`Severity for ${shortLabel(c.package_purl)}`}
            />

            <div className="mt-2 flex flex-wrap items-center gap-1.5">
              <MetricPill
                label="occurrences"
                value={c.occurrence_count}
                title="how many nodes in the graph carry one of these versions"
              />
              <Badge mono title="shallowest → deepest placement in the dependency tree">
                d{c.min_depth}..{c.max_depth}
              </Badge>
              <MetricPill
                label="blast"
                value={blast}
                title="max transitive dependents (blast radius) across these versions"
              />
            </div>

            <div className="mt-2">
              {c.versions.map((v) => (
                <div key={v.version_id} className="font-mono tabular-nums text-meta text-archive py-0.5">
                  {v.purl.slice(v.purl.lastIndexOf('@') + 1) || v.purl}
                </div>
              ))}
            </div>
          </DataCard>
        );
      })}
    </div>
  );
}

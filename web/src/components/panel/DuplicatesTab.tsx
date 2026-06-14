'use client';

import { useEffect, useMemo, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { api, ApiError } from '@/lib/api';
import { verCmp, verKey } from '@/lib/version';
import { shortLabel } from '@/lib/palette';
import FidelityBanner from '@/components/FidelityBanner';
import { Badge, MetricPill, Callout, StatBar, RowButton, PanelState, type Tone, type MeterTone } from '@/components/ui';
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

// Drift tone: major reads as bad, minor as warn, patch as muted.
const DRIFT_TONE: Record<Drift, Tone> = { major: 'bad', minor: 'warn', patch: 'muted' };
// Severity meter tone mirrors drift, but patch falls back to accent for visibility.
const DRIFT_BAR_TONE: Record<Drift, MeterTone> = { major: 'bad', minor: 'warn', patch: 'accent' };

// At most this many introducers per version before collapsing into "… +N more".
const MAX_INTRODUCERS = 4;

/** Infer a Badge tone from the server convergence verdict string. */
function convergenceTone(verdict: string): Tone {
  const v = verdict.toLowerCase();
  // Order matters: "diverged" must be tested before the converged check, and it
  // does not contain the substring "converged", so a plain includes() is safe.
  if (v.includes('diverged') || v.includes('major')) return 'bad';
  if (v.includes('partial') || v.includes('minor')) return 'warn';
  if (v.includes('converged')) return 'ok';
  return 'muted';
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
    <div>
      <FidelityBanner />
      {index?.approximate && (
        <Callout tone="warn" className="mb-2.5">
          Blast radius is approximate on this run (graph exceeds the transitive cap) — severity uses degree proxies.
        </Callout>
      )}
      {causeError && (
        <Callout tone="bad" role="alert" className="mb-2.5">
          Root-cause analysis unavailable: {causeError}
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
                {v.purl.slice(v.purl.lastIndexOf('@') + 1) || v.purl}
              </div>
            ))}
          </div>

          {/* Server-computed root cause: which constraints pin which versions.
              Additive — only rendered when dup-causes has matched this family. */}
          {(() => {
            const rc = causeById.get(c.package_id);
            if (!rc) return null;
            return (
              <div className="border-t border-line px-2.5 py-2 bg-panel-3">
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-micro uppercase tracking-wide text-muted mr-0.5">Root cause</span>
                  <Badge tone={convergenceTone(rc.convergence)} title="convergence verdict from the server">
                    {rc.convergence}
                  </Badge>
                  <Badge
                    tone={rc.distinct_constraints > 1 ? 'warn' : 'muted'}
                    title="distinct version constraints competing across introducers"
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

                <div className="mt-1.5 flex flex-col gap-1.5">
                  {rc.versions.map((vc) => {
                    const shown = vc.introducers.slice(0, MAX_INTRODUCERS);
                    const extra = vc.introducers.length - shown.length;
                    return (
                      <div key={vc.version_id}>
                        <div className="flex flex-wrap items-baseline gap-x-1.5 gap-y-0.5">
                          <span className="font-mono text-mini text-ink">
                            {/* version = everything after the LAST '@' so a scoped
                                "@scope/name@1.0.0" isn't split at the scope marker. */}
                            {vc.purl.slice(vc.purl.lastIndexOf('@') + 1) || shortLabel(vc.purl)}
                          </span>
                          <span className="text-micro text-muted">
                            {vc.occurrences} occurrence{vc.occurrences === 1 ? '' : 's'}
                          </span>
                        </div>
                        {shown.length === 0 ? (
                          <div className="text-micro text-muted pl-2.5">no recorded introducer</div>
                        ) : (
                          <ul className="pl-2.5">
                            {shown.map((intro, i) => {
                              const nodeId = purlToId.get(intro.introducer_purl);
                              const label = shortLabel(intro.introducer_purl);
                              const constraint = intro.constraint || 'unconstrained';
                              return (
                                <li key={`${intro.introducer_purl}@${i}`} className="text-micro py-0.5">
                                  {nodeId != null ? (
                                    <button
                                      type="button"
                                      onClick={() => void selectNode(nodeId)}
                                      className="font-mono text-accent hover:underline"
                                      title={`Select ${label}`}
                                    >
                                      {label}
                                    </button>
                                  ) : (
                                    <span className="font-mono text-muted">{label}</span>
                                  )}
                                  <span className="text-muted"> needs </span>
                                  <span className={intro.constraint ? 'font-mono text-ink' : 'text-muted italic'}>
                                    {constraint}
                                  </span>
                                </li>
                              );
                            })}
                            {extra > 0 && (
                              <li className="text-micro text-muted py-0.5">… +{extra} more</li>
                            )}
                          </ul>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            );
          })()}
        </div>
      ))}
    </div>
  );
}

'use client';

import { useMemo } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { kindName, isBackEdge } from '@/lib/edges';
import type { SccComp } from '@/lib/types';
import type { GraphIndex } from '@/lib/graph';
import FidelityBanner from '@/components/FidelityBanner';
import {
  Badge,
  Callout,
  DataCard,
  MetricPill,
  PanelState,
  PurlLabel,
  SectionHeader,
  SegmentBar,
  ListRow,
} from '@/design-system';

type InternalEdge = {
  source: number;
  target: number;
  kind: number;
  flags: number;
  runtime: boolean;
  back: boolean;
};

type Ranked = {
  scc: SccComp;
  /** Stable id for keying — the smallest member id (order-independent). */
  key: number;
  severity: number;
  maxRevReach: number;
  runtimeFraction: number;
  internal: InternalEdge[];
  runtimeCount: number;
  breakEdges: InternalEdge[];
};

/** Collect the edges that live entirely inside an SCC by probing index.edgeMeta
 *  for every ordered pair of members (a>b). The graph edges aren't passed to the
 *  tab directly, so edgeMeta is the source of truth here. */
function internalEdges(scc: SccComp, index: GraphIndex): InternalEdge[] {
  const out: InternalEdge[] = [];
  const ids = scc.members.map((m) => m.id);
  for (const a of ids) {
    for (const b of ids) {
      if (a === b) continue;
      const meta = index.edgeMeta.get(`${a}>${b}`);
      if (!meta) continue;
      out.push({
        source: a,
        target: b,
        kind: meta.kind,
        flags: meta.flags,
        runtime: kindName(meta.kind) === 'runtime',
        back: isBackEdge({ source: a, target: b, kind: meta.kind, flags: meta.flags }),
      });
    }
  }
  return out;
}

function rank(scc: SccComp, index: GraphIndex): Ranked {
  const internal = internalEdges(scc, index);
  const runtimeCount = internal.filter((e) => e.runtime).length;
  const runtimeFraction = runtimeCount / (internal.length || 1);
  const maxRevReach = scc.members.reduce(
    (mx, m) => Math.max(mx, index.metrics.get(m.id)?.revReach ?? 0),
    0,
  );
  const severity = scc.members.length * maxRevReach * runtimeFraction;
  const breakEdges = internal.filter((e) => e.back);
  const key = scc.members.reduce((mn, m) => Math.min(mn, m.id), Infinity);
  return { scc, key, severity, maxRevReach, runtimeFraction, internal, runtimeCount, breakEdges };
}

export default function CyclesTab() {
  const sccs = useGraphStore((s) => s.sccs);
  const index = useGraphStore((s) => s.index);
  const selectNode = useGraphStore((s) => s.selectNode);

  const ranked = useMemo<Ranked[]>(() => {
    if (!index) return [];
    return sccs
      .filter((c) => c.is_cycle)
      .map((c) => rank(c, index))
      .sort((a, b) => b.severity - a.severity);
  }, [sccs, index]);

  const runtimeCycles = ranked.filter((r) => r.runtimeCount > 0).length;
  const devOnlyCycles = ranked.length - runtimeCycles;

  if (!sccs.length) {
    return (
      <div>
        <FidelityBanner />
        <PanelState kind="prompt">Pick a run to inspect dependency cycles.</PanelState>
      </div>
    );
  }

  if (!ranked.length) {
    return (
      <div>
        <FidelityBanner />
        <Callout tone="good" showWord>
          Acyclic — {sccs.length} strongly-connected components, each a single node.
        </Callout>
      </div>
    );
  }

  return (
    <div>
      <FidelityBanner />

      {index?.approximate && (
        <Callout tone="warn" showWord className="mb-2">
          Blast radius is approximate — the graph exceeds the transitive-reach cap, so revReach
          fell back to a degree proxy. Severity ranking is indicative.
        </Callout>
      )}

      <Callout tone="bad" showWord className="mb-2.5">
        {ranked.length} dependency cycle{ranked.length === 1 ? '' : 's'} found —{' '}
        <span className="font-mono tabular-nums text-bad">{runtimeCycles}</span> reach production
        (runtime edges),{' '}
        <span className="font-mono tabular-nums text-archive">{devOnlyCycles}</span> dev-only.
      </Callout>

      <p className="mb-2 text-data-s text-archive">
        Ranked by severity = members × blast radius × runtime fraction. Break a cycle at its
        loop-closing (back) edge.
      </p>

      {ranked.map((r, i) => {
        const c = r.scc;
        const otherCount = r.internal.length - r.runtimeCount;
        return (
          <DataCard
            key={r.key}
            className={`mb-2.5 ${r.runtimeCount > 0 ? 'border-bad-bd' : ''}`}
            eyebrow="Cycle"
            title={
              <span className="flex items-baseline gap-1.5">
                <span className="font-mono tabular-nums text-bad">#{i + 1}</span>
                <span className="text-data text-bone">
                  <span className="font-mono tabular-nums">{c.members.length}</span> members
                </span>
              </span>
            }
            right={
              <MetricPill
                label="blast"
                value={r.maxRevReach}
                tone="warn"
                title="aggregate blast radius — max transitive dependents (revReach) across the cycle"
              />
            }
          >
            {/* kind-mix mini-bar: runtime vs other internal edges */}
            <div className="flex items-center gap-2">
              <div className="w-[78px] shrink-0 text-meta text-archive">edge mix</div>
              {/* Stacked proportion bar: each segment's WIDTH is its share of the
                  cycle's internal edges (runtime=info vs other=neutral), with an
                  inline legend so the breakdown reads without color alone. */}
              <div className="min-w-0 flex-1">
                <SegmentBar
                  showLegend
                  ariaLabel={`${r.runtimeCount} runtime of ${r.internal.length} internal edges`}
                  segments={[
                    { value: r.runtimeCount, tone: 'info', label: 'rt' },
                    { value: otherCount, tone: 'neutral', label: 'other' },
                  ]}
                />
              </div>
            </div>

            {/* most actionable: break-here rows (loop-closing back edges) */}
            <div className="mt-2">
              {r.breakEdges.length > 0 ? (
                <>
                  <SectionHeader as="h4">
                    break here — loop-closing edge{r.breakEdges.length === 1 ? '' : 's'}
                  </SectionHeader>
                  {r.breakEdges.map((e) => {
                    const src = index?.byId.get(e.source);
                    const tgt = index?.byId.get(e.target);
                    return (
                      <ListRow
                        key={`${e.source}>${e.target}`}
                        onSelect={() => selectNode(e.source)}
                        title={`break ${src?.purl ?? e.source} → ${tgt?.purl ?? e.target} (${kindName(e.kind)})`}
                        ariaLabel={`break edge from ${src?.purl ?? e.source} to ${tgt?.purl ?? e.target}, ${kindName(e.kind)}`}
                      >
                        <span className="flex items-center gap-1.5 font-mono">
                          <span aria-hidden className="shrink-0 text-bad">
                            ✂
                          </span>
                          {src ? (
                            <PurlLabel purl={src.purl} className="text-bone" />
                          ) : (
                            <span className="text-bone">{e.source}</span>
                          )}
                          <span aria-hidden className="shrink-0 text-archive">
                            →
                          </span>
                          {tgt ? (
                            <PurlLabel purl={tgt.purl} className="text-archive" />
                          ) : (
                            <span className="text-archive">{e.target}</span>
                          )}
                          {e.runtime && (
                            <Badge tone="info" className="ml-auto shrink-0">
                              runtime
                            </Badge>
                          )}
                        </span>
                      </ListRow>
                    );
                  })}
                </>
              ) : (
                <div className="text-data-s text-archive">
                  No back-edge flagged in this SCC — break the weakest internal link.
                </div>
              )}
            </div>

            {/* members (kept: click → selectNode) */}
            <div className="mt-2 border-t border-graphite pt-2">
              <SectionHeader as="h4">members</SectionHeader>
              {c.members.map((m) => (
                <ListRow
                  key={m.id}
                  onSelect={() => selectNode(m.id)}
                  title={m.purl}
                  ariaLabel={`select ${m.purl}`}
                >
                  <PurlLabel purl={m.purl} className="text-archive" />
                </ListRow>
              ))}
            </div>
          </DataCard>
        );
      })}
    </div>
  );
}

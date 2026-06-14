'use client';

import { useMemo } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { shortLabel } from '@/lib/palette';
import { kindName, isBackEdge } from '@/lib/edges';
import type { SccComp } from '@/lib/types';
import type { GraphIndex } from '@/lib/graph';
import FidelityBanner from '@/components/FidelityBanner';
import {
  Badge,
  Callout,
  MetricPill,
  PanelState,
  RowButton,
  SectionHeader,
} from '@/components/ui';

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
        <Callout tone="ok">
          Acyclic — {sccs.length} strongly-connected components, each a single node.
        </Callout>
      </div>
    );
  }

  return (
    <div>
      <FidelityBanner />

      {index?.approximate && (
        <Callout tone="warn" className="mb-2">
          Blast radius is approximate — the graph exceeds the transitive-reach cap, so revReach
          fell back to a degree proxy. Severity ranking is indicative.
        </Callout>
      )}

      <Callout tone="bad" className="mb-2.5">
        {ranked.length} dependency cycle{ranked.length === 1 ? '' : 's'} found —{' '}
        <span className="font-semibold text-bad">{runtimeCycles}</span> reach production (runtime
        edges), <span className="font-semibold text-muted">{devOnlyCycles}</span> dev-only.
      </Callout>

      <p className="text-muted text-mini mb-2">
        Ranked by severity = members × blast radius × runtime fraction. Break a cycle at its
        loop-closing (back) edge.
      </p>

      {ranked.map((r, i) => {
        const c = r.scc;
        const otherCount = r.internal.length - r.runtimeCount;
        const runtimePct = Math.round(r.runtimeFraction * 100);
        return (
          <div
            key={r.key}
            className={`border rounded-card mb-2.5 bg-panel-2 overflow-hidden ${
              r.runtimeCount > 0 ? 'border-bad-bd' : 'border-line'
            }`}
          >
            {/* header: counts + blast radius */}
            <div className="flex items-center gap-2 px-2.5 py-2 border-b border-line">
              <span className="font-mono font-bold text-mini text-bad min-w-[20px]">#{i + 1}</span>
              <span className="text-mini">
                <span className="font-semibold tabular-nums">{c.members.length}</span> members
              </span>
              <span className="ml-auto shrink-0">
                <MetricPill
                  label="blast"
                  value={r.maxRevReach}
                  tone="warn"
                  title="aggregate blast radius — max transitive dependents (revReach) across the cycle"
                />
              </span>
            </div>

            {/* kind-mix mini-bar: runtime vs other internal edges */}
            <div className="px-2.5 pt-2">
              <div className="flex items-center gap-2">
                <div className="w-[78px] shrink-0 text-muted text-mini">edge mix</div>
                {/* Stacked proportion bar: each segment's WIDTH is its share of
                    the cycle's internal edges (runtime accent vs other line-2). */}
                <div
                  className="flex-1 flex h-3 rounded-badge overflow-hidden bg-panel-3"
                  role="progressbar"
                  aria-valuenow={r.runtimeCount}
                  aria-valuemin={0}
                  aria-valuemax={r.internal.length}
                  aria-label={`${r.runtimeCount} runtime of ${r.internal.length} internal edges`}
                >
                  {r.runtimeCount > 0 && (
                    <div
                      className="h-full bg-accent"
                      style={{ width: `${runtimePct}%` }}
                      title={`${r.runtimeCount} runtime edge${r.runtimeCount === 1 ? '' : 's'}`}
                    />
                  )}
                  {otherCount > 0 && (
                    <div
                      className="h-full bg-line-2"
                      style={{ width: `${100 - runtimePct}%` }}
                      title={`${otherCount} non-runtime edge${otherCount === 1 ? '' : 's'}`}
                    />
                  )}
                </div>
                <div className="w-[110px] shrink-0 flex items-center justify-end gap-2 text-micro text-muted tabular-nums">
                  <span className="flex items-center gap-1">
                    <span aria-hidden className="inline-block w-1.5 h-1.5 rounded-pill bg-accent" />
                    {r.runtimeCount} rt
                  </span>
                  <span className="flex items-center gap-1">
                    <span aria-hidden className="inline-block w-1.5 h-1.5 rounded-pill bg-line-2" />
                    {otherCount} other
                  </span>
                </div>
              </div>
            </div>

            {/* most actionable: break-here rows (loop-closing back edges) */}
            <div className="px-2.5 pt-2 pb-1">
              {r.breakEdges.length > 0 ? (
                <>
                  <SectionHeader as="h4" className="text-micro">
                    break here — loop-closing edge{r.breakEdges.length === 1 ? '' : 's'}
                  </SectionHeader>
                  {r.breakEdges.map((e) => {
                    const src = index?.byId.get(e.source);
                    const tgt = index?.byId.get(e.target);
                    return (
                      <RowButton
                        key={`${e.source}>${e.target}`}
                        onSelect={() => selectNode(e.source)}
                        title={`break ${src?.purl ?? e.source} → ${tgt?.purl ?? e.target} (${kindName(e.kind)})`}
                        ariaLabel={`break edge from ${src?.purl ?? e.source} to ${tgt?.purl ?? e.target}, ${kindName(e.kind)}`}
                        className="font-mono text-mini"
                      >
                        <span className="text-bad shrink-0">✂</span>
                        <span className="text-ink truncate">
                          {src ? shortLabel(src.purl) : e.source}
                        </span>
                        <span className="text-muted shrink-0">→</span>
                        <span className="text-muted truncate">
                          {tgt ? shortLabel(tgt.purl) : e.target}
                        </span>
                        {e.runtime && (
                          <Badge tone="accent" className="ml-auto shrink-0">
                            runtime
                          </Badge>
                        )}
                      </RowButton>
                    );
                  })}
                </>
              ) : (
                <div className="text-mini text-muted">
                  No back-edge flagged in this SCC — break the weakest internal link.
                </div>
              )}
            </div>

            {/* members (kept: click → selectNode) */}
            <div className="px-2.5 pb-2 pt-1.5 border-t border-line mt-1">
              <SectionHeader as="h4" className="text-micro">
                members
              </SectionHeader>
              {c.members.map((m) => (
                <RowButton
                  key={m.id}
                  onSelect={() => selectNode(m.id)}
                  title={m.purl}
                  ariaLabel={`select ${m.purl}`}
                  className="font-mono text-mini text-muted"
                >
                  <span className="truncate">{shortLabel(m.purl)}</span>
                </RowButton>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}

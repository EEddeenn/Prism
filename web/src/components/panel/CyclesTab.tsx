'use client';

import { useMemo } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { shortLabel } from '@/lib/palette';
import { kindName, isBackEdge } from '@/lib/edges';
import type { SccComp } from '@/lib/types';
import type { GraphIndex } from '@/lib/graph';
import FidelityBanner from '@/components/FidelityBanner';

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
  return { scc, severity, maxRevReach, runtimeFraction, internal, runtimeCount, breakEdges };
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
        <p className="text-muted text-sm">Pick a run.</p>
      </div>
    );
  }

  if (!ranked.length) {
    return (
      <div>
        <FidelityBanner />
        <div className="px-2.5 py-2 rounded-lg text-[12.5px] bg-[#0f2419] border border-[#1d4a32] text-ok">
          Acyclic ✓ — {sccs.length} strongly-connected components, each a single node.
        </div>
      </div>
    );
  }

  return (
    <div>
      <FidelityBanner />

      {index?.approximate && (
        <div className="px-2.5 py-2 rounded-lg text-[11.5px] mb-2 bg-[#20180c] border border-[#4a3a1e] text-warn">
          Blast radius is approximate — the graph exceeds the transitive-reach cap, so
          revReach fell back to a degree proxy. Severity ranking is indicative.
        </div>
      )}

      <div className="px-2.5 py-2 rounded-lg text-[12.5px] mb-2.5 bg-[#2a1414] border border-[#5a2424] text-bad">
        {ranked.length} dependency cycle{ranked.length === 1 ? '' : 's'} found —{' '}
        <span className="font-semibold text-bad">{runtimeCycles}</span> reach production
        (runtime edges), <span className="font-semibold text-muted">{devOnlyCycles}</span> dev-only.
      </div>

      <p className="text-muted text-[11px] mb-2">
        Ranked by severity = members × blast radius × runtime fraction. Break a cycle at its
        loop-closing (back) edge.
      </p>

      {ranked.map((r, i) => {
        const c = r.scc;
        const runtimePct = Math.round(r.runtimeFraction * 100);
        const otherCount = r.internal.length - r.runtimeCount;
        return (
          <div
            key={i}
            className={`border rounded-lg mb-2.5 bg-panel-2 overflow-hidden ${
              r.runtimeCount > 0 ? 'border-[#5a2424]' : 'border-line'
            }`}
          >
            {/* header: counts + blast radius */}
            <div className="flex items-center gap-2 px-2.5 py-2 border-b border-line">
              <span className="font-mono font-bold text-[11px] text-bad min-w-[20px]">#{i + 1}</span>
              <span className="text-[11.5px]">
                <span className="font-semibold tabular-nums">{c.members.length}</span> members
              </span>
              <span
                className="ml-auto shrink-0 text-[11px] text-warn bg-[#20180c] border border-[#4a3a1e] rounded-xl px-2 py-px"
                title="aggregate blast radius — max transitive dependents (revReach) across the cycle"
              >
                blast {r.maxRevReach}
              </span>
            </div>

            {/* kind-mix mini-bar: runtime vs other internal edges */}
            <div className="px-2.5 pt-2">
              <div className="flex items-center gap-2">
                <div className="w-[78px] shrink-0 text-muted text-[11px]">edge mix</div>
                <div className="flex-1 h-3 bg-panel rounded overflow-hidden flex">
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
                <div className="w-[88px] shrink-0 text-right text-[10.5px] text-muted tabular-nums">
                  {r.runtimeCount} rt · {otherCount} other
                </div>
              </div>
            </div>

            {/* most actionable: break-here rows (loop-closing back edges) */}
            <div className="px-2.5 pt-2 pb-1">
              {r.breakEdges.length > 0 ? (
                <>
                  <div className="text-[10px] uppercase tracking-wide text-muted mb-1">
                    break here — loop-closing edge{r.breakEdges.length === 1 ? '' : 's'}
                  </div>
                  {r.breakEdges.map((e) => {
                    const src = index?.byId.get(e.source);
                    const tgt = index?.byId.get(e.target);
                    return (
                      <div
                        key={`${e.source}>${e.target}`}
                        onClick={() => selectNode(e.source)}
                        title={`break ${src?.purl ?? e.source} → ${tgt?.purl ?? e.target} (${kindName(e.kind)})`}
                        className="flex items-center gap-1.5 font-mono text-[11px] py-0.5 px-1.5 -mx-1.5 rounded cursor-pointer hover:bg-panel-3"
                      >
                        <span className="text-bad shrink-0">✂</span>
                        <span className="text-ink truncate">{src ? shortLabel(src.purl) : e.source}</span>
                        <span className="text-muted shrink-0">→</span>
                        <span className="text-muted truncate">{tgt ? shortLabel(tgt.purl) : e.target}</span>
                        {e.runtime && (
                          <span className="ml-auto shrink-0 text-[9.5px] text-accent border border-line-2 rounded-xl px-1.5">
                            runtime
                          </span>
                        )}
                      </div>
                    );
                  })}
                </>
              ) : (
                <div className="text-[10.5px] text-muted">
                  No back-edge flagged in this SCC — break the weakest internal link.
                </div>
              )}
            </div>

            {/* members (kept: click → selectNode) */}
            <div className="px-2.5 pb-2 pt-1.5 border-t border-line mt-1">
              <div className="text-[10px] uppercase tracking-wide text-muted mb-1">members</div>
              {c.members.map((m) => (
                <div
                  key={m.id}
                  title={m.purl}
                  onClick={() => selectNode(m.id)}
                  className="font-mono text-[11px] text-muted py-px cursor-pointer hover:text-accent"
                >
                  {shortLabel(m.purl)}
                </div>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}

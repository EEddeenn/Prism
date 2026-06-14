'use client';

import { useEffect, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { api, ApiError } from '@/lib/api';
import { shortLabel } from '@/lib/palette';
import type {
  RiskProfile, Chokepoint, ClassifiedCycle, Fragmentation, ScopeRow,
} from '@/lib/types';

/** Color a 0–100 risk score (low → high). */
function scoreColor(score: number): string {
  if (score >= 75) return '#e0524a';
  if (score >= 50) return '#e0954a';
  if (score >= 25) return '#e0c84a';
  return '#4a9d6e';
}

const CYCLE_BADGE: Record<string, string> = {
  'runtime-coupling': 'text-bad',
  'runtime-cluster': 'text-warn',
  'tooling-cycle': 'text-muted',
};

/**
 * The Structure tab: a server-computed structural-analysis dashboard — risk
 * profile, scope footprints, chokepoints, classified cycles, and fragmentation —
 * surfacing the prism-api structural views the 2D/3D leaderboards cannot.
 */
export default function StructureTab() {
  const run = useGraphStore((s) => s.run);
  const selectNode = useGraphStore((s) => s.selectNode);

  const [risk, setRisk] = useState<RiskProfile | null>(null);
  const [scope, setScope] = useState<ScopeRow[]>([]);
  const [chokepoints, setChokepoints] = useState<Chokepoint[]>([]);
  const [cycles, setCycles] = useState<ClassifiedCycle[]>([]);
  const [frag, setFrag] = useState<Fragmentation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (run == null) return;
    let live = true;
    setLoading(true);
    setError(null);
    Promise.all([
      api.risk(run),
      api.scope(run),
      api.chokepoints(run, 10),
      api.classifiedCycles(run),
      api.fragmentation(run),
    ])
      .then(([r, sc, cp, cy, fr]) => {
        if (!live) return;
        setRisk(r); setScope(sc); setChokepoints(cp); setCycles(cy); setFrag(fr);
      })
      .catch((e) => {
        if (!live) return;
        setError(e instanceof ApiError ? e.message : 'failed to load structural analysis');
      })
      .finally(() => live && setLoading(false));
    return () => { live = false; };
  }, [run]);

  if (run == null) return <p className="text-muted text-sm">Pick a run.</p>;
  if (error) {
    return (
      <div className="px-2.5 py-2 rounded-lg text-[12px] bg-[#2a1414] border border-[#5a2424] text-bad">
        {error}
      </div>
    );
  }
  if (loading && !risk) return <p className="text-muted text-sm">Computing structural analysis…</p>;

  return (
    <div className="flex flex-col gap-3.5">
      {/* Risk profile */}
      {risk && (
        <section>
          <div className="flex items-baseline gap-2 mb-2">
            <span className="text-[13px] font-semibold">Structural risk</span>
            <span className="text-[20px] font-bold tabular-nums" style={{ color: scoreColor(risk.overall) }}>
              {risk.overall}
            </span>
            <span className="text-muted text-[11px]">/ 100 · {risk.band}</span>
          </div>
          <div className="flex flex-col gap-1.5">
            {risk.dimensions.map((d) => (
              <div key={d.name} className="flex items-center gap-2" title={d.detail}>
                <span className="w-[120px] shrink-0 text-[11px] text-muted">{d.name}</span>
                <div className="flex-1 h-2.5 bg-panel rounded overflow-hidden">
                  <div className="h-full rounded" style={{ width: `${d.score}%`, background: scoreColor(d.score) }} />
                </div>
                <span className="w-[26px] text-right text-[11px] tabular-nums">{d.score}</span>
              </div>
            ))}
          </div>
        </section>
      )}

      {/* Scope footprints */}
      {scope.length > 0 && (
        <section>
          <h3 className="text-[12px] font-semibold mb-1.5">Scope footprints</h3>
          <table className="w-full text-[11px] tabular-nums">
            <thead className="text-muted">
              <tr><th className="text-left font-medium">scope</th><th className="text-right font-medium">footprint</th><th className="text-right font-medium">exclusive</th><th className="text-right font-medium">depth</th></tr>
            </thead>
            <tbody>
              {scope.map((s) => (
                <tr key={s.kind} className="border-t border-line">
                  <td className="py-0.5">{s.kind}</td>
                  <td className="text-right">{s.footprint}</td>
                  <td className="text-right">{s.exclusive}</td>
                  <td className="text-right">{s.max_depth}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="text-muted text-[10px] mt-1">footprint = reachable under production + scope; exclusive = added beyond production.</p>
        </section>
      )}

      {/* Chokepoints */}
      {chokepoints.length > 0 && (
        <section>
          <h3 className="text-[12px] font-semibold mb-1.5">Chokepoints</h3>
          <p className="text-muted text-[10px] mb-1.5">packages that solely gate access to large parts of the graph.</p>
          <ul className="flex flex-col gap-1">
            {chokepoints.map((c) => (
              <li key={c.node.id}>
                <button
                  onClick={() => selectNode(c.node.id)}
                  className="w-full flex items-center gap-2 px-2 py-1 rounded bg-panel-2 hover:bg-panel-3 text-left"
                >
                  <span className="font-mono text-bad text-[11px] tabular-nums min-w-[34px]">{c.dominated}</span>
                  <span className="text-[11px] truncate">{shortLabel(c.node.purl)}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Classified cycles */}
      <section>
        <h3 className="text-[12px] font-semibold mb-1.5">Cycles ({cycles.length})</h3>
        {cycles.length === 0 ? (
          <p className="text-muted text-[11px]">no dependency cycles.</p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {cycles.slice(0, 8).map((c, i) => (
              <li key={i} className="px-2 py-1.5 rounded bg-panel-2">
                <div className="flex items-center gap-2 text-[11px]">
                  <span className={`font-semibold ${CYCLE_BADGE[c.class] ?? 'text-muted'}`}>{c.class}</span>
                  <span className="text-muted">{c.members.length} members</span>
                  <span className="ml-auto text-muted">blast {c.external_dependents}</span>
                </div>
                <div className="text-[10.5px] text-muted truncate mt-0.5">
                  {c.members.map((m) => shortLabel(m.purl)).join(' ⇄ ')}
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* Fragmentation */}
      {frag && (
        <section>
          <h3 className="text-[12px] font-semibold mb-1.5">Fragmentation</h3>
          <p className="text-[11px]">{frag.note}</p>
          {frag.subsystem_islands.length > 0 && (
            <ul className="flex flex-col gap-1 mt-1.5">
              {frag.subsystem_islands.slice(0, 5).map((isl, i) => (
                <li key={i} className="text-[10.5px] text-muted px-2 py-1 rounded bg-panel-2">
                  <span className="font-semibold text-ink tabular-nums">{isl.size}</span> pkgs
                  {isl.detached_from_roots ? ' · detached' : ''} — {isl.members.slice(0, 3).map(shortLabel).join(', ')}
                </li>
              ))}
            </ul>
          )}
        </section>
      )}
    </div>
  );
}

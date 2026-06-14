'use client';

import { useEffect, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { api, ApiError } from '@/lib/api';
import { shortLabel } from '@/lib/palette';
import FidelityBanner from '@/components/FidelityBanner';
import {
  SectionHeader,
  RowButton,
  Badge,
  MetricPill,
  Callout,
  StatBar,
  PanelState,
  type MeterTone,
  type Tone,
} from '@/components/ui';
import type {
  RiskProfile, Chokepoint, ClassifiedCycle, Fragmentation, ScopeRow,
} from '@/lib/types';

/** Tone a 0–100 risk score (low → high) so risk red matches the shared `bad` red. */
function scoreTone(score: number): MeterTone | 'gen' {
  if (score >= 75) return 'bad';
  if (score >= 50) return 'warn';
  if (score >= 25) return 'gen';
  return 'ok';
}

/** A score tone as a Tailwind text-color token (`gen` has no semantic alias). */
function scoreTextClass(score: number): string {
  const t = scoreTone(score);
  return t === 'gen' ? 'text-gen' : `text-${t}`;
}

/** A score tone usable by StatBar, collapsing the bespoke `gen` ramp stop to warn. */
function scoreMeterTone(score: number): MeterTone {
  const t = scoreTone(score);
  return t === 'gen' ? 'warn' : t;
}

const CYCLE_TONE: Record<string, Tone> = {
  'runtime-coupling': 'bad',
  'runtime-cluster': 'warn',
  'tooling-cycle': 'muted',
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

  if (run == null) return <PanelState kind="prompt">Pick a run.</PanelState>;
  if (error) return <Callout tone="bad" role="alert">{error}</Callout>;
  if (loading && !risk) return <PanelState kind="loading">Computing structural analysis…</PanelState>;

  return (
    <div className="flex flex-col gap-3.5">
      <FidelityBanner />

      {/* Risk profile */}
      {risk && (
        <section>
          <div className="flex items-baseline gap-2 mb-2">
            <span className="text-head font-semibold">Structural risk</span>
            <span className={`text-display font-bold tabular-nums ${scoreTextClass(risk.overall)}`}>
              {risk.overall}
            </span>
            <span className="text-muted text-mini">/ 100 · {risk.band}</span>
          </div>
          <div className="flex flex-col gap-1.5">
            {risk.dimensions.map((d) => (
              <div key={d.name} className="flex items-center gap-2" title={d.detail}>
                <span className="w-[120px] shrink-0 text-mini text-muted">{d.name}</span>
                <StatBar
                  value={d.score}
                  max={100}
                  tone={scoreMeterTone(d.score)}
                  height="lg"
                  ariaLabel={`${d.name} risk ${d.score} of 100`}
                  className="flex-1"
                />
                <span className="w-[26px] text-right text-mini tabular-nums">{d.score}</span>
              </div>
            ))}
          </div>
        </section>
      )}

      {/* Scope footprints */}
      {scope.length > 0 && (
        <section>
          <SectionHeader>Scope footprints</SectionHeader>
          <table className="w-full text-mini tabular-nums">
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
          <p className="text-muted text-micro mt-1">footprint = reachable under production + scope; exclusive = added beyond production.</p>
        </section>
      )}

      {/* Chokepoints */}
      {chokepoints.length > 0 && (
        <section>
          <SectionHeader>Chokepoints</SectionHeader>
          <p className="text-muted text-micro mb-1.5">packages that solely gate access to large parts of the graph.</p>
          <ul className="flex flex-col gap-1">
            {chokepoints.map((c) => (
              <li key={c.node.id}>
                <RowButton
                  onSelect={() => selectNode(c.node.id)}
                  ariaLabel={`${shortLabel(c.node.purl)} dominates ${c.dominated} packages`}
                >
                  <span className="font-mono text-bad text-mini tabular-nums min-w-[34px]">{c.dominated}</span>
                  <span className="text-mini truncate">{shortLabel(c.node.purl)}</span>
                </RowButton>
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Classified cycles */}
      <section>
        <SectionHeader count={cycles.length}>Cycles</SectionHeader>
        {cycles.length === 0 ? (
          <p className="text-muted text-mini">no dependency cycles.</p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {cycles.slice(0, 8).map((c, i) => (
              <li key={i} className="px-2.5 py-1.5 rounded-md bg-panel-2">
                <div className="flex items-center gap-2 text-mini">
                  <Badge tone={CYCLE_TONE[c.class] ?? 'muted'}>{c.class}</Badge>
                  <span className="text-muted">{c.members.length} members</span>
                  <span className="ml-auto"><MetricPill label="blast" value={c.external_dependents} /></span>
                </div>
                <div className="text-micro text-muted truncate mt-0.5">
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
          <SectionHeader>Fragmentation</SectionHeader>
          <p className="text-mini">{frag.note}</p>
          {frag.subsystem_islands.length > 0 && (
            <ul className="flex flex-col gap-1 mt-1.5">
              {frag.subsystem_islands.slice(0, 5).map((isl, i) => (
                <li key={i} className="text-micro text-muted px-2.5 py-1 rounded-md bg-panel-2">
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

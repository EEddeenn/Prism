'use client';

import { useEffect, useMemo, useState } from 'react';
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
  Subsystem, DiamondReuse, MinimalView, OverlapView,
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
 * Render a purl's shortLabel: a clickable button selecting its node when the purl
 * maps to one in the loaded graph (e.g. a versioned purl), else plain text (e.g. a
 * base purl with no `@version`).
 */
function PurlLabel({
  purl, id, selectNode, className,
}: {
  purl: string;
  id: number | undefined;
  selectNode: (id: number) => void;
  className?: string;
}) {
  const label = shortLabel(purl);
  if (id == null) return <span className={className}>{label}</span>;
  return (
    <button
      type="button"
      onClick={() => selectNode(id)}
      className={`text-left hover:text-accent transition-colors ${className ?? ''}`}
    >
      {label}
    </button>
  );
}

/**
 * The Structure tab: a server-computed structural-analysis dashboard — risk
 * profile, scope footprints, chokepoints, classified cycles, and fragmentation —
 * surfacing the prism-api structural views the 2D/3D leaderboards cannot.
 */
export default function StructureTab() {
  const run = useGraphStore((s) => s.run);
  const nodes = useGraphStore((s) => s.nodes);
  const selectNode = useGraphStore((s) => s.selectNode);

  const [risk, setRisk] = useState<RiskProfile | null>(null);
  const [scope, setScope] = useState<ScopeRow[]>([]);
  const [chokepoints, setChokepoints] = useState<Chokepoint[]>([]);
  const [cycles, setCycles] = useState<ClassifiedCycle[]>([]);
  const [frag, setFrag] = useState<Fragmentation | null>(null);
  const [subsystems, setSubsystems] = useState<Subsystem[]>([]);
  const [diamonds, setDiamonds] = useState<DiamondReuse | null>(null);
  const [minimal, setMinimal] = useState<MinimalView | null>(null);
  const [shared, setShared] = useState<OverlapView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  /** purl → node id, for making owner/foundation purls clickable. */
  const purlToId = useMemo(() => {
    const m = new Map<string, number>();
    for (const n of nodes) m.set(n.purl, n.id);
    return m;
  }, [nodes]);

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
      api.subsystems(run),
      api.diamonds(run, 8),
      api.minimal(run),
      api.shared(run, 8),
    ])
      .then(([r, sc, cp, cy, fr, sub, dia, min, shr]) => {
        if (!live) return;
        setRisk(r); setScope(sc); setChokepoints(cp); setCycles(cy); setFrag(fr);
        setSubsystems(sub); setDiamonds(dia); setMinimal(min); setShared(shr);
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

      {/* Subsystems — the top-level partition */}
      {subsystems.length > 0 && (() => {
        const sorted = [...subsystems].sort((a, b) => b.size - a.size);
        const maxSize = sorted[0]?.size ?? 1;
        const shown = sorted.slice(0, 8);
        const more = sorted.length - shown.length;
        return (
          <section>
            <SectionHeader count={subsystems.length}>Subsystems</SectionHeader>
            <p className="text-muted text-micro mb-1.5">the top-level partition: who owns what.</p>
            <ul className="flex flex-col gap-1">
              {shown.map((sub) => (
                <li key={sub.owner_purl} className="flex items-center gap-2">
                  <PurlLabel
                    purl={sub.owner_purl}
                    id={purlToId.get(sub.owner_purl)}
                    selectNode={selectNode}
                    className="text-mini truncate min-w-0 flex-[2]"
                  />
                  <StatBar
                    value={sub.size}
                    max={maxSize}
                    tone="accent"
                    ariaLabel={`${shortLabel(sub.owner_purl)} subsystem size ${sub.size}`}
                    className="flex-1"
                  />
                  <span className="w-[34px] text-right text-mini tabular-nums shrink-0">{sub.size}</span>
                  <span className="w-[44px] text-right text-micro text-muted tabular-nums shrink-0" title={`${sub.exclusive} exclusive`}>
                    {sub.exclusive} excl
                  </span>
                  <Badge tone={sub.runtime ? 'ok' : 'muted'}>{sub.runtime ? 'runtime' : 'dev'}</Badge>
                </li>
              ))}
            </ul>
            {more > 0 && <p className="text-muted text-micro mt-1">… +{more} more</p>}
          </section>
        );
      })()}

      {/* Shared surfaces — packages spanning multiple subsystems */}
      {shared && (
        <section>
          <SectionHeader>Shared surfaces</SectionHeader>
          {shared.shared_surfaces.length === 0 ? (
            <p className="text-muted text-mini">
              Only {shared.subsystems} subsystem{shared.subsystems === 1 ? '' : 's'} — no packages span multiple subsystems.
            </p>
          ) : (
            <>
              <p className="text-muted text-micro mb-1.5">packages reached across more than one subsystem.</p>
              <ul className="flex flex-col gap-1">
                {shared.shared_surfaces.map((s) => (
                  <li key={s.purl} className="flex items-center gap-2 text-mini">
                    <PurlLabel
                      purl={s.purl}
                      id={purlToId.get(s.purl)}
                      selectNode={selectNode}
                      className="truncate min-w-0 flex-1"
                    />
                    <span className="text-muted text-micro tabular-nums shrink-0">in {s.spread} subsystems</span>
                  </li>
                ))}
              </ul>
              {shared.top_pairs.length > 0 && (
                <ul className="flex flex-col gap-1 mt-1.5">
                  {shared.top_pairs.map((p, i) => (
                    <li key={i} className="text-micro text-muted px-2.5 py-1 rounded-md bg-panel-2 truncate">
                      <span className="text-ink">{shortLabel(p.a)}</span> ∩ <span className="text-ink">{shortLabel(p.b)}</span>:{' '}
                      <span className="tabular-nums">{p.shared}</span>
                    </li>
                  ))}
                </ul>
              )}
            </>
          )}
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

      {/* Diamond reuse */}
      {diamonds && (
        <section>
          <SectionHeader>Diamond reuse</SectionHeader>
          <div className="flex items-baseline gap-2 mb-1">
            <span className="text-head font-semibold tabular-nums">{diamonds.total_reconvergence}</span>
            <span className="text-muted text-mini">reconvergence</span>
          </div>
          {diamonds.foundations.length > 0 && (
            <p className="text-muted text-micro mb-1.5">
              top {diamonds.foundations.length} foundation{diamonds.foundations.length === 1 ? '' : 's'} carry {Math.round(diamonds.top_share_pct)}% of reconvergence.
            </p>
          )}
          {diamonds.foundations.length === 0 ? (
            <p className="text-muted text-mini">no shared foundations.</p>
          ) : (
            <ul className="flex flex-col gap-1">
              {diamonds.foundations.map((f) => (
                <li key={f.purl} className="flex items-center gap-2 text-mini">
                  <PurlLabel
                    purl={f.purl}
                    id={purlToId.get(f.purl)}
                    selectNode={selectNode}
                    className="truncate min-w-0 flex-1"
                  />
                  <span className="text-muted text-micro tabular-nums shrink-0" title={`${f.parents} parents`}>{f.parents} parents</span>
                  <span className="text-accent text-micro tabular-nums shrink-0 w-[40px] text-right" title={`${f.reconvergence} reconvergence`}>{f.reconvergence}</span>
                </li>
              ))}
            </ul>
          )}
          <p className="text-muted text-micro mt-1">shared foundations reached along many paths — the diamonds that create reconvergence (≈ the H₁ loop count).</p>
        </section>
      )}

      {/* Minimal skeleton */}
      {minimal && (
        <section>
          <SectionHeader>Minimal skeleton</SectionHeader>
          <div className="flex items-baseline gap-2 mb-1">
            <span className="text-head font-semibold tabular-nums">{minimal.original_edges}</span>
            <span className="text-muted text-mini">→</span>
            <span className="text-head font-semibold tabular-nums">{minimal.reduced_edges}</span>
            {minimal.original_edges > 0 && (
              <span className="text-muted text-mini tabular-nums">
                −{Math.round(100 * (1 - minimal.reduced_edges / minimal.original_edges))}%
              </span>
            )}
          </div>
          <p className="text-muted text-micro">the dominator skeleton: the edges that actually control reachability.</p>
          {minimal.edges.length > 0 && (
            <ul className="flex flex-col gap-1 mt-1.5">
              {minimal.edges.slice(0, 6).map((e, i) => (
                <li key={i} className="flex items-center gap-1.5 text-micro text-muted truncate">
                  <span className="text-ink truncate min-w-0">{shortLabel(e.gateway)}</span>
                  <span className="shrink-0">→</span>
                  <span className="text-ink truncate min-w-0">{shortLabel(e.node)}</span>
                </li>
              ))}
            </ul>
          )}
        </section>
      )}

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

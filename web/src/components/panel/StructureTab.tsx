'use client';

import { useEffect, useMemo, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { api, ApiError } from '@/lib/api';
import { shortLabel } from '@/lib/palette';
import FidelityBanner from '@/components/FidelityBanner';
import {
  Section,
  SectionHeader,
  Eyebrow,
  Badge,
  MetricPill,
  Callout,
  PanelState,
  DataTable,
  HeroFigure,
  RankedBarRow,
  PurlLabel,
  DataCard,
  TruncationNote,
} from '@/design-system';
import { type Tone, severity, pickTone } from '@/lib/tone';
import type {
  RiskProfile, Chokepoint, ClassifiedCycle, Fragmentation, ScopeRow,
  Subsystem, DiamondReuse, MinimalView, OverlapView,
} from '@/lib/types';

/** Cycle class → tone. These are classifier verdicts (not edge kinds), so they
 *  resolve through `pickTone` against this small map. */
const CYCLE_TONE: Record<string, Tone> = {
  'runtime-coupling': 'bad',
  'runtime-cluster': 'warn',
  'tooling-cycle': 'neutral',
};

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
  if (error) return <PanelState kind="error">{error}</PanelState>;
  if (loading && !risk) return <PanelState kind="loading">Computing structural analysis…</PanelState>;

  return (
    <div className="flex flex-col gap-3.5">
      <FidelityBanner />

      {/* Risk profile */}
      {risk && (
        <section>
          <Eyebrow>Structural risk</Eyebrow>
          <HeroFigure
            value={risk.overall}
            unit={`/ 100 · ${risk.band}`}
            tone={severity(risk.overall)}
          />
          <div className="mt-2 flex flex-col gap-1.5">
            {risk.dimensions.map((d) => (
              <RankedBarRow
                key={d.name}
                label={<span className="text-data-s text-archive">{d.name}</span>}
                value={d.score}
                max={100}
                tone={severity(d.score)}
                title={d.detail}
                ariaLabel={`${d.name} risk ${d.score} of 100`}
              />
            ))}
          </div>
        </section>
      )}

      {/* Scope footprints */}
      {scope.length > 0 && (
        <Section title="Scope footprints">
          <DataTable
            caption="Scope footprints"
            columns={[
              { key: 'kind', header: 'scope' },
              { key: 'footprint', header: 'footprint', align: 'right' },
              { key: 'exclusive', header: 'exclusive', align: 'right' },
              { key: 'max_depth', header: 'depth', align: 'right' },
            ]}
            rows={scope.map((s) => ({
              key: s.kind,
              cells: {
                kind: s.kind,
                footprint: s.footprint,
                exclusive: s.exclusive,
                max_depth: s.max_depth,
              },
            }))}
          />
          <p className="mt-1 font-mono text-meta text-dust">footprint = reachable under production + scope; exclusive = added beyond production.</p>
        </Section>
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
            <p className="mb-1.5 font-mono text-meta text-dust">the top-level partition: who owns what.</p>
            <ul className="flex flex-col gap-1">
              {shown.map((sub) => {
                const id = purlToId.get(sub.owner_purl);
                return (
                  <li key={sub.owner_purl}>
                    <RankedBarRow
                      label={
                        <PurlLabel
                          purl={sub.owner_purl}
                          onSelect={id != null ? () => selectNode(id) : undefined}
                          className="text-data-s"
                        />
                      }
                      value={sub.size}
                      max={maxSize}
                      tone="accent"
                      sub={`${sub.exclusive} excl`}
                      badge={<Badge tone={sub.runtime ? 'good' : 'neutral'}>{sub.runtime ? 'runtime' : 'dev'}</Badge>}
                      ariaLabel={`${shortLabel(sub.owner_purl)} subsystem size ${sub.size}, ${sub.exclusive} exclusive`}
                    />
                  </li>
                );
              })}
            </ul>
            {more > 0 && <TruncationNote shown={shown.length} total={sorted.length} more={more} className="mt-1" />}
          </section>
        );
      })()}

      {/* Shared surfaces — packages spanning multiple subsystems */}
      {shared && (
        <section>
          <SectionHeader>Shared surfaces</SectionHeader>
          {shared.shared_surfaces.length === 0 ? (
            <p className="text-data-s text-archive">
              Only {shared.subsystems} subsystem{shared.subsystems === 1 ? '' : 's'} — no packages span multiple subsystems.
            </p>
          ) : (
            <>
              <p className="mb-1.5 font-mono text-meta text-dust">packages reached across more than one subsystem.</p>
              <ul className="flex flex-col gap-1">
                {shared.shared_surfaces.map((s) => {
                  const id = purlToId.get(s.purl);
                  return (
                    <li key={s.purl} className="flex items-center gap-2 text-data-s">
                      <PurlLabel
                        purl={s.purl}
                        onSelect={id != null ? () => selectNode(id) : undefined}
                        className="min-w-0 flex-1 text-data-s"
                      />
                      <span className="shrink-0 font-mono tabular-nums text-meta text-archive">in {s.spread} subsystems</span>
                    </li>
                  );
                })}
              </ul>
              {shared.top_pairs.length > 0 && (
                <ul className="mt-1.5 flex flex-col gap-1">
                  {shared.top_pairs.map((p, i) => (
                    <li key={i} className="truncate rounded-control bg-charcoal px-2.5 py-1 font-mono text-meta text-archive">
                      <span className="text-bone">{shortLabel(p.a)}</span> ∩ <span className="text-bone">{shortLabel(p.b)}</span>:{' '}
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
          <p className="mb-1.5 font-mono text-meta text-dust">packages that solely gate access to large parts of the graph.</p>
          <ul className="flex flex-col gap-1">
            {chokepoints.map((c) => (
              <li key={c.node.id}>
                <RankedBarRow
                  label={<PurlLabel purl={c.node.purl} className="text-data-s" />}
                  value={c.dominated}
                  max={chokepoints[0]?.dominated ?? 1}
                  tone="bad"
                  onSelect={() => selectNode(c.node.id)}
                  ariaLabel={`${shortLabel(c.node.purl)} dominates ${c.dominated} packages`}
                />
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Classified cycles */}
      <section>
        <SectionHeader count={cycles.length}>Cycles</SectionHeader>
        {cycles.length === 0 ? (
          <p className="text-data-s text-archive">no dependency cycles.</p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {cycles.slice(0, 8).map((c, i) => (
              <DataCard
                key={i}
                right={<MetricPill label="blast" value={c.external_dependents} />}
              >
                <div className="flex items-center gap-2 text-data-s">
                  <Badge tone={pickTone(CYCLE_TONE, c.class)}>{c.class}</Badge>
                  <span className="font-mono tabular-nums text-archive">{c.members.length} members</span>
                </div>
                <div className="mt-0.5 truncate font-mono text-meta text-archive">
                  {c.members.map((m) => shortLabel(m.purl)).join(' ⇄ ')}
                </div>
              </DataCard>
            ))}
          </ul>
        )}
      </section>

      {/* Diamond reuse */}
      {diamonds && (
        <section>
          <SectionHeader>Diamond reuse</SectionHeader>
          <HeroFigure value={diamonds.total_reconvergence} label="reconvergence" tone="neutral" />
          {diamonds.foundations.length > 0 && (
            <p className="mb-1.5 mt-2 font-mono text-meta text-dust">
              top {diamonds.foundations.length} foundation{diamonds.foundations.length === 1 ? '' : 's'} carry {Math.round(diamonds.top_share_pct)}% of reconvergence.
            </p>
          )}
          {diamonds.foundations.length === 0 ? (
            <p className="mt-2 text-data-s text-archive">no shared foundations.</p>
          ) : (
            <ul className="flex flex-col gap-1">
              {diamonds.foundations.map((f) => {
                const id = purlToId.get(f.purl);
                return (
                  <li key={f.purl} className="flex items-center gap-2 text-data-s">
                    <PurlLabel
                      purl={f.purl}
                      onSelect={id != null ? () => selectNode(id) : undefined}
                      className="min-w-0 flex-1 text-data-s"
                    />
                    <span className="shrink-0 font-mono tabular-nums text-meta text-archive" title={`${f.parents} parents`}>{f.parents} parents</span>
                    <span className="w-[40px] shrink-0 text-right font-mono tabular-nums text-meta text-bone" title={`${f.reconvergence} reconvergence`}>{f.reconvergence}</span>
                  </li>
                );
              })}
            </ul>
          )}
          <p className="mt-1 font-mono text-meta text-dust">shared foundations reached along many paths — the diamonds that create reconvergence (≈ the H₁ loop count).</p>
        </section>
      )}

      {/* Minimal skeleton */}
      {minimal && (
        <section>
          <SectionHeader>Minimal skeleton</SectionHeader>
          <div className="flex items-baseline gap-2">
            <span className="font-mono text-h3 tabular-nums text-bone">{minimal.original_edges}</span>
            <span className="font-mono text-data-s text-archive">→</span>
            <span className="font-mono text-h3 tabular-nums text-bone">{minimal.reduced_edges}</span>
            {minimal.original_edges > 0 && (
              <span className="font-mono text-data-s tabular-nums text-archive">
                −{Math.round(100 * (1 - minimal.reduced_edges / minimal.original_edges))}%
              </span>
            )}
          </div>
          <p className="mt-1 font-mono text-meta text-dust">the dominator skeleton: the edges that actually control reachability.</p>
          {minimal.edges.length > 0 && (
            <ul className="mt-1.5 flex flex-col gap-1">
              {minimal.edges.slice(0, 6).map((e, i) => (
                <li key={i} className="flex items-center gap-1.5 truncate font-mono text-meta text-archive">
                  <span className="min-w-0 truncate text-bone">{shortLabel(e.gateway)}</span>
                  <span className="shrink-0">→</span>
                  <span className="min-w-0 truncate text-bone">{shortLabel(e.node)}</span>
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
          <p className="text-data-s text-bone">{frag.note}</p>
          {frag.subsystem_islands.length > 0 && (
            <ul className="mt-1.5 flex flex-col gap-1">
              {frag.subsystem_islands.slice(0, 5).map((isl, i) => (
                <li key={i} className="rounded-control bg-charcoal px-2.5 py-1 font-mono text-meta text-archive">
                  <span className="font-semibold tabular-nums text-bone">{isl.size}</span> pkgs
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

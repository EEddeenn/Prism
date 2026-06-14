'use client';

/* DOM overlays drawn OVER the canvas (never inside it, so they cost nothing per
 * frame): a hover/selection readout, the terrain elevation legend, and a hint
 * for TDA mode before a feature is picked. All read the shared store — `hovered`
 * lives there so a hover change re-renders ONLY this DOM, never the WebGL scene. */

import { useMemo } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { rampHex, PALETTE } from '@/lib/palette';
import { signalValue, SIGNAL_LABELS, REACH_SIGNALS } from '@/lib/layout3d';
import HoverCard from '@/components/HoverCard';
import { Hud, Eyebrow, Button } from '@/design-system';

function fmt(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(n >= 10000 ? 0 : 1)}k`;
  return String(Math.round(n));
}

function TerrainLegend() {
  const signal = useGraphStore((s) => s.signal);
  const nodes = useGraphStore((s) => s.nodes);
  const index = useGraphStore((s) => s.index);

  const max = useMemo(() => {
    if (!index) return 0;
    let mx = 0;
    for (const n of nodes) mx = Math.max(mx, signalValue(index.metrics.get(n.id)!, signal));
    return mx;
  }, [nodes, index, signal]);

  const meta = SIGNAL_LABELS[signal];
  const sqrtHeight = REACH_SIGNALS.has(signal);
  const approx = !!index?.approximate && sqrtHeight;
  // 6-stop CSS gradient sampled from the elevation ramp (top = high).
  const stops = [1, 0.8, 0.6, 0.4, 0.2, 0].map((t) => rampHex(t));
  const gradient = `linear-gradient(to top, ${[0, 0.2, 0.4, 0.6, 0.8, 1].map((t) => rampHex(t)).join(', ')})`;
  // The colour bar runs 0..1 in HEIGHT; under √ compression the midpoint height
  // maps back to ¼·max (height f ↦ value f²·max), so label the value honestly.
  const midValue = sqrtHeight ? max * 0.25 : max * 0.5;

  return (
    <Hud className="absolute top-3.5 right-3.5 z-10 p-2.5 text-data-s">
      <Eyebrow className="mb-0.5">{meta.name}</Eyebrow>
      <div className="text-archive text-meta mb-1.5">height · {meta.unit}{sqrtHeight ? ' · √ scale' : ''}</div>
      <div className="flex items-stretch gap-2">
        <div className="w-3 rounded-control" style={{ background: gradient, minHeight: 84 }} aria-hidden />
        <div className="flex flex-col justify-between text-archive font-mono tabular-nums">
          <span style={{ color: stops[0] }}>{fmt(max)}</span>
          <span>{fmt(midValue)}</span>
          <span style={{ color: stops[5] }}>0</span>
        </div>
      </div>
      {approx && (
        <div className="text-meta text-warn mt-1.5 max-w-[150px] leading-snug">
          approximate (graph over reach cap — degree proxy)
        </div>
      )}
    </Hud>
  );
}

function TdaHint() {
  const barcode = useGraphStore((s) => s.barcode);
  const setActiveTab = useGraphStore((s) => s.setActiveTab);
  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center pointer-events-none">
      <Hud className="px-4 py-3 text-center max-w-[320px] pointer-events-auto">
        <div className="text-bone text-h4 font-semibold mb-1">Pick a topology feature</div>
        <p className="text-archive text-data-s leading-snug">
          {barcode ? (
            <>
              This run has <span className="font-mono tabular-nums">{barcode.component_count}</span> H₀ component(s)
              and <span className="font-mono tabular-nums">{barcode.cycle_count}</span> H₁ loop(s).{' '}
            </>
          ) : (
            ''
          )}
          Choose one in the <span className="text-cyan-gray">TDA</span> panel to light up the packages and edges
          that generate it.
        </p>
        <Button variant="quiet" size="sm" onClick={() => setActiveTab('tda')} className="mt-2">
          Open TDA panel
        </Button>
      </Hud>
    </div>
  );
}

function FeatureCard() {
  const feature = useGraphStore((s) => s.feature);
  const clearFeature = useGraphStore((s) => s.clearFeature);
  const setActiveTab = useGraphStore((s) => s.setActiveTab);
  if (!feature) return null;

  // Dim-2 voids read VIOLET; H₀/H₁ generators read GOLD.
  const isVoid = feature.dim === 2;
  const accent = isVoid ? PALETTE.void : PALETTE.gen;
  const label = isVoid ? 'H₂ void' : `H${feature.dim} generator`;

  return (
    <Hud className="absolute top-3.5 left-3.5 z-10 p-2.5 text-data-s">
      <div className="font-semibold mb-0.5" style={{ color: accent }}>{label}</div>
      <div className="text-archive font-mono tabular-nums">
        {feature.nodeIds.size} package(s) · {feature.edgeKeys.size} arc(s) lit
      </div>
      <div className="flex gap-1.5 mt-2">
        <Button variant="quiet" size="sm" onClick={() => clearFeature()}>
          Clear
        </Button>
        <Button variant="secondary" size="sm" onClick={() => setActiveTab('tda')}>
          Open TDA panel
        </Button>
      </div>
    </Hud>
  );
}

export default function SceneHud() {
  const mode = useGraphStore((s) => s.mode3d);
  const feature = useGraphStore((s) => s.feature);
  return (
    <>
      <HoverCard />
      {mode === 'terrain' && <TerrainLegend />}
      {mode === 'tda' && !feature && <TdaHint />}
      {mode === 'tda' && feature && <FeatureCard />}
    </>
  );
}

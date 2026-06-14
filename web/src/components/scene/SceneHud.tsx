'use client';

/* DOM overlays drawn OVER the canvas (never inside it, so they cost nothing per
 * frame): a hover/selection readout, the terrain elevation legend, and a hint
 * for TDA mode before a feature is picked. All read the shared store — `hovered`
 * lives there so a hover change re-renders ONLY this DOM, never the WebGL scene. */

import { useMemo } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { rampHex } from '@/lib/palette';
import { signalValue, SIGNAL_LABELS, REACH_SIGNALS } from '@/lib/layout3d';
import HoverCard from '@/components/HoverCard';

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
  const approx = !!index?.approximate && REACH_SIGNALS.has(signal);
  // 6-stop CSS gradient sampled from the elevation ramp (top = high).
  const stops = [1, 0.8, 0.6, 0.4, 0.2, 0].map((t) => rampHex(t));
  const gradient = `linear-gradient(to top, ${[0, 0.2, 0.4, 0.6, 0.8, 1].map((t) => rampHex(t)).join(', ')})`;

  return (
    <div className="absolute top-3.5 right-3.5 z-10 p-2.5 rounded-xl border border-line bg-panel/90 backdrop-blur-sm shadow-panel text-mini">
      <div className="font-semibold text-ink mb-0.5">{meta.name}</div>
      <div className="text-muted text-micro mb-1.5">height · {meta.unit}</div>
      <div className="flex items-stretch gap-2">
        <div className="w-3 rounded" style={{ background: gradient, minHeight: 84 }} aria-hidden />
        <div className="flex flex-col justify-between text-muted tabular-nums">
          <span style={{ color: stops[0] }}>{fmt(max)}</span>
          <span>{fmt(max / 2)}</span>
          <span style={{ color: stops[5] }}>0</span>
        </div>
      </div>
      {approx && (
        <div className="text-micro text-warn mt-1.5 max-w-[150px] leading-snug">
          approximate (graph over reach cap — degree proxy)
        </div>
      )}
    </div>
  );
}

function TdaHint() {
  const barcode = useGraphStore((s) => s.barcode);
  const setActiveTab = useGraphStore((s) => s.setActiveTab);
  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center pointer-events-none">
      <div className="px-4 py-3 rounded-xl border border-line bg-panel/85 backdrop-blur-sm shadow-panel text-center max-w-[320px] pointer-events-auto">
        <div className="text-ink text-sm font-semibold mb-1">Pick a topology feature</div>
        <p className="text-muted text-mini leading-snug">
          {barcode
            ? `This run has ${barcode.component_count} H₀ component(s) and ${barcode.cycle_count} H₁ loop(s). `
            : ''}
          Choose one in the <span className="text-accent">TDA</span> panel to light up the packages and edges
          that generate it.
        </p>
        <button
          type="button"
          onClick={() => setActiveTab('tda')}
          className="mt-2 px-2.5 py-1 text-mini hover:border-accent"
        >
          Open TDA panel
        </button>
      </div>
    </div>
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
    </>
  );
}

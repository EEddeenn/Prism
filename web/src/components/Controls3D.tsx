'use client';

import { useState } from 'react';
import { SegmentedControl, type SegOption } from '@/components/ui';
import type { Mode3D, TerrainSignal } from '@/lib/types';

const MODES: { id: Mode3D; label: string; title: string }[] = [
  { id: 'layered', label: 'Layered', title: 'Depth as stacked layers' },
  { id: 'fiber', label: 'Fiber', title: 'Each family as a vertical version fiber' },
  { id: 'terrain', label: 'Terrain', title: 'Graph as an analytical heightfield' },
  { id: 'tda', label: 'TDA', title: "Highlight a persistence feature's generators" },
];

const MODE_OPTIONS: SegOption<Mode3D>[] = MODES.map((m) => ({
  value: m.id,
  label: m.label,
  title: m.title,
}));

const SIGNALS: { id: TerrainSignal; label: string }[] = [
  { id: 'centrality', label: 'centrality — transitive dependents' },
  { id: 'reach', label: 'reach — transitive dependencies' },
  { id: 'fanin', label: 'fan-in — direct dependents' },
  { id: 'fanout', label: 'fan-out — direct dependencies' },
  { id: 'duplicates', label: 'duplicate pressure' },
  { id: 'depth', label: 'depth — TDA filtration value' },
  { id: 'risk', label: 'risk — composite' },
];

// What each mode encodes — shown under the mode switch so the view is readable
// without leaving the canvas.
const MODE_HELP: Record<Mode3D, string> = {
  layered: 'Stacked by dependency depth — roots on top, leaves below. Red edges rise across layers = cycle back-edges.',
  fiber: 'One vertical fiber per package family. Bright pink + taller = resolved at multiple versions.',
  terrain: 'The 2D map lifted into a heightfield; pick the signal below. Peaks are labelled; see the legend, top-right.',
  tda: 'Gold lights the generator support of the persistence feature picked in the TDA panel.',
};

interface Props {
  mode: Mode3D;
  setMode: (m: Mode3D) => void;
  signal: TerrainSignal;
  setSignal: (s: TerrainSignal) => void;
  spin: boolean;
  toggleSpin: () => void;
  onReset: () => void;
}

/** × glyph for the header's close button. */
function CloseIcon() {
  return (
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={3} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </svg>
  );
}

/** Sliders glyph for the collapsed-state reopen button — telegraphs "view
 *  controls" without needing a label, matching the conventional UI pattern. */
function SlidersIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <line x1="4" y1="6" x2="20" y2="6" />
      <line x1="4" y1="12" x2="20" y2="12" />
      <line x1="4" y1="18" x2="20" y2="18" />
      <circle cx="9" cy="6" r="2" fill="currentColor" />
      <circle cx="15" cy="12" r="2" fill="currentColor" />
      <circle cx="11" cy="18" r="2" fill="currentColor" />
    </svg>
  );
}

export default function Controls3D({ mode, setMode, signal, setSignal, spin, toggleSpin, onReset }: Props) {
  // Local collapsed state — gives users a way to clear the top-left chrome
  // when they want an unobstructed view of the topology. State is component-
  // local (not URL or store) because the right answer depends on viewport
  // size and current focus, and shouldn't bleed into shared deep links.
  const [collapsed, setCollapsed] = useState(false);

  if (collapsed) {
    // Collapsed: a tiny floating button in the same top-left position so
    // the user knows where the controls went and how to bring them back.
    return (
      <button
        type="button"
        onClick={() => setCollapsed(false)}
        title="Show 3D controls"
        aria-label="Show 3D controls"
        className="absolute left-3.5 top-3.5 z-10 w-9 h-9 inline-flex items-center justify-center rounded-card border border-line bg-panel/90 shadow-panel backdrop-blur-sm text-muted hover:text-ink hover:border-accent transition-colors"
      >
        <SlidersIcon />
      </button>
    );
  }

  return (
    <div className="absolute left-3.5 top-3.5 z-10 flex flex-col gap-2 p-2.5 max-w-[330px] rounded-card border border-line bg-panel/90 shadow-panel backdrop-blur-sm">
      {/* Header strip: title + close button. Establishes the box's identity
          and surfaces the hide affordance unambiguously. */}
      <div className="flex items-center justify-between gap-2 -mb-0.5">
        <span className="text-micro uppercase tracking-[1.5px] text-muted">3D controls</span>
        <button
          type="button"
          onClick={() => setCollapsed(true)}
          title="Hide 3D controls"
          aria-label="Hide 3D controls"
          className="w-5 h-5 inline-flex items-center justify-center rounded-control text-muted hover:text-bad hover:bg-panel-2 transition-colors"
        >
          <CloseIcon />
        </button>
      </div>

      <SegmentedControl
        options={MODE_OPTIONS}
        value={mode}
        onChange={setMode}
        ariaLabel="3D mode"
        size="sm"
        className="w-full [&>button]:flex-1"
      />

      {mode === 'terrain' && (
        <div className="flex items-center gap-2">
          <label htmlFor="signal" className="text-mini text-muted shrink-0">Height</label>
          <select
            id="signal"
            value={signal}
            onChange={(e) => setSignal(e.target.value as TerrainSignal)}
            className="flex-1 px-2 py-1 text-mini"
          >
            {SIGNALS.map((s) => (
              <option key={s.id} value={s.id}>{s.label}</option>
            ))}
          </select>
        </div>
      )}

      <p className="text-micro leading-snug text-ink/80">{MODE_HELP[mode]}</p>
      <p className="text-micro leading-snug text-muted">
        Drag to orbit · scroll to zoom · shift-drag to pan · hover to identify · click a node
      </p>

      <div className="flex gap-2">
        <button
          type="button"
          onClick={toggleSpin}
          aria-pressed={spin}
          className={`flex-1 px-2 py-1.5 text-mini rounded-control border ${
            spin ? 'bg-accent text-onAccent border-accent font-semibold' : 'border-line-2 text-muted hover:border-accent'
          }`}
        >
          Spin
        </button>
        <button
          type="button"
          onClick={onReset}
          className="flex-1 px-2 py-1.5 text-mini rounded-control border border-line-2 text-muted hover:border-accent"
        >
          Reset view
        </button>
      </div>
    </div>
  );
}

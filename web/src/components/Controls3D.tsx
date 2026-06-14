'use client';

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

export default function Controls3D({ mode, setMode, signal, setSignal, spin, toggleSpin, onReset }: Props) {
  return (
    <div className="absolute left-3.5 top-3.5 z-10 flex flex-col gap-2 p-2.5 max-w-[330px] rounded-xl border border-line bg-panel/90 shadow-panel backdrop-blur-sm">
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
          className={`flex-1 px-2 py-1.5 text-mini rounded-md border ${
            spin ? 'bg-accent text-onAccent border-accent font-semibold' : 'border-line-2 text-muted hover:border-accent'
          }`}
        >
          Spin
        </button>
        <button
          type="button"
          onClick={onReset}
          className="flex-1 px-2 py-1.5 text-mini rounded-md border border-line-2 text-muted hover:border-accent"
        >
          Reset view
        </button>
      </div>
    </div>
  );
}

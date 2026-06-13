'use client';

import type { Mode3D, TerrainSignal } from '@/lib/types';

const MODES: { id: Mode3D; label: string; title: string }[] = [
  { id: 'layered', label: 'Layered', title: 'Depth as stacked layers' },
  { id: 'fiber', label: 'Fiber', title: 'Each family as a vertical version fiber' },
  { id: 'terrain', label: 'Terrain', title: 'Graph as an analytical heightfield' },
  { id: 'tda', label: 'TDA', title: "Highlight a persistence feature's generators" },
];

const SIGNALS: { id: TerrainSignal; label: string }[] = [
  { id: 'centrality', label: 'centrality — transitive dependents' },
  { id: 'reach', label: 'reach — transitive dependencies' },
  { id: 'fanin', label: 'fan-in — direct dependents' },
  { id: 'fanout', label: 'fan-out — direct dependencies' },
  { id: 'duplicates', label: 'duplicate pressure' },
  { id: 'depth', label: 'depth — TDA filtration value' },
  { id: 'risk', label: 'risk — composite' },
];

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
      <div className="flex">
        {MODES.map((m, i) => (
          <button
            key={m.id}
            type="button"
            title={m.title}
            onClick={() => setMode(m.id)}
            className={`flex-1 px-2 py-1.5 text-[11.5px] font-semibold border border-line-2 ${
              i === 0 ? 'rounded-l-lg' : i === MODES.length - 1 ? 'rounded-r-lg border-l-0' : 'border-l-0'
            } ${mode === m.id ? 'bg-panel-3 text-ink border-accent' : 'bg-panel-2 text-muted'}`}
          >
            {m.label}
          </button>
        ))}
      </div>

      {mode === 'terrain' && (
        <div className="flex items-center gap-2">
          <label htmlFor="signal" className="text-[11.5px] text-muted shrink-0">Height</label>
          <select
            id="signal"
            value={signal}
            onChange={(e) => setSignal(e.target.value as TerrainSignal)}
            className="flex-1 px-2 py-1 text-[11.5px]"
          >
            {SIGNALS.map((s) => (
              <option key={s.id} value={s.id}>{s.label}</option>
            ))}
          </select>
        </div>
      )}

      <p className="text-[10.5px] leading-snug text-muted">
        Drag to orbit · scroll to zoom · shift-drag to pan · click a node
      </p>

      <div className="flex gap-2">
        <button
          type="button"
          onClick={toggleSpin}
          className={`flex-1 px-2 py-1.5 text-[11.5px] ${spin ? 'bg-accent text-[#06203f] border-accent' : ''}`}
        >
          Spin
        </button>
        <button type="button" onClick={onReset} className="flex-1 px-2 py-1.5 text-[11.5px] hover:border-accent">
          Reset view
        </button>
      </div>
    </div>
  );
}

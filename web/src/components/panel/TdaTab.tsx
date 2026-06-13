'use client';

import { useGraphStore } from '@/store/useGraphStore';
import type { TdaPair } from '@/lib/types';

const fmt = (x: number | null) => (x == null ? '?' : String(Math.round(x * 10) / 10));

function Bar({ label, count, max, essential }: { label: string; count: number; max: number; essential?: boolean }) {
  return (
    <div className="flex items-center gap-2 my-1">
      <div className="w-[78px] shrink-0 text-muted text-[11.5px]">{label}</div>
      <div className="flex-1 h-3 bg-panel-2 rounded overflow-hidden">
        <div className={`h-full rounded ${essential ? 'bg-accent-2' : 'bg-accent'}`} style={{ width: `${Math.round((count / max) * 100)}%` }} />
      </div>
      <div className="w-[34px] shrink-0 text-right tabular-nums">{count}</div>
    </div>
  );
}

function FeatureRow({ p }: { p: TdaPair }) {
  const feature = useGraphStore((s) => s.feature);
  const selectFeature = useGraphStore((s) => s.selectFeature);
  const active = feature?.pair === p.pair;
  return (
    <div
      onClick={() => selectFeature(p.pair, p.dim)}
      className={`flex items-center gap-2 px-2.5 py-1.5 my-1 rounded-lg bg-panel-2 cursor-pointer border ${
        active ? 'border-node-center bg-panel-3' : 'border-transparent hover:border-accent hover:bg-panel-3'
      }`}
    >
      <span className="shrink-0 font-mono font-bold text-[11px] text-accent min-w-[22px]">H{p.dim}</span>
      <span className="flex-1 text-[11px] tabular-nums">
        {p.essential ? `born d${fmt(p.birth)} · never dies` : `d${fmt(p.birth)} → d${fmt(p.death)} · life ${fmt(p.persistence)}`}
      </span>
      <span className={`shrink-0 text-[10px] rounded-xl px-2 border ${p.essential ? 'text-node-center border-[#4a3a1e]' : 'text-muted border-line-2'}`}>
        {p.essential ? 'essential' : 'finite'}
      </span>
    </div>
  );
}

function Group({ title, list }: { title: string; list: TdaPair[] }) {
  if (!list.length) return null;
  const sorted = [...list].sort(
    (a, b) => Number(b.essential) - Number(a.essential) || (b.persistence ?? 0) - (a.persistence ?? 0) || a.pair - b.pair,
  );
  const CAP = 40;
  return (
    <div>
      <div className="text-[11px] uppercase tracking-wide text-muted mt-3 mb-1">{title} ({list.length})</div>
      {sorted.slice(0, CAP).map((p) => (
        <FeatureRow key={p.pair} p={p} />
      ))}
      {sorted.length > CAP && (
        <p className="text-muted text-[11px] mt-1.5">… and {sorted.length - CAP} more (showing the {CAP} longest-lived).</p>
      )}
    </div>
  );
}

export default function TdaTab() {
  const barcode = useGraphStore((s) => s.barcode);
  const pairs = useGraphStore((s) => s.pairs);
  const run = useGraphStore((s) => s.run);

  if (!barcode) {
    return (
      <div>
        <p className="text-muted text-sm">No persistent homology for this run yet.</p>
        <p className="text-muted text-sm mt-1.5">
          Compute it with <code className="bg-panel-2 px-1.5 rounded">prism tda run --run {run ?? '<run>'}</code>.
        </p>
      </div>
    );
  }

  const max = Math.max(1, ...barcode.dims.map((d) => d.total));

  return (
    <div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 mb-2">
        <dt className="text-muted">H₀ components</dt>
        <dd className="text-right tabular-nums font-semibold">{barcode.component_count}</dd>
        <dt className="text-muted">H₁ cycles</dt>
        <dd className="text-right tabular-nums font-semibold">{barcode.cycle_count}</dd>
      </dl>

      {barcode.dims.map((d) => (
        <div key={d.dim} className="mb-3">
          <div className="text-[11px] uppercase tracking-wide text-muted mb-1">Dimension H{d.dim}</div>
          <Bar label="finite" count={d.finite} max={max} />
          <Bar label="essential" count={d.essential} max={max} essential />
        </div>
      ))}

      {pairs.length > 0 && (
        <>
          <p className="text-muted text-[11px] mt-1.5">Click a feature to light up the packages that carry it (opens 3D · TDA).</p>
          <Group title="H₀ — connected components" list={pairs.filter((p) => p.dim === 0)} />
          <Group title="H₁ — loops" list={pairs.filter((p) => p.dim === 1)} />
        </>
      )}
    </div>
  );
}

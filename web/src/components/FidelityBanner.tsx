'use client';

import { useGraphStore } from '@/store/useGraphStore';

export default function FidelityBanner() {
  const runMeta = useGraphStore((s) => s.runMeta);

  if (!runMeta) return null;

  const caveats = runMeta.fidelity_caveats ?? [];
  const isExact = runMeta.fidelity_tier === 'lockfile-exact';

  if (isExact && caveats.length === 0) return null;

  return (
    <div className="border border-[#4a3a1e] bg-[#20180c] rounded-lg px-2.5 py-1.5 mb-2.5 text-[11px] text-warn">
      <div className="flex items-center gap-1.5">
        <span aria-hidden>⚠</span>
        <span>
          Resolution: <span className="font-mono text-bad">{runMeta.fidelity_tier}</span>
          {' — derived rankings are approximate'}
        </span>
      </div>
      {caveats.length > 0 && (
        <div className="flex flex-wrap gap-1 mt-1">
          {caveats.map((c, i) => (
            <span
              key={i}
              className="font-mono text-[10px] text-muted bg-panel-2 border border-line rounded-xl px-1.5 py-px"
            >
              {c}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

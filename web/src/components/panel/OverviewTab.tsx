'use client';

import { useGraphStore } from '@/store/useGraphStore';

function Chip({ label, value, tone }: { label?: string; value: string; tone?: 'eco' | 'ok' | 'warn' }) {
  const toneCls =
    tone === 'ok'
      ? 'text-ok border-[#1d4a32]'
      : tone === 'warn'
        ? 'text-warn border-[#4a3a1e] bg-[#20180c]'
        : tone === 'eco'
          ? 'text-accent border-[#2b4a6f]'
          : 'text-ink border-line-2';
  return (
    <span className={`inline-flex items-center gap-1.5 px-2.5 py-[3px] rounded-full text-[11.5px] bg-panel-2 border ${toneCls}`}>
      {label && <b className="font-bold">{label}</b>}
      {value}
    </span>
  );
}

export default function OverviewTab() {
  const meta = useGraphStore((s) => s.runMeta);
  const stats = useGraphStore((s) => s.stats);

  return (
    <div>
      <div className="flex flex-wrap gap-1.5 mb-2">
        {meta && (
          <>
            <Chip label="ecosystem" value={meta.ecosystem} tone="eco" />
            <Chip
              label="fidelity"
              value={meta.fidelity_tier}
              tone={meta.fidelity_tier === 'lockfile-exact' ? 'ok' : 'warn'}
            />
            {meta.resolver && <Chip label="resolver" value={meta.resolver} />}
            {meta.fidelity_caveats.map((c) => (
              <Chip key={c} value={c} tone="warn" />
            ))}
          </>
        )}
      </div>

      <h3 className="mt-4 mb-1.5 text-[11px] uppercase tracking-wide text-muted">Graph</h3>
      {stats ? (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
          {(
            [
              ['nodes', String(stats.nodes), false],
              ['edges', String(stats.edges), false],
              ['SCCs', String(stats.scc_count), false],
              ['has cycle', stats.has_cycle ? 'yes' : 'no', stats.has_cycle],
              ['max depth', String(stats.max_depth), false],
              ['duplicate families', String(stats.duplicate_version_count), stats.duplicate_version_count > 0],
            ] as [string, string, boolean][]
          ).map(([k, v, flag]) => (
            <div key={k} className="contents">
              <dt className="text-muted">{k}</dt>
              <dd className={`text-right tabular-nums font-semibold ${flag ? 'text-warn' : ''}`}>{v}</dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className="text-muted text-sm">Pick a run.</p>
      )}

      <h3 className="mt-4 mb-1.5 text-[11px] uppercase tracking-wide text-muted">How to read it</h3>
      <ul className="list-disc pl-4 text-muted text-xs space-y-1">
        <li>Click a node to trace what it <b className="text-ink">depends on</b> and what <b className="text-ink">depends on it</b>.</li>
        <li><b className="text-ink">2D</b> is a layered graph; <b className="text-ink">3D</b> is a spatial topology.</li>
        <li>In 3D: <b className="text-ink">Layered</b> stacks by depth, <b className="text-ink">Fiber</b> by version, <b className="text-ink">Terrain</b> by a signal, <b className="text-ink">TDA</b> lights up a feature.</li>
      </ul>
    </div>
  );
}

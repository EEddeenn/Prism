'use client';

/* The shared hover/selection readout for BOTH graph views. It subscribes to the
 * store's `hovered` (and falls back to `sel`) so it re-renders on its own when
 * the cursor moves — the 2D SVG and the 3D WebGL scene never re-render on hover.
 * Surfaces the node's key metrics (blast radius, reach, degrees, dup, depth). */

import { useGraphStore } from '@/store/useGraphStore';
import { shortLabel } from '@/lib/palette';

function fmt(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(n >= 10000 ? 0 : 1)}k`;
  return String(Math.round(n));
}

export default function HoverCard() {
  const byId = useGraphStore((s) => s.index?.byId);
  const metrics = useGraphStore((s) => s.index?.metrics);
  const sel = useGraphStore((s) => s.sel);
  const hovered = useGraphStore((s) => s.hovered);

  const id = hovered ?? sel;
  if (id == null || !byId) return null;
  const n = byId.get(id);
  if (!n) return null;
  const m = metrics?.get(id);
  const isHover = hovered != null;

  return (
    <div className="absolute top-3.5 left-1/2 -translate-x-1/2 z-10 pointer-events-none">
      <div className="px-3 py-1.5 rounded-full border border-line bg-panel/90 backdrop-blur-sm shadow-panel flex items-center gap-2 text-mini font-mono whitespace-nowrap">
        <span className={isHover ? 'text-accent' : 'text-node-center'}>{isHover ? 'hover' : 'selected'}</span>
        <span className="text-ink">{shortLabel(n.purl)}</span>
        {m && (
          <span className="flex items-center gap-1.5 text-muted">
            <span title="blast radius — transitive dependents">blast {fmt(m.revReach)}</span>
            <span title="transitive dependencies">· reach {fmt(m.fwdReach)}</span>
            <span title="direct dependents · direct dependencies">· in {m.inDeg} · out {m.outDeg}</span>
            {m.famSize > 1 && (
              <span className="text-node-dup" title="versions of this package family">· ×{m.famSize}</span>
            )}
            <span title="depth from a root">· d{n.depth}</span>
          </span>
        )}
      </div>
    </div>
  );
}

'use client';

import { Html } from '@react-three/drei';
import { useGraphStore } from '@/store/useGraphStore';
import { shortLabel } from '@/lib/palette';
import { node3dStyle, type StyleCtx } from './nodeStyle';
import type { Layout3D } from '@/lib/layout3d';
import type { MetricStats } from '@/lib/graph';

const SMALL = 70;
const MAX_LABELS = 90;
const ZERO_STATS: MetricStats = {
  maxRevReach: 0, maxFwdReach: 0, maxInDeg: 0, maxOutDeg: 0, maxFamSize: 0, maxDepth: 0,
};

/* DOM labels billboarded over highlighted/selected nodes (and every node on a
 * small graph). Capped so a big highlight never floods the canvas with DOM. */
export default function SceneLabels({ layout }: { layout: Layout3D }) {
  const nodes = useGraphStore((s) => s.nodes);
  const sel = useGraphStore((s) => s.sel);
  const fwd = useGraphStore((s) => s.fwd);
  const impact = useGraphStore((s) => s.impact);
  const matches = useGraphStore((s) => s.matches);
  const dup = useGraphStore((s) => s.dupVersionIds);
  const index = useGraphStore((s) => s.index);
  const roots = index?.rootIds;
  const mode = useGraphStore((s) => s.mode3d);
  const feature = useGraphStore((s) => s.feature);

  const ctx: StyleCtx = {
    sel, fwd, impact, matches, dup,
    roots: roots ?? new Set(),
    peaks: new Set(layout.peaks ?? []),
    metrics: index?.metrics ?? new Map(),
    stats: index?.stats ?? ZERO_STATS,
    mode, feature, small: nodes.length <= SMALL,
  };

  const show: typeof nodes = [];
  for (const n of nodes) {
    if (show.length >= MAX_LABELS) break;
    if (node3dStyle(n, ctx).label && layout.pos.has(n.id)) show.push(n);
  }

  return (
    <>
      {show.map((n) => {
        const p = layout.pos.get(n.id)!;
        return (
          <Html key={n.id} position={[p.x, p.y + 9, p.z]} center style={{ pointerEvents: 'none' }}>
            <div className="px-1 text-micro font-mono text-ink whitespace-nowrap" style={{ textShadow: '0 0 3px #000, 0 0 3px #000' }}>
              {shortLabel(n.purl)}
            </div>
          </Html>
        );
      })}
    </>
  );
}

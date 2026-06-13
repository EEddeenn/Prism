'use client';

/* All edges as one LineSegments with per-vertex colours. Dimmed edges fade
 * toward the background (a stand-in for per-vertex alpha, which line materials
 * don't support); highlighted/feature/back edges read bright. */

import { useEffect, useMemo, useRef } from 'react';
import * as THREE from 'three';
import { useGraphStore } from '@/store/useGraphStore';
import { PALETTE, hexToRgb, mixHex } from '@/lib/palette';
import { EDGE_FLAG_BACK_EDGE } from '@/lib/types';
import type { Layout3D } from '@/lib/layout3d';

export default function Edges({ layout }: { layout: Layout3D }) {
  const edges = useGraphStore((s) => s.edges);
  const sel = useGraphStore((s) => s.sel);
  const fwd = useGraphStore((s) => s.fwd);
  const impact = useGraphStore((s) => s.impact);
  const mode = useGraphStore((s) => s.mode3d);
  const feature = useGraphStore((s) => s.feature);

  const geom = useMemo(() => new THREE.BufferGeometry(), []);
  const matRef = useRef<THREE.LineBasicMaterial>(null);

  useEffect(() => {
    const pos: number[] = [];
    const col: number[] = [];
    const hasSel = sel != null;
    const feat = mode === 'tda' && feature;
    for (const e of edges) {
      const a = layout.pos.get(e.source);
      const b = layout.pos.get(e.target);
      if (!a || !b) continue;
      let hex = PALETTE.edge;
      let fade = 0.4; // toward bg
      if (feat) {
        if (feature!.edgeKeys.has(`${e.source}>${e.target}`)) { hex = PALETTE.gen; fade = 0; }
        else fade = 0.86;
      } else if (hasSel) {
        if (fwd.has(e.source) && fwd.has(e.target)) { hex = PALETTE.fwd; fade = 0; }
        else if (impact.has(e.source) && impact.has(e.target)) { hex = PALETTE.impact; fade = 0; }
        else fade = 0.86;
      } else if ((e.flags & EDGE_FLAG_BACK_EDGE) !== 0) {
        hex = PALETTE.impact; fade = 0.25;
      }
      const [r, g, bl] = hexToRgb(mixHex(hex, PALETTE.bg, fade));
      pos.push(a.x, a.y, a.z, b.x, b.y, b.z);
      col.push(r / 255, g / 255, bl / 255, r / 255, g / 255, bl / 255);
    }
    geom.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
    geom.setAttribute('color', new THREE.Float32BufferAttribute(col, 3));
    geom.computeBoundingSphere();
  }, [edges, layout, sel, fwd, impact, mode, feature, geom]);

  useEffect(() => () => geom.dispose(), [geom]);

  return (
    <lineSegments geometry={geom}>
      <lineBasicMaterial ref={matRef} vertexColors transparent opacity={0.85} />
    </lineSegments>
  );
}

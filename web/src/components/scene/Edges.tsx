'use client';

/* Edges in two passes:
 *   - bulk: every edge as one cheap LineSegments with per-vertex colours, faded
 *     toward the background (a stand-in for per-vertex alpha, which line
 *     materials don't support).
 *   - lit:  the highlighted subset (forward cone / impact cone / generator
 *     support / cycle back-edges) as a real-width drei <Line> drawn on top, so
 *     the traced path reads as a bright ribbon instead of a lost 1px hairline.
 *
 * Fiber mode draws NO edges: between vertical version-stacks they'd be the
 * chaotic hairball the layout is designed to avoid (the fibers ARE the structure). */

import { useEffect, useMemo, useRef } from 'react';
import * as THREE from 'three';
import { Line } from '@react-three/drei';
import { useGraphStore } from '@/store/useGraphStore';
import { PALETTE, hexToRgb, mixHex } from '@/lib/palette';
import { edgeVisible, isBackEdge, isFlagged, kindColor } from '@/lib/edges';
import type { Layout3D } from '@/lib/layout3d';

type Rgb = [number, number, number];
const rgb01 = (hex: string): Rgb => {
  const [r, g, b] = hexToRgb(hex);
  return [r / 255, g / 255, b / 255];
};

export default function Edges({ layout }: { layout: Layout3D }) {
  const edges = useGraphStore((s) => s.edges);
  const run = useGraphStore((s) => s.run);
  const sel = useGraphStore((s) => s.sel);
  const fwd = useGraphStore((s) => s.fwd);
  const impact = useGraphStore((s) => s.impact);
  const mode = useGraphStore((s) => s.mode3d);
  const feature = useGraphStore((s) => s.feature);
  const edgeFilter = useGraphStore((s) => s.edgeFilter);

  const geom = useMemo(() => new THREE.BufferGeometry(), []);
  const matRef = useRef<THREE.LineBasicMaterial>(null);

  // Classify every edge into the dim bulk or the bright lit set in one pass.
  const lit = useMemo(() => {
    const litPts: number[][] = [];
    const litCol: Rgb[] = [];
    const pos: number[] = [];
    const col: number[] = [];

    // Fiber mode: no edges (see header).
    if (mode === 'fiber') {
      geom.setAttribute('position', new THREE.Float32BufferAttribute([], 3));
      geom.setAttribute('color', new THREE.Float32BufferAttribute([], 3));
      geom.computeBoundingSphere();
      return { points: litPts, colors: litCol };
    }

    const hasSel = sel != null;
    const feat = mode === 'tda' && feature;
    for (const e of edges) {
      const a = layout.pos.get(e.source);
      const b = layout.pos.get(e.target);
      if (!a || !b) continue;

      // litHex != null => promote this edge to the bright ribbon pass.
      let litHex: string | null = null;
      let hex = PALETTE.edge;
      let fade = 0.5; // toward bg

      if (!edgeVisible(e, edgeFilter)) {
        hex = PALETTE.edge; fade = 0.93; // filtered out by the edge lens
      } else if (feat) {
        if (feature!.edgeKeys.has(`${e.source}>${e.target}`))
          litHex = feature!.dim === 2 ? PALETTE.void : PALETTE.gen;
        else fade = 0.9;
      } else if (hasSel) {
        if (fwd.has(e.source) && fwd.has(e.target)) litHex = PALETTE.fwd;
        else if (impact.has(e.source) && impact.has(e.target)) litHex = PALETTE.impact;
        else fade = 0.9;
      } else if (isBackEdge(e)) {
        // Neutral state: a back-edge rises across depth layers — the visual
        // signature of a dependency cycle. Light it so layered mode reads as a
        // cycle detector.
        litHex = PALETTE.impact;
      } else {
        // Smell-flagged edges stay the most visible thing at rest; plain edges
        // colour by kind, a touch brighter than before so kind reads at a glance.
        hex = kindColor(e.kind); fade = isFlagged(e) ? 0.12 : 0.5;
      }

      if (litHex) {
        litPts.push([a.x, a.y, a.z], [b.x, b.y, b.z]);
        const c = rgb01(litHex);
        litCol.push(c, c);
      } else {
        // Direction cue with zero extra geometry: fade the SOURCE vertex harder
        // toward the background and keep the TARGET vertex bright, so each edge
        // reads as a gradient flowing toward the dependency (matches the 2D
        // arrowhead, which points source → target).
        const src = hexToRgb(mixHex(hex, PALETTE.bg, Math.min(0.95, fade + 0.32)));
        const tgt = hexToRgb(mixHex(hex, PALETTE.bg, fade));
        pos.push(a.x, a.y, a.z, b.x, b.y, b.z);
        col.push(src[0] / 255, src[1] / 255, src[2] / 255, tgt[0] / 255, tgt[1] / 255, tgt[2] / 255);
      }
    }

    geom.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
    geom.setAttribute('color', new THREE.Float32BufferAttribute(col, 3));
    geom.computeBoundingSphere();
    return { points: litPts, colors: litCol };
  }, [edges, layout, sel, fwd, impact, mode, feature, edgeFilter, geom]);

  useEffect(() => () => geom.dispose(), [geom]);

  return (
    <>
      <lineSegments geometry={geom}>
        <lineBasicMaterial ref={matRef} vertexColors transparent opacity={0.85} />
      </lineSegments>
      {lit.points.length > 0 && (
        <Line
          // Remount when Line2's buffer size MUST change (run swap, or the lit
          // set grew/shrank). The previous key was `lit.points.length` alone,
          // which collided across two runs that happen to share a lit count
          // and reused the previous run's GPU buffer in place. Including the
          // run id eliminates the collision; a within-run selection change
          // that doesn't alter the buffer size now updates in place without
          // a Line2 remount.
          key={`lit-edges-${run ?? 'none'}-${lit.points.length}`}
          points={lit.points as [number, number, number][]}
          vertexColors={lit.colors}
          segments
          lineWidth={2.2}
          transparent
          opacity={0.95}
          toneMapped={false}
        />
      )}
    </>
  );
}

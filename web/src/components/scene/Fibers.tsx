'use client';

import { useEffect, useMemo } from 'react';
import * as THREE from 'three';
import { Html } from '@react-three/drei';
import { PALETTE } from '@/lib/palette';
import type { Fiber } from '@/lib/layout3d';

/* Vertical "fibers" — one per package family. Duplicated families (more than one
 * version) read bright pink and get a label; single-version families are faint
 * stubs. All stems share one LineSegments for performance. */
export default function Fibers({ fibers }: { fibers: Fiber[] }) {
  const geom = useMemo(() => new THREE.BufferGeometry(), []);

  useEffect(() => {
    const pos: number[] = [];
    const col: number[] = [];
    for (const f of fibers) {
      const c = f.dup ? PALETTE.dup : PALETTE.muted;
      const r = parseInt(c.slice(1, 3), 16) / 255;
      const g = parseInt(c.slice(3, 5), 16) / 255;
      const b = parseInt(c.slice(5, 7), 16) / 255;
      const a = f.dup ? 1 : 0.4; // fade stubs by darkening toward bg
      pos.push(f.x, 0, f.z, f.x, f.top + 18, f.z);
      col.push(r * a, g * a, b * a, r * a, g * a, b * a);
    }
    geom.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3));
    geom.setAttribute('color', new THREE.Float32BufferAttribute(col, 3));
    geom.computeBoundingSphere();
  }, [fibers, geom]);

  useEffect(() => () => geom.dispose(), [geom]);

  // Label the tallest duplicated families (cap to avoid clutter).
  const labels = useMemo(
    () => fibers.filter((f) => f.dup).sort((a, b) => b.top - a.top).slice(0, 40),
    [fibers],
  );

  return (
    <group>
      <lineSegments geometry={geom}>
        <lineBasicMaterial vertexColors transparent opacity={0.9} />
      </lineSegments>
      {labels.map((f, i) => (
        <Html
          key={i}
          position={[f.x, f.top + 30, f.z]}
          center
          // Same z-index clamp as SceneLabels — keeps fiber labels above
          // the WebGL canvas but below every HUD overlay at z-10.
          zIndexRange={[8, 0]}
          style={{ pointerEvents: 'none' }}
        >
          <div className="px-1 text-[10px] font-mono text-node-dup whitespace-nowrap" style={{ textShadow: '0 0 4px #000' }}>
            {f.label} ·{f.count}
          </div>
        </Html>
      ))}
    </group>
  );
}

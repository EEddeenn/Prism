'use client';

import { useEffect, useMemo } from 'react';
import * as THREE from 'three';
import { Html } from '@react-three/drei';
import { rampHex } from '@/lib/palette';
import type { LayerPlane } from '@/lib/layout3d';

/* Translucent horizontal planes, one per BFS depth, tinted along the elevation
 * ramp (roots cool → deep layers warm) so the stack reads as distinct strata
 * instead of one uniform fog. The tinted label border ties each plane to its
 * depth colour. */
export default function LayerPlanes({ planes }: { planes: LayerPlane[] }) {
  const maxD = planes.reduce((m, p) => Math.max(m, p.depth), 1);

  // Share ONE unit-square geometry across every plane and scale per-instance.
  // Inline `<planeGeometry>` inside a `.map()` allocates a fresh buffer per
  // plane every render, and JSX never disposes the orphaned ones on mode/run
  // switch — that's the GPU leak. A single shared geometry has no such failure
  // mode and runs strictly faster.
  const sharedGeom = useMemo(() => new THREE.PlaneGeometry(1, 1), []);
  useEffect(() => () => sharedGeom.dispose(), [sharedGeom]);

  return (
    <group>
      {planes.map((pl) => {
        const R = Math.max(pl.radius, 92);
        const tint = rampHex(pl.depth / maxD);
        return (
          <group key={pl.depth}>
            <mesh
              geometry={sharedGeom}
              rotation={[-Math.PI / 2, 0, 0]}
              position={[0, pl.y, 0]}
              scale={[2 * R, 2 * R, 1]}
            >
              <meshBasicMaterial
                color={tint}
                transparent
                opacity={0.07}
                side={THREE.DoubleSide}
                depthWrite={false}
              />
            </mesh>
            {/* Same z-index clamp as SceneLabels — without it drei's default
                [16777271, 0] range floats the depth tag OVER the HUD
                overlays (the TDA hint, FeatureCard, etc.). */}
            <Html
              position={[-R, pl.y, -R]}
              center
              zIndexRange={[8, 0]}
              style={{ pointerEvents: 'none' }}
            >
              <div
                className="px-1.5 py-0.5 rounded bg-panel-2/80 border text-[10px] text-muted whitespace-nowrap font-mono"
                style={{ borderColor: tint }}
              >
                {pl.depth === 0 ? 'roots' : `d${pl.depth}`} · {pl.count}
              </div>
            </Html>
          </group>
        );
      })}
    </group>
  );
}

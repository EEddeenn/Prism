'use client';

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
  return (
    <group>
      {planes.map((pl) => {
        const R = Math.max(pl.radius, 92);
        const tint = rampHex(pl.depth / maxD);
        return (
          <group key={pl.depth}>
            <mesh rotation={[-Math.PI / 2, 0, 0]} position={[0, pl.y, 0]}>
              <planeGeometry args={[2 * R, 2 * R]} />
              <meshBasicMaterial
                color={tint}
                transparent
                opacity={0.07}
                side={THREE.DoubleSide}
                depthWrite={false}
              />
            </mesh>
            <Html position={[-R, pl.y, -R]} center style={{ pointerEvents: 'none' }}>
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

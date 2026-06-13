'use client';

import * as THREE from 'three';
import { Html } from '@react-three/drei';
import { PALETTE } from '@/lib/palette';
import type { LayerPlane } from '@/lib/layout3d';

/* Translucent horizontal planes, one per BFS depth, with a depth/count label —
 * the readable "layers" of the layered topology view. */
export default function LayerPlanes({ planes }: { planes: LayerPlane[] }) {
  return (
    <group>
      {planes.map((pl) => {
        const R = Math.max(pl.radius, 92);
        return (
          <group key={pl.depth}>
            <mesh rotation={[-Math.PI / 2, 0, 0]} position={[0, pl.y, 0]}>
              <planeGeometry args={[2 * R, 2 * R]} />
              <meshBasicMaterial
                color={PALETTE.accent}
                transparent
                opacity={0.05}
                side={THREE.DoubleSide}
                depthWrite={false}
              />
            </mesh>
            <Html position={[-R, pl.y, -R]} center style={{ pointerEvents: 'none' }}>
              <div className="px-1.5 py-0.5 rounded bg-panel-2/80 border border-line text-[10px] text-muted whitespace-nowrap font-mono">
                {pl.depth === 0 ? 'roots' : `d${pl.depth}`} · {pl.count}
              </div>
            </Html>
          </group>
        );
      })}
    </group>
  );
}

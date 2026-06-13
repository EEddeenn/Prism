'use client';

import { useEffect, useMemo } from 'react';
import * as THREE from 'three';

/* The "lit up" overlay: for each highlighted node (selection, search, generator
 * support) draw a full-bright unlit core orb plus a soft additive halo. This
 * gives a bloom-like glow with no postprocessing pass — the highlight set is
 * bounded (it mirrors the labelled set), so plain meshes are cheap here.
 *
 * The overlay is non-interactive (raycast disabled): hover/click are owned by
 * the base instanced layer underneath, so the glow never steals pointer events. */
export interface Glow {
  id: number;
  pos: [number, number, number];
  color: string;
  scale: number;
}

const NONE = () => {};

export default function GlowNodes({ glows }: { glows: Glow[] }) {
  const geom = useMemo(() => new THREE.SphereGeometry(1, 18, 18), []);

  // One core + one halo material per DISTINCT colour. `glows` is a fresh array
  // every render (and Nodes re-renders on every hover crossing), so key the
  // material build on the stable set of colours — not the array identity —
  // to avoid recreating/disposing materials on every pointer move.
  const colorKey = useMemo(
    () => Array.from(new Set(glows.map((g) => g.color))).sort().join('|'),
    [glows],
  );
  const mats = useMemo(() => {
    const core = new Map<string, THREE.Material>();
    const halo = new Map<string, THREE.Material>();
    for (const color of colorKey ? colorKey.split('|') : []) {
      core.set(color, new THREE.MeshBasicMaterial({ color, toneMapped: false }));
      halo.set(
        color,
        new THREE.MeshBasicMaterial({
          color,
          transparent: true,
          opacity: 0.16,
          blending: THREE.AdditiveBlending,
          depthWrite: false,
          toneMapped: false,
        }),
      );
    }
    return { core, halo };
  }, [colorKey]);

  // Dispose the previous materials when the highlight set's colours change, and
  // the shared geometry on unmount.
  useEffect(() => () => {
    mats.core.forEach((m) => m.dispose());
    mats.halo.forEach((m) => m.dispose());
  }, [mats]);
  useEffect(() => () => geom.dispose(), [geom]);

  if (!glows.length) return null;

  return (
    <group>
      {glows.map((g) => (
        <group key={g.id} position={g.pos}>
          <mesh geometry={geom} material={mats.core.get(g.color)} scale={g.scale * 0.85} raycast={NONE} />
          <mesh geometry={geom} material={mats.halo.get(g.color)} scale={g.scale * 2.3} raycast={NONE} />
        </group>
      ))}
    </group>
  );
}

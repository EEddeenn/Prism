'use client';

import { useEffect, useMemo } from 'react';
import * as THREE from 'three';
import { rampRgb } from '@/lib/palette';
import type { TerrainField } from '@/lib/layout3d';

/* The dependency terrain: a heightfield mesh whose elevation is the chosen
 * analytical signal, coloured by a low→high ramp. The scene's directional light
 * sculpts the relief (normals are computed below); a faint baked slope-shade in
 * the vertex colours deepens valleys so ridges read even in flat light. */
export default function TerrainMesh({ terrain }: { terrain: TerrainField }) {
  const geom = useMemo(() => {
    const { H, G, cell, size, maxHeight } = terrain;
    const g = new THREE.BufferGeometry();
    const verts: number[] = [];
    const colors: number[] = [];
    const idx: number[] = [];
    for (let j = 0; j < G; j++) {
      for (let i = 0; i < G; i++) {
        const x = -size + i * cell;
        const z = -size + j * cell;
        const y = H[j * G + i];
        verts.push(x, y, z);
        const t = y / (maxHeight || 1);
        const hx = (i < G - 1 ? H[j * G + i + 1] : y) - (i > 0 ? H[j * G + i - 1] : y);
        const hz = (j < G - 1 ? H[(j + 1) * G + i] : y) - (j > 0 ? H[(j - 1) * G + i] : y);
        const slope = Math.min(1, Math.hypot(hx, hz) / 45);
        // Lighter baked shade now that real lighting does most of the relief.
        const shade = 0.85 + 0.15 * (1 - slope);
        const [r, gg, b] = rampRgb(t);
        colors.push((r / 255) * shade, (gg / 255) * shade, (b / 255) * shade);
      }
    }
    for (let j = 0; j < G - 1; j++) {
      for (let i = 0; i < G - 1; i++) {
        const a = j * G + i;
        const b = j * G + i + 1;
        const c = (j + 1) * G + i + 1;
        const d = (j + 1) * G + i;
        idx.push(a, b, d, b, c, d);
      }
    }
    g.setIndex(idx);
    g.setAttribute('position', new THREE.Float32BufferAttribute(verts, 3));
    g.setAttribute('color', new THREE.Float32BufferAttribute(colors, 3));
    g.computeVertexNormals();
    return g;
  }, [terrain]);

  useEffect(() => () => geom.dispose(), [geom]);

  return (
    <mesh geometry={geom}>
      <meshStandardMaterial vertexColors side={THREE.DoubleSide} roughness={0.92} metalness={0} />
    </mesh>
  );
}

'use client';

import { useEffect, useMemo, useRef } from 'react';
import { Instances, Instance } from '@react-three/drei';
import type { ThreeEvent } from '@react-three/fiber';
import { useGraphStore } from '@/store/useGraphStore';
import { node3dStyle, type StyleCtx } from './nodeStyle';
import GlowNodes, { type Glow } from './GlowNodes';
import type { Layout3D } from '@/lib/layout3d';
import type { GraphNode } from '@/lib/types';

const SMALL = 70;

export default function Nodes({ layout }: { layout: Layout3D }) {
  const nodes = useGraphStore((s) => s.nodes);
  const sel = useGraphStore((s) => s.sel);
  const fwd = useGraphStore((s) => s.fwd);
  const impact = useGraphStore((s) => s.impact);
  const matches = useGraphStore((s) => s.matches);
  const dup = useGraphStore((s) => s.dupVersionIds);
  const roots = useGraphStore((s) => s.index?.rootIds);
  const mode = useGraphStore((s) => s.mode3d);
  const feature = useGraphStore((s) => s.feature);
  const selectNode = useGraphStore((s) => s.selectNode);
  const setHovered = useGraphStore((s) => s.setHovered);

  // Only nodes that actually have a position render — keep them in one ordered
  // array so a raycast's `instanceId` maps straight back to the node.
  const placed = useMemo<GraphNode[]>(
    () => nodes.filter((n) => layout.pos.has(n.id)),
    [nodes, layout],
  );
  const lastHover = useRef<number | null>(null);

  // R3F does NOT fire onPointerOut when the mesh unmounts (view switch, or the
  // key-remount on a node-count change), so clear the global cursor + hover
  // here or the 'pointer' cursor sticks across the rest of the app.
  useEffect(
    () => () => {
      document.body.style.cursor = '';
      setHovered(null);
    },
    [setHovered],
  );

  const ctx: StyleCtx = {
    sel, fwd, impact, matches, dup,
    roots: roots ?? new Set(),
    peaks: new Set(layout.peaks ?? []),
    mode, feature, small: nodes.length <= SMALL,
  };

  // One pass: base instances for every node + the bright glow set on top.
  const glows: Glow[] = [];
  for (const n of placed) {
    const st = node3dStyle(n, ctx);
    if (st.glow) {
      const p = layout.pos.get(n.id)!;
      glows.push({ id: n.id, pos: [p.x, p.y, p.z], color: st.color, scale: st.scale });
    }
  }

  if (!placed.length) return null;

  // The whole node cloud is ONE merged InstancedMesh, so onPointerOver fires
  // only on first entry; use onPointerMove + a ref to report each instance the
  // cursor crosses without a re-render on every frame.
  const move = (e: ThreeEvent<PointerEvent>) => {
    e.stopPropagation();
    const id = e.instanceId != null ? placed[e.instanceId]?.id ?? null : null;
    if (id !== lastHover.current) {
      lastHover.current = id;
      setHovered(id);
      document.body.style.cursor = id != null ? 'pointer' : '';
    }
  };
  const leave = () => {
    if (lastHover.current !== null) {
      lastHover.current = null;
      setHovered(null);
      document.body.style.cursor = '';
    }
  };

  return (
    <>
      {/* `key` forces a fresh instanced-buffer allocation when the node count
          changes — drei sizes its instanceMatrix/instanceColor arrays once at
          mount, so a larger run would otherwise overrun buffers sized to the
          first run. */}
      <Instances
        key={placed.length}
        limit={placed.length}
        range={placed.length}
        castShadow={false}
        onPointerMove={move}
        onPointerOut={leave}
      >
        <sphereGeometry args={[1, 16, 16]} />
        <meshStandardMaterial roughness={0.5} metalness={0.1} />
        {placed.map((n) => {
          const p = layout.pos.get(n.id)!;
          const st = node3dStyle(n, ctx);
          return (
            <Instance
              key={n.id}
              position={[p.x, p.y, p.z]}
              color={st.color}
              scale={st.scale}
              onClick={(e) => {
                e.stopPropagation();
                selectNode(n.id);
              }}
            />
          );
        })}
      </Instances>
      <GlowNodes glows={glows} />
    </>
  );
}

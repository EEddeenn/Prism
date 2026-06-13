'use client';

import { Instances, Instance } from '@react-three/drei';
import { useGraphStore } from '@/store/useGraphStore';
import { node3dStyle, type StyleCtx } from './nodeStyle';
import type { Layout3D } from '@/lib/layout3d';

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

  if (!nodes.length) return null;
  const ctx: StyleCtx = {
    sel, fwd, impact, matches, dup,
    roots: roots ?? new Set(),
    mode, feature, small: nodes.length <= SMALL,
  };

  return (
    // `key` forces a fresh instanced-buffer allocation when the node count
    // changes — drei sizes its instanceMatrix/instanceColor arrays once at mount,
    // so a larger run would otherwise overrun buffers sized to the first run.
    <Instances key={nodes.length} limit={nodes.length} range={nodes.length} castShadow={false}>
      <sphereGeometry args={[1, 16, 16]} />
      <meshStandardMaterial roughness={0.45} metalness={0.12} />
      {nodes.map((n) => {
        const p = layout.pos.get(n.id);
        if (!p) return null;
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
  );
}

'use client';

import { useMemo, useState } from 'react';
import { Canvas } from '@react-three/fiber';
import { OrbitControls } from '@react-three/drei';
import { useGraphStore } from '@/store/useGraphStore';
import { buildLayout3D, type Layout3D } from '@/lib/layout3d';
import { PALETTE } from '@/lib/palette';
import Nodes from './scene/Nodes';
import Edges from './scene/Edges';
import LayerPlanes from './scene/LayerPlanes';
import Fibers from './scene/Fibers';
import TerrainMesh from './scene/TerrainMesh';
import SceneLabels from './scene/SceneLabels';
import CameraRig from './scene/CameraRig';
import Controls3D from './Controls3D';

function SceneContent({ fitNonce, spin }: { fitNonce: number; spin: boolean }) {
  const nodes = useGraphStore((s) => s.nodes);
  const index = useGraphStore((s) => s.index);
  const pos2d = useGraphStore((s) => s.pos2d);
  const mode = useGraphStore((s) => s.mode3d);
  const signal = useGraphStore((s) => s.signal);

  const layout: Layout3D = useMemo(
    () => (index ? buildLayout3D(mode, nodes, index, pos2d, signal) : { pos: new Map() }),
    [mode, nodes, index, pos2d, signal],
  );

  return (
    <>
      <color attach="background" args={[PALETTE.bg]} />
      <ambientLight intensity={0.75} />
      <directionalLight position={[300, 500, 200]} intensity={0.7} />
      <CameraRig layout={layout} fitNonce={fitNonce} />
      <OrbitControls makeDefault enableDamping dampingFactor={0.12} autoRotate={spin} autoRotateSpeed={0.5} />
      <Edges layout={layout} />
      <Nodes layout={layout} />
      {layout.planes && <LayerPlanes planes={layout.planes} />}
      {layout.fibers && <Fibers fibers={layout.fibers} />}
      {layout.terrain && <TerrainMesh terrain={layout.terrain} />}
      <SceneLabels layout={layout} />
    </>
  );
}

export default function Graph3D() {
  const mode = useGraphStore((s) => s.mode3d);
  const setMode3d = useGraphStore((s) => s.setMode3d);
  const signal = useGraphStore((s) => s.signal);
  const setSignal = useGraphStore((s) => s.setSignal);
  const [spin, setSpin] = useState(false);
  const [fitNonce, setFitNonce] = useState(0);

  return (
    <div className="absolute inset-0">
      <Canvas
        camera={{ fov: 52, position: [400, 320, 400], near: 1, far: 20000 }}
        gl={{ antialias: true }}
        dpr={[1, 2]}
      >
        <SceneContent fitNonce={fitNonce} spin={spin} />
      </Canvas>
      <Controls3D
        mode={mode}
        setMode={setMode3d}
        signal={signal}
        setSignal={setSignal}
        spin={spin}
        toggleSpin={() => setSpin((v) => !v)}
        onReset={() => setFitNonce((n) => n + 1)}
      />
    </div>
  );
}

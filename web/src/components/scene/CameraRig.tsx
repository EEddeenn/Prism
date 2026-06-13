'use client';

import { useEffect } from 'react';
import { useThree } from '@react-three/fiber';
import * as THREE from 'three';
import { useGraphStore } from '@/store/useGraphStore';
import type { Layout3D } from '@/lib/layout3d';

/* Fits the camera to the current layout (and on an explicit reset), and recenters
 * the orbit target when something requests focus (search, duplicate highlight,
 * feature selection). */
export default function CameraRig({ layout, fitNonce }: { layout: Layout3D; fitNonce: number }) {
  const camera = useThree((s) => s.camera) as THREE.PerspectiveCamera;
  // drei's OrbitControls (makeDefault) registers itself here.
  const controls = useThree((s) => s.controls) as unknown as
    | { target: THREE.Vector3; update: () => void }
    | null;
  const focusNonce = useGraphStore((s) => s.focusNonce);
  const focusTarget = useGraphStore((s) => s.focusTarget);

  // Fit to the whole layout whenever it changes or a reset is requested.
  useEffect(() => {
    const box = new THREE.Box3();
    let any = false;
    layout.pos.forEach((p) => {
      box.expandByPoint(new THREE.Vector3(p.x, p.y, p.z));
      any = true;
    });
    if (!any) return;
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3());
    const radius = Math.max(size.x, size.y, size.z, 120) / 2;
    const dist = (radius / Math.tan((camera.fov * Math.PI) / 180 / 2)) * 1.7;
    const dir = new THREE.Vector3(0.7, 0.55, 1).normalize();
    camera.position.copy(center).addScaledVector(dir, dist);
    camera.near = Math.max(0.1, dist / 200);
    camera.far = dist * 12;
    camera.updateProjectionMatrix();
    if (controls) {
      controls.target.copy(center);
      controls.update();
    } else {
      camera.lookAt(center);
    }
    // `controls` is included so the fit re-runs once OrbitControls registers
    // itself (it mounts a tick after this rig), keeping the orbit target aligned
    // with where the camera looks.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layout, fitNonce, controls]);

  // Recenter the orbit target on a focus request.
  useEffect(() => {
    if (focusTarget == null) return;
    const p = layout.pos.get(focusTarget);
    if (!p || !controls) return;
    controls.target.set(p.x, p.y, p.z);
    controls.update();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusNonce, controls]);

  return null;
}

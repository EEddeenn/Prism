'use client';

import { useEffect, useRef } from 'react';
import { useFrame, useThree } from '@react-three/fiber';
import * as THREE from 'three';
import { useGraphStore } from '@/store/useGraphStore';
import type { Layout3D } from '@/lib/layout3d';

/* Fits the camera to the current layout (and on an explicit reset), and frames
 * the selection's neighbourhood when something requests focus (search, duplicate
 * highlight, feature selection). Transitions are lerped over ~0.5s (unless the
 * user prefers reduced motion). */

const ANIM_MS = 500;
const EPS = 0.5; // world units; below this we snap + stop.
const FIT_DIR = new THREE.Vector3(0.7, 0.55, 1).normalize();

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

function smoothstep(t: number): number {
  const c = t < 0 ? 0 : t > 1 ? 1 : t;
  return c * c * (3 - 2 * c);
}

export default function CameraRig({
  layout,
  fitKey,
  fitNonce,
}: {
  layout: Layout3D;
  /** A coarse signature (mode + node count) that changes only when a refit is
   *  actually warranted — NOT on within-mode signal tweaks, which recompute the
   *  layout object but should leave the camera where the user put it. */
  fitKey: string;
  fitNonce: number;
}) {
  const camera = useThree((s) => s.camera) as THREE.PerspectiveCamera;
  // drei's OrbitControls (makeDefault) registers itself here.
  const controls = useThree((s) => s.controls) as unknown as
    | { target: THREE.Vector3; update: () => void }
    | null;
  const focusNonce = useGraphStore((s) => s.focusNonce);
  const focusTarget = useGraphStore((s) => s.focusTarget);
  // Subscribed so a focus can UPGRADE to the full neighbourhood box once
  // selectNode's async fwd/impact resolve (they are empty at requestFocus time).
  const fwd = useGraphStore((s) => s.fwd);
  const impact = useGraphStore((s) => s.impact);
  const pendingFocus = useRef<number | null>(null);

  // Animation state, held in refs so useFrame can drive it without re-renders.
  const anim = useRef<{
    active: boolean;
    start: number; // performance.now() when the tween began
    fromPos: THREE.Vector3;
    fromTarget: THREE.Vector3;
    goalPos: THREE.Vector3;
    goalTarget: THREE.Vector3;
    near: number;
    far: number;
  }>({
    active: false,
    start: 0,
    fromPos: new THREE.Vector3(),
    fromTarget: new THREE.Vector3(),
    goalPos: new THREE.Vector3(),
    goalTarget: new THREE.Vector3(),
    near: camera.near,
    far: camera.far,
  });

  // Apply a computed goal: either snap instantly (reduced motion / no controls)
  // or kick off a lerp that useFrame advances.
  function applyGoal(
    goalPos: THREE.Vector3,
    goalTarget: THREE.Vector3,
    near: number,
    far: number,
  ) {
    if (prefersReducedMotion()) {
      camera.position.copy(goalPos);
      camera.near = near;
      camera.far = far;
      camera.updateProjectionMatrix();
      if (controls) {
        controls.target.copy(goalTarget);
        controls.update();
      } else {
        camera.lookAt(goalTarget);
      }
      anim.current.active = false;
      return;
    }
    const a = anim.current;
    a.fromPos.copy(camera.position);
    a.fromTarget.copy(controls ? controls.target : goalTarget);
    a.goalPos.copy(goalPos);
    a.goalTarget.copy(goalTarget);
    a.near = near;
    a.far = far;
    a.start = performance.now();
    a.active = true;
  }

  // Frame a box (its center + radius) from the canonical fit direction.
  function frameBox(center: THREE.Vector3, radius: number) {
    const r = Math.max(radius, 60);
    const dist = (r / Math.tan((camera.fov * Math.PI) / 180 / 2)) * 1.7;
    const goalPos = center.clone().addScaledVector(FIT_DIR, dist);
    const near = Math.max(0.1, dist / 200);
    const far = dist * 12;
    applyGoal(goalPos, center.clone(), near, far);
  }

  useFrame(() => {
    const a = anim.current;
    if (!a.active) return;
    const t = smoothstep((performance.now() - a.start) / ANIM_MS);
    camera.position.lerpVectors(a.fromPos, a.goalPos, t);
    if (controls) {
      controls.target.lerpVectors(a.fromTarget, a.goalTarget, t);
    }
    // Ease projection bounds toward the goal so deep zooms don't clip mid-flight.
    camera.near = THREE.MathUtils.lerp(camera.near, a.near, t);
    camera.far = THREE.MathUtils.lerp(camera.far, a.far, t);
    camera.updateProjectionMatrix();
    if (controls) controls.update();
    else camera.lookAt(a.goalTarget);

    const done =
      t >= 1 ||
      (camera.position.distanceTo(a.goalPos) < EPS &&
        (!controls || controls.target.distanceTo(a.goalTarget) < EPS));
    if (done) {
      camera.position.copy(a.goalPos);
      camera.near = a.near;
      camera.far = a.far;
      camera.updateProjectionMatrix();
      if (controls) {
        controls.target.copy(a.goalTarget);
        controls.update();
      } else {
        camera.lookAt(a.goalTarget);
      }
      a.active = false;
    }
  });

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
    frameBox(center, radius);
    // `controls` is included so the fit re-runs once OrbitControls registers
    // itself (it mounts a tick after this rig), keeping the orbit target aligned
    // with where the camera looks. `layout` is intentionally NOT a dep — refit
    // only on a mode/size change (fitKey) or an explicit reset (fitNonce).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fitKey, fitNonce, controls]);

  // Frame a focus target: the bounding box of {target} ∪ its neighbourhood
  // (fwd ∪ impact) when those sets already describe `target`, else just recenter
  // the orbit target on the single node (keeping the camera where the user put it).
  function frameFocus(target: number) {
    if (!controls) return;
    const ids = new Set<number>([target]);
    if (fwd.has(target) || impact.has(target)) {
      fwd.forEach((id) => ids.add(id));
      impact.forEach((id) => ids.add(id));
    }
    const box = new THREE.Box3();
    let any = false;
    ids.forEach((id) => {
      const p = layout.pos.get(id);
      if (p) {
        box.expandByPoint(new THREE.Vector3(p.x, p.y, p.z));
        any = true;
      }
    });
    if (!any) return;
    const center = box.getCenter(new THREE.Vector3());
    if (ids.size <= 1) {
      applyGoal(camera.position.clone(), center, camera.near, camera.far);
    } else {
      const size = box.getSize(new THREE.Vector3());
      frameBox(center, Math.max(size.x, size.y, size.z) / 2);
    }
  }

  // On a focus request, frame what's available NOW (a single node, or an already-
  // resolved neighbourhood from the synchronous highlight path), and remember the
  // target so we upgrade to the full blast radius when its async fwd/impact land.
  useEffect(() => {
    if (focusTarget == null || !controls) return;
    pendingFocus.current = focusTarget;
    frameFocus(focusTarget);
    if (fwd.has(focusTarget) || impact.has(focusTarget)) pendingFocus.current = null;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusNonce, controls]);

  // When the neighbourhood resolves for the still-pending focus, reframe to it.
  // (Plain selections never set pendingFocus, so they don't move the camera.)
  useEffect(() => {
    const t = pendingFocus.current;
    if (t != null && (fwd.has(t) || impact.has(t))) {
      frameFocus(t);
      pendingFocus.current = null;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fwd, impact]);

  return null;
}

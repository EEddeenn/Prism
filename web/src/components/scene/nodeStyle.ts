/* Shared visual role for a node in the 3D scene — the same selection semantics
 * as the 2D view, returned as a colour/scale/label decision so the instanced
 * mesh and the labels stay in lockstep. */

import { PALETTE, dimHex } from '@/lib/palette';
import type { GraphNode, Mode3D } from '@/lib/types';
import type { FeatureHL } from '@/store/useGraphStore';

export interface StyleCtx {
  sel: number | null;
  fwd: Set<number>;
  impact: Set<number>;
  matches: Set<number>;
  dup: Set<number>;
  roots: Set<number>;
  mode: Mode3D;
  feature: FeatureHL | null;
  small: boolean;
}

export interface NodeStyle {
  color: string;
  scale: number;
  dim: boolean;
  label: boolean;
}

const BASE = 6;

export function node3dStyle(n: GraphNode, c: StyleCtx): NodeStyle {
  let color = c.roots.has(n.id) ? PALETTE.root : PALETTE.node;
  if (c.dup.has(n.version_id)) color = PALETTE.dup;

  // TDA mode: the selected feature's generator support is the focus.
  if (c.mode === 'tda' && c.feature) {
    if (c.feature.nodeIds.has(n.id)) return { color: PALETTE.gen, scale: BASE * 1.5, dim: false, label: true };
    return { color: dimHex(color, 0.72), scale: BASE * 0.7, dim: true, label: false };
  }

  if (c.sel != null) {
    if (n.id === c.sel) return { color: PALETTE.center, scale: BASE * 1.5, dim: false, label: true };
    if (c.fwd.has(n.id)) return { color: PALETTE.fwd, scale: BASE, dim: false, label: true };
    if (c.impact.has(n.id)) return { color: PALETTE.impact, scale: BASE, dim: false, label: true };
    return { color: dimHex(color, 0.72), scale: BASE * 0.7, dim: true, label: false };
  }

  if (c.matches.has(n.id)) return { color: PALETTE.accent, scale: BASE * 1.15, dim: false, label: true };
  return { color, scale: BASE, dim: false, label: c.small };
}

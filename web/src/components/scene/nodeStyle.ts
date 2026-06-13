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
  peaks: Set<number>;
  mode: Mode3D;
  feature: FeatureHL | null;
  small: boolean;
}

export interface NodeStyle {
  color: string;
  scale: number;
  dim: boolean;
  label: boolean;
  /** Emit a bright second-layer orb + halo (the "lit up" set: selection, search,
   *  generator support, the selected node itself). */
  glow: boolean;
}

const BASE = 6;

export function node3dStyle(n: GraphNode, c: StyleCtx): NodeStyle {
  let color = c.roots.has(n.id) ? PALETTE.root : PALETTE.node;
  if (c.dup.has(n.version_id)) color = PALETTE.dup;

  // TDA mode: the selected feature's generator support is the focus.
  if (c.mode === 'tda' && c.feature) {
    if (c.feature.nodeIds.has(n.id)) return { color: PALETTE.gen, scale: BASE * 1.5, dim: false, label: true, glow: true };
    return { color: dimHex(color, 0.82), scale: BASE * 0.6, dim: true, label: false, glow: false };
  }

  if (c.sel != null) {
    if (n.id === c.sel) return { color: PALETTE.center, scale: BASE * 1.5, dim: false, label: true, glow: true };
    if (c.fwd.has(n.id)) return { color: PALETTE.fwd, scale: BASE, dim: false, label: true, glow: true };
    if (c.impact.has(n.id)) return { color: PALETTE.impact, scale: BASE, dim: false, label: true, glow: true };
    return { color: dimHex(color, 0.82), scale: BASE * 0.6, dim: true, label: false, glow: false };
  }

  if (c.matches.has(n.id)) return { color: PALETTE.accent, scale: BASE * 1.15, dim: false, label: true, glow: true };
  // Terrain peaks read as labeled markers even with nothing selected.
  if (c.peaks.has(n.id)) return { color, scale: BASE * 1.1, dim: false, label: true, glow: false };
  return { color, scale: BASE, dim: false, label: c.small, glow: false };
}

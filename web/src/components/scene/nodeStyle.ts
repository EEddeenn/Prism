/* Shared visual role for a node in the 3D scene — the same selection semantics
 * as the 2D view, returned as a colour/scale/label decision so the instanced
 * mesh and the labels stay in lockstep.
 *
 * Size and neutral colour both encode IMPORTANCE (blast radius); the selection /
 * search / TDA / hover roles layer their own colour + a size multiplier on top. */

import { PALETTE, dimHex } from '@/lib/palette';
import { importanceColor, importanceT, scale3D } from '@/lib/encoding';
import type { MetricStats, NodeMetrics } from '@/lib/graph';
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
  metrics: Map<number, NodeMetrics>;
  stats: MetricStats;
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
   *  generator support, hover, the selected node itself). */
  glow: boolean;
}

const BASE = 6;

export function node3dStyle(n: GraphNode, c: StyleCtx): NodeStyle {
  // Importance (blast radius) drives base size + neutral hue in every mode.
  const m = c.metrics.get(n.id);
  const t = m ? importanceT(m, c.stats) : 0;
  const base = BASE * scale3D(t);
  let neutral = c.roots.has(n.id) ? PALETTE.root : importanceColor(t);
  if (c.dup.has(n.version_id)) neutral = PALETTE.dup;

  let style: NodeStyle;
  // TDA mode: the selected feature's generator support is the focus.
  if (c.mode === 'tda' && c.feature) {
    style = c.feature.nodeIds.has(n.id)
      ? { color: PALETTE.gen, scale: base * 1.5, dim: false, label: true, glow: true }
      : { color: dimHex(neutral, 0.82), scale: base * 0.6, dim: true, label: false, glow: false };
  } else if (c.sel != null) {
    if (n.id === c.sel) style = { color: PALETTE.center, scale: base * 1.5, dim: false, label: true, glow: true };
    else if (c.fwd.has(n.id)) style = { color: PALETTE.fwd, scale: base, dim: false, label: true, glow: true };
    else if (c.impact.has(n.id)) style = { color: PALETTE.impact, scale: base, dim: false, label: true, glow: true };
    else style = { color: dimHex(neutral, 0.82), scale: base * 0.6, dim: true, label: false, glow: false };
  } else if (c.matches.has(n.id)) {
    style = { color: PALETTE.accent, scale: base * 1.15, dim: false, label: true, glow: true };
  } else if (c.peaks.has(n.id)) {
    // Terrain peaks read as labeled markers even with nothing selected.
    style = { color: neutral, scale: base * 1.1, dim: false, label: true, glow: false };
  } else {
    style = { color: neutral, scale: base, dim: false, label: c.small, glow: false };
  }
  return style;
}

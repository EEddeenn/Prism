/* The unified visual-encoding scale, applied identically in the 2D and 3D views
 * so they reinforce each other. The headline channel is IMPORTANCE = a node's
 * blast radius (transitive dependents, revReach): the more of the graph depends
 * on a package, the bigger and warmer it renders. Area scales with the metric
 * (radius ∝ √value), and the same value drives a neutral-state hue ramp, so size
 * AND colour both carry importance (robust to colour-vision deficiency). */

import type { MetricStats, NodeMetrics } from './graph';
import { rampHex } from './palette';

/** Normalized 0..1 importance of a node by blast radius, area-proportional
 *  (√) so visual area tracks the dependent count rather than exaggerating hubs. */
export function importanceT(m: NodeMetrics, stats: MetricStats): number {
  const max = stats.maxRevReach || 1;
  return Math.sqrt(Math.max(0, Math.min(1, m.revReach / max)));
}

/** 2D node radius (px) from importance `t∈[0,1]`: 6 (clickable floor) → 16. */
export function radius2D(t: number): number {
  return 6 + 10 * t;
}

/** 3D base-scale factor from importance `t∈[0,1]`: 0.55 → 1.7 of the base size,
 *  applied BEFORE the selection-role multiplier so hubs stay prominent. */
export function scale3D(t: number): number {
  return 0.55 + 1.15 * t;
}

/** Neutral-state importance hue (reuses the terrain elevation ramp so the resting
 *  node-link views match the 3D terrain: cool = peripheral, warm = high blast).
 *  Floored into the ramp's upper range so peripheral nodes stay visible as node
 *  markers on the dark background (unlike the terrain surface, which may go dark
 *  in its valleys). */
export function importanceColor(t: number): string {
  return rampHex(0.35 + 0.6 * Math.max(0, Math.min(1, t)));
}

/** The metric label the size/colour channel currently encodes (for legends). */
export const SIZE_SIGNAL_LABEL = 'blast radius (transitive dependents)';

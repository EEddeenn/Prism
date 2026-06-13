/* 3D positions + decorations for each view mode. Pure functions of the graph and
 * metrics; the R3F scene consumes the output. Y is up (R3F convention).
 *
 *   layered — depth becomes stacked horizontal planes (roots on top).
 *   fiber   — each family is a vertical version stack; tall = duplicated.
 *   terrain — ground from the 2D layout; height = an analytical signal. */

import type { GraphEdge, GraphNode, Mode3D, TerrainSignal, Vec3 } from './types';
import type { GraphIndex, NodeMetrics } from './graph';
import { familyKey } from './graph';
import { shortLabel } from './palette';
import { verCmp } from './version';

export const LAYER_GAP = 78;
const NODE_SPACE = 46;
const FAM_GAP = 76;
const VER_GAP = 50;
export const TERRAIN_S = 320;
export const TERRAIN_H = 150;
const GRID_N = 46;

export interface LayerPlane {
  y: number;
  depth: number;
  count: number;
  radius: number;
}
export interface Fiber {
  x: number;
  z: number;
  top: number;
  dup: boolean;
  count: number;
  label: string;
}
export interface TerrainField {
  H: Float32Array;
  G: number;
  cell: number;
  size: number;
  maxHeight: number;
  /** Largest raw signal value across nodes — the top of the elevation ramp. */
  maxSignal: number;
  /** Which signal the elevation encodes (for the legend's name/units). */
  signal: TerrainSignal;
  /** True when revReach/fwdReach fell back to degree proxies (graph over cap). */
  approximate: boolean;
}
export interface Layout3D {
  pos: Map<number, Vec3>;
  planes?: LayerPlane[];
  fibers?: Fiber[];
  terrain?: TerrainField;
  /** Highest-signal node ids (terrain peaks) to label without a selection. */
  peaks?: number[];
  /** A scene-extent radius, for fog/grid sizing without a second bbox pass. */
  radius?: number;
}

/** Human name + unit for each terrain signal (shared by the controls + HUD). */
export const SIGNAL_LABELS: Record<TerrainSignal, { name: string; unit: string }> = {
  centrality: { name: 'Centrality', unit: 'transitive dependents' },
  reach: { name: 'Reach', unit: 'transitive dependencies' },
  fanin: { name: 'Fan-in', unit: 'direct dependents' },
  fanout: { name: 'Fan-out', unit: 'direct dependencies' },
  duplicates: { name: 'Duplicate pressure', unit: 'extra versions' },
  depth: { name: 'Depth', unit: 'filtration value' },
  risk: { name: 'Risk', unit: 'composite score' },
};

/** Signals that derive from transitive reach, so they read as approximate when
 *  the graph is over the reach cap (graph.ts TRANSITIVE_CAP). */
export const REACH_SIGNALS: ReadonlySet<TerrainSignal> = new Set<TerrainSignal>(['centrality', 'reach', 'risk']);

export function signalValue(m: NodeMetrics, sig: TerrainSignal): number {
  switch (sig) {
    case 'centrality': return m.revReach;
    case 'reach': return m.fwdReach;
    case 'fanin': return m.inDeg;
    case 'fanout': return m.outDeg;
    case 'duplicates': return m.famSize - 1;
    case 'depth': return m.depth;
    case 'risk': return m.revReach * (1 + (m.famSize - 1) * 0.6) * (1 + m.depth * 0.05);
    default: return m.revReach;
  }
}

export function buildLayout3D(
  mode: Mode3D,
  nodes: GraphNode[],
  index: GraphIndex,
  pos2d: Map<number, { x: number; y: number }>,
  signal: TerrainSignal,
): Layout3D {
  if (!nodes.length) return { pos: new Map() };
  const layout =
    mode === 'fiber'
      ? layoutFiber(nodes)
      : mode === 'terrain'
        ? layoutTerrain(nodes, index, pos2d, signal)
        : layoutLayered(nodes, index); // 'layered' and 'tda' share the substrate
  layout.radius = sceneRadius(layout.pos);
  return layout;
}

/** Half-diagonal of the layout's bounding box — drives fog/grid sizing. */
function sceneRadius(pos: Map<number, Vec3>): number {
  let minx = Infinity, miny = Infinity, minz = Infinity;
  let maxx = -Infinity, maxy = -Infinity, maxz = -Infinity;
  for (const p of pos.values()) {
    minx = Math.min(minx, p.x); maxx = Math.max(maxx, p.x);
    miny = Math.min(miny, p.y); maxy = Math.max(maxy, p.y);
    minz = Math.min(minz, p.z); maxz = Math.max(maxz, p.z);
  }
  if (!Number.isFinite(minx)) return 400;
  return Math.max(120, Math.hypot(maxx - minx, maxy - miny, maxz - minz) / 2);
}

export function layoutLayered(nodes: GraphNode[], index: GraphIndex): Layout3D {
  const pos = new Map<number, Vec3>();
  const byDepth = new Map<number, GraphNode[]>();
  let maxD = 0;
  for (const n of nodes) {
    const d = n.depth || 0;
    maxD = Math.max(maxD, d);
    if (!byDepth.has(d)) byDepth.set(d, []);
    byDepth.get(d)!.push(n);
  }
  const depths = Array.from(byDepth.keys()).sort((a, b) => a - b);
  const planes: LayerPlane[] = [];
  for (const d of depths) {
    const layerNodes = byDepth.get(d)!;
    const y = (maxD / 2 - d) * LAYER_GAP;
    const keyed = layerNodes.map((n) => {
      const parents = (index.inn.get(n.id) || []).map((p) => pos.get(p)).filter(Boolean) as Vec3[];
      const bx = parents.length ? parents.reduce((s, p) => s + p.x, 0) / parents.length : 0;
      const bz = parents.length ? parents.reduce((s, p) => s + p.z, 0) / parents.length : 0;
      return { n, k: bx * 1000 + bz };
    });
    keyed.sort((a, b) => a.k - b.k || a.n.id - b.n.id);
    const count = keyed.length;
    const cols = Math.max(1, Math.ceil(Math.sqrt(count * 1.3)));
    const rows = Math.ceil(count / cols);
    const w = (cols - 1) * NODE_SPACE;
    const h = (rows - 1) * NODE_SPACE;
    keyed.forEach((e, i) => {
      const c = i % cols;
      const r = Math.floor(i / cols);
      pos.set(e.n.id, { x: c * NODE_SPACE - w / 2, y, z: r * NODE_SPACE - h / 2 });
    });
    planes.push({ y, depth: d, count, radius: Math.max(w, h) / 2 + NODE_SPACE });
  }
  return { pos, planes };
}

export function layoutFiber(nodes: GraphNode[]): Layout3D {
  const pos = new Map<number, Vec3>();
  const fams = new Map<string, { nodes: GraphNode[]; label: string }>();
  for (const n of nodes) {
    const k = familyKey(n);
    if (!fams.has(k)) fams.set(k, { nodes: [], label: shortLabel(n.purl).split('@')[0] });
    fams.get(k)!.nodes.push(n);
  }
  const list = Array.from(fams.values());
  list.sort((a, b) => b.nodes.length - a.nodes.length || a.label.localeCompare(b.label));
  const cols = Math.max(1, Math.ceil(Math.sqrt(list.length * 1.2)));
  const rowsTotal = Math.floor(list.length / cols);
  const fibers: Fiber[] = [];
  list.forEach((fam, i) => {
    const c = i % cols;
    const r = Math.floor(i / cols);
    const gx = (c - (cols - 1) / 2) * FAM_GAP;
    const gz = (r - rowsTotal / 2) * FAM_GAP;
    const versions = Array.from(new Set(fam.nodes.map((n) => n.purl))).sort(verCmp);
    const rankOf = new Map<string, number>();
    versions.forEach((v, k) => rankOf.set(v, k));
    const seenRank = new Map<number, number>();
    for (const n of fam.nodes) {
      const rank = rankOf.get(n.purl) || 0;
      const dupAt = seenRank.get(rank) || 0;
      seenRank.set(rank, dupAt + 1);
      const a = dupAt * 1.7;
      const rr = dupAt ? 12 : 0;
      pos.set(n.id, { x: gx + Math.cos(a) * rr, y: rank * VER_GAP, z: gz + Math.sin(a) * rr });
    }
    fibers.push({
      x: gx,
      z: gz,
      top: (versions.length - 1) * VER_GAP,
      dup: versions.length > 1,
      count: versions.length,
      label: fam.label,
    });
  });
  return { pos, fibers };
}

export function layoutTerrain(
  nodes: GraphNode[],
  index: GraphIndex,
  pos2d: Map<number, { x: number; y: number }>,
  signal: TerrainSignal,
): Layout3D {
  const pos = new Map<number, Vec3>();
  let minx = Infinity, maxx = -Infinity, minz = Infinity, maxz = -Infinity;
  for (const n of nodes) {
    const p = pos2d.get(n.id);
    if (!p) continue;
    minx = Math.min(minx, p.x);
    maxx = Math.max(maxx, p.x);
    minz = Math.min(minz, p.y);
    maxz = Math.max(maxz, p.y);
  }
  const s = Math.min(
    (TERRAIN_S * 2) / Math.max(1, maxx - minx),
    (TERRAIN_S * 2) / Math.max(1, maxz - minz),
  );
  let maxSig = 0;
  for (const n of nodes) maxSig = Math.max(maxSig, signalValue(index.metrics.get(n.id)!, signal));
  const hScale = TERRAIN_H / (maxSig || 1);

  const ground = new Map<number, { gx: number; gz: number }>();
  const sig = new Map<number, number>();
  for (const n of nodes) {
    const p = pos2d.get(n.id) || { x: 0, y: 0 };
    const gx = (p.x - (minx + maxx) / 2) * s;
    const gz = (p.y - (minz + maxz) / 2) * s;
    const v = signalValue(index.metrics.get(n.id)!, signal);
    sig.set(n.id, v);
    ground.set(n.id, { gx, gz });
    pos.set(n.id, { x: gx, y: v * hScale, z: gz });
  }

  // The highest-signal nodes are the "peaks" worth naming without a selection.
  const peaks = Array.from(sig.entries())
    .sort((a, b) => b[1] - a[1])
    .slice(0, maxSig > 0 ? 12 : 0)
    .map(([id]) => id);

  const G = GRID_N;
  const H = new Float32Array(G * G).fill(NaN);
  const cell = (TERRAIN_S * 2) / (G - 1);
  for (const n of nodes) {
    const g = ground.get(n.id)!;
    const i = Math.round((g.gx + TERRAIN_S) / cell);
    const j = Math.round((g.gz + TERRAIN_S) / cell);
    if (i < 0 || j < 0 || i >= G || j >= G) continue;
    const y = pos.get(n.id)!.y;
    const k = j * G + i;
    if (Number.isNaN(H[k]) || y > H[k]) H[k] = y;
  }
  fillAndBlur(H, G);
  return {
    pos,
    peaks,
    terrain: {
      H, G, cell, size: TERRAIN_S, maxHeight: TERRAIN_H,
      maxSignal: maxSig, signal, approximate: index.approximate,
    },
  };
}

function fillAndBlur(H: Float32Array, G: number): void {
  for (let pass = 0; pass < 6; pass++) {
    let changed = false;
    const next = H.slice();
    for (let j = 0; j < G; j++)
      for (let i = 0; i < G; i++) {
        const k = j * G + i;
        if (!Number.isNaN(H[k])) continue;
        let sum = 0, cnt = 0;
        for (const [di, dj] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
          const ni = i + di, nj = j + dj;
          if (ni < 0 || nj < 0 || ni >= G || nj >= G) continue;
          const v = H[nj * G + ni];
          if (!Number.isNaN(v)) {
            sum += v;
            cnt++;
          }
        }
        if (cnt) {
          next[k] = sum / cnt;
          changed = true;
        }
      }
    H.set(next);
    if (!changed) break;
  }
  for (let k = 0; k < H.length; k++) if (Number.isNaN(H[k])) H[k] = 0;
  const out = H.slice();
  for (let j = 0; j < G; j++)
    for (let i = 0; i < G; i++) {
      let sum = 0, cnt = 0;
      for (let dj = -1; dj <= 1; dj++)
        for (let di = -1; di <= 1; di++) {
          const ni = i + di, nj = j + dj;
          if (ni < 0 || nj < 0 || ni >= G || nj >= G) continue;
          sum += H[nj * G + ni];
          cnt++;
        }
      out[j * G + i] = sum / cnt;
    }
  H.set(out);
}

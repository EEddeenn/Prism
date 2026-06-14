/* Derived graph structures: adjacency, roots, family grouping, and per-node
 * analytical metrics (degrees + bounded transitive reach), all computed once per
 * loaded run. Pure functions of the export payload. */

import type { GraphEdge, GraphNode } from './types';

export interface NodeMetrics {
  inDeg: number;
  outDeg: number;
  /** Transitive dependents (#nodes that can reach this one). */
  revReach: number;
  /** Transitive dependencies (#nodes reachable from this one). */
  fwdReach: number;
  /** Number of distinct versions in this node's package family. */
  famSize: number;
  depth: number;
}

/** Per-metric maxima over the whole run, computed once so every view normalizes
 *  size/colour against the SAME numbers (no per-render scans). */
export interface MetricStats {
  maxRevReach: number;
  maxFwdReach: number;
  maxInDeg: number;
  maxOutDeg: number;
  maxFamSize: number;
  maxDepth: number;
}

export interface GraphIndex {
  byId: Map<number, GraphNode>;
  out: Map<number, number[]>;
  inn: Map<number, number[]>;
  rootIds: Set<number>;
  metrics: Map<number, NodeMetrics>;
  /** Whole-run maxima for normalizing the size/colour encodings. */
  stats: MetricStats;
  /** `"source>target"` → edge kind/flags, for semantics-aware UI. */
  edgeMeta: Map<string, { kind: number; flags: number }>;
  /** True when transitive reach fell back to degree proxies (graph over cap), so
   *  revReach/fwdReach-derived rankings must be flagged as approximate. */
  approximate: boolean;
}

/** Family identity: the package id when present, else the purl's name part. */
export function familyKey(n: GraphNode): string {
  if (n.package_id != null) return 'p' + n.package_id;
  const s = (n.purl || '').replace(/^pkg:[^/]+\//, '');
  const at = s.lastIndexOf('@');
  return 'n' + (at >= 0 ? s.slice(0, at) : s);
}

/** Cap above which transitive reach falls back to degree proxies (keeps big
 * graphs interactive). The DFS is O(n·(n+m)); 4000 nodes stays well under ~100ms,
 * and rankings flag themselves as approximate beyond it. */
const TRANSITIVE_CAP = 4000;

export function buildIndex(nodes: GraphNode[], edges: GraphEdge[]): GraphIndex {
  const byId = new Map<number, GraphNode>();
  const out = new Map<number, number[]>();
  const inn = new Map<number, number[]>();
  const rootIds = new Set<number>();
  const edgeMeta = new Map<string, { kind: number; flags: number }>();
  for (const n of nodes) {
    byId.set(n.id, n);
    out.set(n.id, []);
    inn.set(n.id, []);
    if (n.depth === 0) rootIds.add(n.id);
  }
  for (const e of edges) {
    out.get(e.source)?.push(e.target);
    inn.get(e.target)?.push(e.source);
    edgeMeta.set(`${e.source}>${e.target}`, { kind: e.kind, flags: e.flags });
  }

  // Family sizes (distinct versions per family).
  const famVersions = new Map<string, Set<number>>();
  for (const n of nodes) {
    const k = familyKey(n);
    let set = famVersions.get(k);
    if (!set) famVersions.set(k, (set = new Set()));
    set.add(n.version_id);
  }

  const metrics = new Map<number, NodeMetrics>();
  for (const n of nodes) {
    metrics.set(n.id, {
      inDeg: inn.get(n.id)!.length,
      outDeg: out.get(n.id)!.length,
      revReach: inn.get(n.id)!.length,
      fwdReach: out.get(n.id)!.length,
      famSize: famVersions.get(familyKey(n))!.size,
      depth: n.depth || 0,
    });
  }

  // Bounded transitive reach via a forward DFS from every source: fwdReach[src]
  // = #reachable, and tallying each visited node yields revReach[t] = #ancestors.
  const approximate = nodes.length > TRANSITIVE_CAP;
  if (!approximate) {
    const idx = new Map<number, number>();
    nodes.forEach((n, i) => idx.set(n.id, i));
    for (const n of nodes) metrics.get(n.id)!.revReach = 0;
    const seen = new Int32Array(nodes.length).fill(-1);
    for (let s = 0; s < nodes.length; s++) {
      const src = nodes[s].id;
      let reached = 0;
      const stack = [src];
      while (stack.length) {
        const cur = stack.pop()!;
        for (const t of out.get(cur) || []) {
          const ti = idx.get(t);
          if (ti == null || seen[ti] === s) continue;
          seen[ti] = s;
          reached++;
          metrics.get(t)!.revReach++;
          stack.push(t);
        }
      }
      metrics.get(src)!.fwdReach = reached;
    }
  }

  // Whole-run maxima (one pass) so size/colour scales normalize consistently.
  const stats: MetricStats = {
    maxRevReach: 0, maxFwdReach: 0, maxInDeg: 0, maxOutDeg: 0, maxFamSize: 0, maxDepth: 0,
  };
  for (const m of metrics.values()) {
    if (m.revReach > stats.maxRevReach) stats.maxRevReach = m.revReach;
    if (m.fwdReach > stats.maxFwdReach) stats.maxFwdReach = m.fwdReach;
    if (m.inDeg > stats.maxInDeg) stats.maxInDeg = m.inDeg;
    if (m.outDeg > stats.maxOutDeg) stats.maxOutDeg = m.outDeg;
    if (m.famSize > stats.maxFamSize) stats.maxFamSize = m.famSize;
    if (m.depth > stats.maxDepth) stats.maxDepth = m.depth;
  }

  return { byId, out, inn, rootIds, metrics, stats, edgeMeta, approximate };
}

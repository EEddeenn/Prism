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

export interface GraphIndex {
  byId: Map<number, GraphNode>;
  out: Map<number, number[]>;
  inn: Map<number, number[]>;
  rootIds: Set<number>;
  metrics: Map<number, NodeMetrics>;
}

/** Family identity: the package id when present, else the purl's name part. */
export function familyKey(n: GraphNode): string {
  if (n.package_id != null) return 'p' + n.package_id;
  const s = (n.purl || '').replace(/^pkg:[^/]+\//, '');
  const at = s.lastIndexOf('@');
  return 'n' + (at >= 0 ? s.slice(0, at) : s);
}

/** Cap above which transitive reach falls back to degree proxies (keeps big
 * graphs interactive). */
const TRANSITIVE_CAP = 1500;

export function buildIndex(nodes: GraphNode[], edges: GraphEdge[]): GraphIndex {
  const byId = new Map<number, GraphNode>();
  const out = new Map<number, number[]>();
  const inn = new Map<number, number[]>();
  const rootIds = new Set<number>();
  for (const n of nodes) {
    byId.set(n.id, n);
    out.set(n.id, []);
    inn.set(n.id, []);
    if (n.depth === 0) rootIds.add(n.id);
  }
  for (const e of edges) {
    out.get(e.source)?.push(e.target);
    inn.get(e.target)?.push(e.source);
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
  if (nodes.length <= TRANSITIVE_CAP) {
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

  return { byId, out, inn, rootIds, metrics };
}

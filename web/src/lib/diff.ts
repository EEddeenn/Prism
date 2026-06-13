/* Cross-run package diff: join two runs by family (the portable PURL-derived
 * key) and classify each family as added / removed / version-changed / unchanged.
 * Pure function of two loaded runs — the same key works across re-resolutions
 * (and, less usefully, across ecosystems). */

import { familyKey, type GraphIndex } from './graph';
import type { GraphNode } from './types';
import { verCmp } from './version';

export type DiffStatus = 'added' | 'removed' | 'changed';

export interface FamilyDiff {
  key: string;
  /** Display name (no version), from the family's first occurrence. */
  name: string;
  /** Version strings present in run A (the active run), ascending. */
  aVersions: string[];
  /** Version strings present in run B (the compare run), ascending. */
  bVersions: string[];
  status: DiffStatus;
  /** Max blast radius (transitive dependents) in whichever run carries it. */
  blast: number;
  /** Sign of comparing the newest B vs newest A version: +1 up, -1 down, 0 same. */
  direction: number;
  /** Occurrence ids in the ACTIVE run (A), for highlightSet; empty for `added`. */
  ids: number[];
}

export interface DiffResult {
  added: FamilyDiff[];
  removed: FamilyDiff[];
  changed: FamilyDiff[];
  unchangedCount: number;
  /** Change counts bucketed by depth in the active run (added/changed only). */
  byDepth: Map<number, number>;
}

interface FamilyAgg {
  name: string;
  versions: Set<string>; // version strings (after '@')
  ids: number[]; // node ids (in that run)
  maxBlast: number;
  minDepth: number;
}

function versionOf(purl: string): string {
  const at = purl.lastIndexOf('@');
  return at >= 0 ? purl.slice(at + 1) : purl;
}

/** Family display name (no version), scoped-npm aware: a leading `@scope/` must
 *  not be mistaken for a version separator. */
function nameOf(purl: string): string {
  const s = (purl || '').replace(/^pkg:[^/]+\//, '');
  const at = s.lastIndexOf('@');
  return at > 0 ? s.slice(0, at) : s;
}

function aggregate(nodes: GraphNode[], index: GraphIndex): Map<string, FamilyAgg> {
  const m = new Map<string, FamilyAgg>();
  for (const n of nodes) {
    const k = familyKey(n);
    let f = m.get(k);
    if (!f) {
      f = { name: nameOf(n.purl), versions: new Set(), ids: [], maxBlast: 0, minDepth: Infinity };
      m.set(k, f);
    }
    f.versions.add(versionOf(n.purl));
    f.ids.push(n.id);
    f.maxBlast = Math.max(f.maxBlast, index.metrics.get(n.id)?.revReach ?? 0);
    f.minDepth = Math.min(f.minDepth, n.depth);
  }
  return m;
}

function sortedVersions(set: Set<string>): string[] {
  return [...set].sort((a, b) => verCmp(`@${a}`, `@${b}`));
}

export function diffRuns(
  aNodes: GraphNode[], aIndex: GraphIndex,
  bNodes: GraphNode[], bIndex: GraphIndex,
): DiffResult {
  const A = aggregate(aNodes, aIndex);
  const B = aggregate(bNodes, bIndex);

  const added: FamilyDiff[] = [];
  const removed: FamilyDiff[] = [];
  const changed: FamilyDiff[] = [];
  let unchangedCount = 0;
  const byDepth = new Map<number, number>();

  const bump = (depth: number) => {
    if (!Number.isFinite(depth)) return;
    byDepth.set(depth, (byDepth.get(depth) ?? 0) + 1);
  };

  const keys = new Set<string>([...A.keys(), ...B.keys()]);
  for (const k of keys) {
    const a = A.get(k);
    const b = B.get(k);
    if (a && !b) {
      removed.push({ key: k, name: a.name, aVersions: sortedVersions(a.versions), bVersions: [], status: 'removed', blast: a.maxBlast, direction: 0, ids: a.ids });
    } else if (!a && b) {
      const bv = sortedVersions(b.versions);
      added.push({ key: k, name: b.name, aVersions: [], bVersions: bv, status: 'added', blast: b.maxBlast, direction: 0, ids: [] });
      bump(b.minDepth);
    } else if (a && b) {
      const av = sortedVersions(a.versions);
      const bv = sortedVersions(b.versions);
      const same = av.length === bv.length && av.every((v, i) => v === bv[i]);
      if (same) {
        unchangedCount++;
      } else {
        const dir = Math.sign(verCmp(`@${bv[bv.length - 1]}`, `@${av[av.length - 1]}`));
        changed.push({ key: k, name: a.name, aVersions: av, bVersions: bv, status: 'changed', blast: Math.max(a.maxBlast, b.maxBlast), direction: dir, ids: a.ids });
        bump(a.minDepth);
      }
    }
  }

  const byBlast = (x: FamilyDiff, y: FamilyDiff) => y.blast - x.blast || x.name.localeCompare(y.name);
  added.sort(byBlast);
  removed.sort(byBlast);
  changed.sort(byBlast);
  return { added, removed, changed, unchangedCount, byDepth };
}

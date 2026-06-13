/* Edge semantics — decode the kind/flags the API already sends but the UI mostly
 * ignored. Kind ∈ runtime|dev|build|peer|optional|test|plugin|workspace;
 * flags carry resolution outcomes (the high-signal supply-chain smells). */

import { EDGE_KINDS, type EdgeFilter, type GraphEdge } from './types';

/** EdgeFlags bits (prism-model flags.rs). */
export const EDGE_FLAGS = {
  RESOLVED: 1 << 0,
  OPTIONAL_OMITTED: 1 << 1,
  PEER_UNMET: 1 << 2,
  OVERRIDDEN: 1 << 3,
  BACK_EDGE: 1 << 4,
} as const;

/** The flags that read as smells worth surfacing (not plain "resolved"). */
export const SMELL_FLAGS = EDGE_FLAGS.OPTIONAL_OMITTED | EDGE_FLAGS.PEER_UNMET | EDGE_FLAGS.OVERRIDDEN | EDGE_FLAGS.BACK_EDGE;

export function kindName(kind: number): string {
  return EDGE_KINDS[kind] ?? 'unknown';
}

/** Human names of the set flags (smells first). */
export function flagNames(flags: number): string[] {
  const out: string[] = [];
  if (flags & EDGE_FLAGS.PEER_UNMET) out.push('peer-unmet');
  if (flags & EDGE_FLAGS.OVERRIDDEN) out.push('overridden');
  if (flags & EDGE_FLAGS.OPTIONAL_OMITTED) out.push('optional-omitted');
  if (flags & EDGE_FLAGS.BACK_EDGE) out.push('back-edge');
  return out;
}

export function isBackEdge(e: GraphEdge): boolean {
  return (e.flags & EDGE_FLAGS.BACK_EDGE) !== 0;
}
export function isFlagged(e: GraphEdge): boolean {
  return (e.flags & SMELL_FLAGS) !== 0;
}

/** Per-kind edge colour for the lens (kept distinct on the dark theme). */
export const EDGE_KIND_COLOR: Record<string, string> = {
  runtime: '#5aa9ff',
  dev: '#6b7280',
  build: '#a78bfa',
  peer: '#f472b6',
  optional: '#586074',
  test: '#586074',
  plugin: '#34d399',
  workspace: '#ffb454',
  unknown: '#3a4150',
};

export function kindColor(kind: number): string {
  return EDGE_KIND_COLOR[kindName(kind)] ?? EDGE_KIND_COLOR.unknown;
}

/** Whether an edge passes the active lens filter. */
export function edgeVisible(e: GraphEdge, f: EdgeFilter): boolean {
  switch (f) {
    case 'runtime':
      return kindName(e.kind) === 'runtime';
    case 'nondev':
      return !['dev', 'test'].includes(kindName(e.kind));
    case 'flagged':
      return isFlagged(e);
    case 'all':
    default:
      return true;
  }
}

export const EDGE_FILTER_LABELS: Record<EdgeFilter, string> = {
  all: 'all',
  runtime: 'runtime',
  nondev: 'non-dev',
  flagged: 'flagged',
};

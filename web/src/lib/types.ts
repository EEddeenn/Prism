/* TypeScript mirror of the prism-serve JSON API shapes (see crates/prism-server
 * src/views_json.rs + router.rs). The PURL is the portable key; integer ids are
 * snapshot-local request handles. */

export interface RunSummary {
  run: number;
  nodes?: number;
  edges?: number;
  has_cycle?: boolean;
  available?: boolean;
}

export interface StatusResp {
  name: string;
  run_count: number;
  runs: number[];
}

export interface RunMeta {
  run: number;
  ecosystem: string;
  resolver: string;
  status: string;
  fidelity_tier: string;
  fidelity_caveats: string[];
}

export interface GraphStats {
  run: number;
  nodes: number;
  edges: number;
  scc_count: number;
  has_cycle: boolean;
  max_depth: number;
  duplicate_version_count: number;
}

export interface GraphNode {
  id: number;
  purl: string;
  version_id: number;
  package_id?: number;
  depth: number;
}

/** Edge `kind` discriminant (prism-graph builder.rs). */
export const EDGE_KINDS = [
  'runtime', 'dev', 'build', 'peer', 'optional', 'test', 'plugin', 'workspace',
] as const;

/** EdgeFlags bits (prism-model flags.rs). */
export const EDGE_FLAG_BACK_EDGE = 1 << 4;

export interface GraphEdge {
  source: number;
  target: number;
  kind: number;
  flags: number;
}

export interface ExportResp {
  run: number;
  nodes: number;
  edges: number;
  graph_nodes: GraphNode[];
  graph_edges: GraphEdge[];
}

export interface NeighborhoodResp {
  center: GraphNode;
  nodes: GraphNode[];
  edges: GraphEdge[];
  truncated: boolean;
}

export interface SccComp {
  is_cycle: boolean;
  members: GraphNode[];
}

export interface DupVersion {
  version_id: number;
  purl: string;
}

export interface DuplicateCluster {
  package_id: number;
  package_purl: string;
  version_count: number;
  occurrence_count: number;
  min_depth: number;
  max_depth: number;
  versions: DupVersion[];
}

export interface BarcodeDim {
  dim: number;
  finite: number;
  essential: number;
  total: number;
}

export interface Barcode {
  run: number;
  filtration: number;
  component_count: number;
  cycle_count: number;
  dims: BarcodeDim[];
}

export interface TdaPair {
  pair: number;
  dim: number;
  birth: number;
  death: number | null;
  essential: boolean;
  persistence: number | null;
}

export interface TdaPairsResp {
  run: number;
  filtration: number;
  count: number;
  pairs: TdaPair[];
}

export type SupportObject =
  | { type: 'node'; id: number; purl: string; version_id: number; package_id?: number; depth: number }
  | { type: 'edge'; id: number; source: number; target: number };

export interface TdaSupport {
  pair: number;
  dim: number;
  object_count: number;
  objects: SupportObject[];
}

export interface CandidateVersion {
  ordinal: number;
  version_id: number;
  purl: string;
}
export interface CandidateRange {
  start: number;
  end: number;
}
export interface CandidateExclusion {
  ordinal: number;
  version_id: number;
  reason: string;
}
export interface CandidatesResp {
  package_id: number;
  package_purl: string;
  constraint: string;
  total_versions: number;
  reason: string;
  satisfying_count: number;
  candidates: CandidateVersion[];
  ranges: CandidateRange[];
  exclusions: CandidateExclusion[];
}

// --- UI-side enums ----------------------------------------------------------

export type View = '2d' | '3d' | 'map';
export type Mode3D = 'layered' | 'fiber' | 'terrain' | 'tda';
export type EdgeFilter = 'all' | 'runtime' | 'nondev' | 'flagged';
export type TerrainSignal =
  | 'centrality' | 'reach' | 'fanin' | 'fanout' | 'duplicates' | 'depth' | 'risk';

export interface Vec3 {
  x: number;
  y: number;
  z: number;
}

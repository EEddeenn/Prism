/* TypeScript mirror of the in-browser WASM dispatcher's JSON shapes
 * (see crates/prism-api src/dispatch.rs + src/views_json.rs). The PURL is
 * the portable key; integer ids are snapshot-local request handles. */

export interface RunSummary {
  run: number;
  /** Ecosystem label (`cargo` / `npm` / `pypi` / …). Omitted by older
   *  dispatcher versions; the run-picker falls back to just `run N`. */
  ecosystem?: string;
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

// --- Structural analysis (prism-api structure/risk/clusters views) ---------

export interface RiskDimension {
  name: string;
  score: number;
  detail: string;
}

export interface RiskProfile {
  run: number;
  overall: number;
  band: string;
  dimensions: RiskDimension[];
}

export interface Chokepoint {
  dominated: number;
  node: GraphNode;
}

export interface ClassifiedCycle {
  class: string;
  advice: string;
  runtime_relevant: boolean;
  touches_root: boolean;
  internal_edges: number;
  external_dependents: number;
  kinds: string[];
  members: GraphNode[];
}

export interface IslandView {
  size: number;
  detached_from_roots: boolean;
  members: string[];
}

export interface Fragmentation {
  run: number;
  component_count: number;
  largest_component: number;
  artifact_singletons: number;
  note: string;
  subsystem_islands: IslandView[];
}

export interface ScopeRow {
  kind: string;
  direct_edges: number;
  direct_targets: number;
  footprint: number;
  exclusive: number;
  max_depth: number;
}

export interface WorkspaceRegion {
  run: number;
  ecosystem: string;
  nodes: number;
  edges: number;
  runtime_footprint: number;
  cycles: number;
  duplicate_families: number;
  risk_score: number;
  risk_band: string;
}

// --- "Why is this here?" (graph/why) ---------------------------------------

export interface Introducer {
  node: GraphNode;
  kinds: string[];
}

export interface Explanation {
  target: GraphNode;
  introducers: Introducer[];
  paths: GraphNode[][];
  nearest_owner: GraphNode | null;
  owner_distance: number;
  production_reachable: boolean;
  non_production_only: boolean;
}

// --- Duplicate-version root cause (graph/dup-causes) ------------------------

export interface ConstraintIntroducer {
  introducer_purl: string;
  constraint: string;
}

export interface DuplicateVersionCause {
  version_id: number;
  purl: string;
  occurrences: number;
  introducers: ConstraintIntroducer[];
}

export interface DuplicateRootCause {
  package_id: number;
  package_purl: string;
  versions: DuplicateVersionCause[];
  occurrence_count: number;
  blast_radius: number;
  distinct_constraints: number;
  convergence: string;
  risk_score: number;
}

// --- Subsystem partition (graph/subsystems) --------------------------------

export interface Subsystem {
  owner_purl: string;
  size: number;
  exclusive: number;
  runtime: boolean;
}

// --- Diamond reuse (graph/diamonds) ----------------------------------------

export interface DiamondNode {
  purl: string;
  parents: number;
  reconvergence: number;
  sample_parents: string[];
}

export interface DiamondReuse {
  run: number;
  total_reconvergence: number;
  top_share_pct: number;
  foundations: DiamondNode[];
}

// --- Minimal dominator skeleton (graph/minimal) ----------------------------

export interface MinimalEdge {
  gateway: string;
  node: string;
}

export interface MinimalView {
  run: number;
  original_edges: number;
  reduced_edges: number;
  edges: MinimalEdge[];
}

// --- Higher-order overlap (graph/shared) -----------------------------------

export interface SharedSurface {
  purl: string;
  spread: number;
  subsystems: string[];
}

export interface OverlapPair {
  a: string;
  b: string;
  shared: number;
}

export interface OverlapView {
  run: number;
  subsystems: number;
  shared_surfaces: SharedSurface[];
  top_pairs: OverlapPair[];
}

// --- Topology shape distance (graph/distance) ------------------------------

export interface MetricDelta {
  metric: string;
  base: number;
  head: number;
  change_pct: number;
}

export interface TopologyDistance {
  base: number;
  head: number;
  distance_pct: number;
  deltas: MetricDelta[];
}

// --- TDA: H2 voids (graph/h2) ----------------------------------------------

export interface H2View {
  run: number;
  betti0: number;
  betti1: number;
  betti2: number;
  triangles: number;
  /** Each void is the cycle of package occurrences bounding a 2-dimensional hole. */
  voids: GraphNode[][];
}

// --- TDA: multi-filtration comparison (tda/compare) ------------------------

export interface FiltrationSummary {
  filtration: string;
  h0_components: number;
  h1_loops: number;
  finite_bars: number;
  total_persistence: number;
  max_persistence: number;
}

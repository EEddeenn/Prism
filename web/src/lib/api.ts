/* Typed client for the Prism view dispatcher.
 *
 * Every method forwards into `prism_wasm.call_view(name, JSON.stringify(args))`
 * — the same view-name set the old `prism-server` HTTP router accepted. The
 * exported `api` object shape is unchanged, so existing call sites keep
 * working verbatim.
 */

import type {
  Barcode, CandidatesResp, DuplicateCluster, ExportResp, GraphStats, NeighborhoodResp,
  RunMeta, RunSummary, SccComp, StatusResp, TdaPairsResp, TdaSupport,
  RiskProfile, Chokepoint, ClassifiedCycle, Fragmentation, ScopeRow, WorkspaceRegion,
  Explanation, DuplicateRootCause, Subsystem, DiamondReuse, MinimalView, OverlapView,
  TopologyDistance, H2View, FiltrationSummary,
} from './types';
import { wasm } from './wasm';

export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
    this.name = 'ApiError';
  }
}

async function call<T>(name: string, args: object): Promise<T> {
  const m = await wasm();
  try {
    return JSON.parse(m.call_view(name, JSON.stringify(args))) as T;
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    throw new ApiError(msg, 0);
  }
}

export const api = {
  status: () => call<StatusResp>('status', {}),
  runs: () => call<RunSummary[]>('runs', {}),
  run: (run: number) => call<RunMeta>('run', { run }),
  stats: (run: number) => call<GraphStats>('graph/stats', { run }),
  exportGraph: (run: number) => call<ExportResp>('export', { run }),
  neighborhood: (run: number, node: number, depth = 64) =>
    call<NeighborhoodResp>('graph/neighborhood', { run, node, depth }),
  impact: (run: number, node: number) =>
    call<NeighborhoodResp>('graph/impact', { run, node }),
  scc: (run: number) => call<SccComp[]>('graph/scc', { run }),
  duplicates: (run: number) => call<DuplicateCluster[]>('graph/duplicates', { run }),
  barcode: (run: number) => call<Barcode>('tda', { run }),
  tdaPairs: (run: number) => call<TdaPairsResp>('tda/pairs', { run }),
  tdaSupport: (run: number, pair: number) =>
    call<TdaSupport>('tda/support', { run, pair }),
  candidates: (pkg: string, constraint?: string) =>
    call<CandidatesResp>(
      'package/candidates',
      constraint ? { package: pkg, constraint } : { package: pkg },
    ),
  risk: (run: number) => call<RiskProfile>('graph/risk', { run }),
  chokepoints: (run: number, limit = 12) =>
    call<Chokepoint[]>('graph/chokepoints', { run, limit }),
  classifiedCycles: (run: number) => call<ClassifiedCycle[]>('graph/cycles', { run }),
  fragmentation: (run: number) => call<Fragmentation>('graph/fragmentation', { run }),
  scope: (run: number) => call<ScopeRow[]>('graph/scope', { run }),
  regions: () => call<WorkspaceRegion[]>('regions', {}),

  // Interpretive analytics (formerly CLI-only) ------------------------------
  why: (run: number, node: number) =>
    call<Explanation>('graph/why', { run, node }),
  dupCauses: (run: number) => call<DuplicateRootCause[]>('graph/dup-causes', { run }),
  subsystems: (run: number) => call<Subsystem[]>('graph/subsystems', { run }),
  diamonds: (run: number, limit = 12) =>
    call<DiamondReuse>('graph/diamonds', { run, limit }),
  minimal: (run: number) => call<MinimalView>('graph/minimal', { run }),
  shared: (run: number, limit = 12) =>
    call<OverlapView>('graph/shared', { run, limit }),
  distance: (base: number, head: number) =>
    call<TopologyDistance>('graph/distance', { base, head }),

  // TDA: H2 voids + multi-filtration comparison -----------------------------
  h2: (run: number) => call<H2View>('graph/h2', { run }),
  tdaCompare: (run: number) => call<FiltrationSummary[]>('tda/compare', { run }),
};

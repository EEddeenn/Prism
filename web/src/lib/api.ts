/* Typed client for the prism-serve JSON API.
 *
 * By default the browser calls this Next.js app same-origin and the server-side
 * proxy at `app/api/[...path]/route.ts` forwards to prism-serve — so no CORS is
 * involved. Set `NEXT_PUBLIC_PRISM_API` to have the browser hit prism-serve
 * directly instead (prism-serve sends open CORS headers). */

import type {
  Barcode, CandidatesResp, DuplicateCluster, ExportResp, GraphStats, NeighborhoodResp,
  RunMeta, RunSummary, SccComp, StatusResp, TdaPairsResp, TdaSupport,
  RiskProfile, Chokepoint, ClassifiedCycle, Fragmentation, ScopeRow, WorkspaceRegion,
  Explanation, DuplicateRootCause, Subsystem, DiamondReuse, MinimalView, OverlapView,
  TopologyDistance, H2View, FiltrationSummary,
} from './types';

const BASE = process.env.NEXT_PUBLIC_PRISM_API ?? '';

export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
    this.name = 'ApiError';
  }
}

async function getJson<T>(path: string): Promise<T> {
  let resp: Response;
  try {
    resp = await fetch(`${BASE}${path}`, { cache: 'no-store' });
  } catch {
    throw new ApiError('cannot reach the Prism API (is prism-serve running?)', 0);
  }
  let body: unknown = null;
  try {
    body = await resp.json();
  } catch {
    /* tolerate a non-JSON body (e.g. an upstream error) */
  }
  if (!resp.ok) {
    const msg =
      body && typeof body === 'object' && 'error' in body
        ? String((body as { error: unknown }).error)
        : `HTTP ${resp.status}`;
    throw new ApiError(msg, resp.status);
  }
  return body as T;
}

export const api = {
  status: () => getJson<StatusResp>('/api/status'),
  runs: () => getJson<RunSummary[]>('/api/runs'),
  run: (run: number) => getJson<RunMeta>(`/api/run?run=${run}`),
  stats: (run: number) => getJson<GraphStats>(`/api/graph/stats?run=${run}`),
  exportGraph: (run: number) => getJson<ExportResp>(`/api/export?run=${run}`),
  neighborhood: (run: number, node: number, depth = 64) =>
    getJson<NeighborhoodResp>(`/api/graph/neighborhood?run=${run}&node=${node}&depth=${depth}`),
  impact: (run: number, node: number) =>
    getJson<NeighborhoodResp>(`/api/graph/impact?run=${run}&node=${node}`),
  scc: (run: number) => getJson<SccComp[]>(`/api/graph/scc?run=${run}`),
  duplicates: (run: number) => getJson<DuplicateCluster[]>(`/api/graph/duplicates?run=${run}`),
  barcode: (run: number) => getJson<Barcode>(`/api/tda?run=${run}`),
  tdaPairs: (run: number) => getJson<TdaPairsResp>(`/api/tda/pairs?run=${run}`),
  tdaSupport: (run: number, pair: number) =>
    getJson<TdaSupport>(`/api/tda/support?run=${run}&pair=${pair}`),
  candidates: (pkg: string, constraint?: string) =>
    getJson<CandidatesResp>(
      `/api/package/candidates?package=${encodeURIComponent(pkg)}` +
        (constraint ? `&constraint=${encodeURIComponent(constraint)}` : ''),
    ),
  risk: (run: number) => getJson<RiskProfile>(`/api/graph/risk?run=${run}`),
  chokepoints: (run: number, limit = 12) =>
    getJson<Chokepoint[]>(`/api/graph/chokepoints?run=${run}&limit=${limit}`),
  classifiedCycles: (run: number) => getJson<ClassifiedCycle[]>(`/api/graph/cycles?run=${run}`),
  fragmentation: (run: number) => getJson<Fragmentation>(`/api/graph/fragmentation?run=${run}`),
  scope: (run: number) => getJson<ScopeRow[]>(`/api/graph/scope?run=${run}`),
  regions: () => getJson<WorkspaceRegion[]>('/api/regions'),

  // Interpretive analytics (formerly CLI-only) ------------------------------
  why: (run: number, node: number) =>
    getJson<Explanation>(`/api/graph/why?run=${run}&node=${node}`),
  dupCauses: (run: number) => getJson<DuplicateRootCause[]>(`/api/graph/dup-causes?run=${run}`),
  subsystems: (run: number) => getJson<Subsystem[]>(`/api/graph/subsystems?run=${run}`),
  diamonds: (run: number, limit = 12) =>
    getJson<DiamondReuse>(`/api/graph/diamonds?run=${run}&limit=${limit}`),
  minimal: (run: number) => getJson<MinimalView>(`/api/graph/minimal?run=${run}`),
  shared: (run: number, limit = 12) =>
    getJson<OverlapView>(`/api/graph/shared?run=${run}&limit=${limit}`),
  distance: (base: number, head: number) =>
    getJson<TopologyDistance>(`/api/graph/distance?base=${base}&head=${head}`),

  // TDA: H2 voids + multi-filtration comparison -----------------------------
  h2: (run: number) => getJson<H2View>(`/api/graph/h2?run=${run}`),
  tdaCompare: (run: number) => getJson<FiltrationSummary[]>(`/api/tda/compare?run=${run}`),
};

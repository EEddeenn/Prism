/* The single shared store for the explorer.
 *
 * Both the 2D and 3D views render from the SAME selection/highlight state here,
 * so clicking a node, a duplicate family, a cycle member, or a TDA feature lights
 * up in whichever view is active. Async actions call the typed `api` client. */

import { create } from 'zustand';
import { api, ApiError } from '@/lib/api';
import { buildIndex, type GraphIndex } from '@/lib/graph';
import { layout2d } from '@/lib/layout2d';
import type {
  Barcode, DuplicateCluster, EdgeFilter, GraphEdge, GraphNode, Mode3D, RunMeta, RunSummary,
  SccComp, TdaPair, TerrainSignal, View,
} from '@/lib/types';

export interface FeatureHL {
  pair: number;
  dim: number;
  nodeIds: Set<number>;
  edgeKeys: Set<string>;
}

/** A second run loaded alongside the active one, for compare/diff. Survives
 *  primary-run switches (its own slot — `selectRun` never touches it). */
export interface CompareData {
  run: number;
  nodes: GraphNode[];
  index: GraphIndex;
  stats: import('@/lib/types').GraphStats | null;
  runMeta: RunMeta | null;
  barcode: Barcode | null;
}

interface GraphState {
  // status
  statusText: string;
  statusError: boolean;

  // runs + current graph
  runs: RunSummary[];
  run: number | null;
  nodes: GraphNode[];
  edges: GraphEdge[];
  index: GraphIndex | null;
  pos2d: Map<number, { x: number; y: number }>;
  world2d: { w: number; h: number };

  // selection / highlight (shared by 2D + 3D)
  sel: number | null;
  fwd: Set<number>;
  impact: Set<number>;
  matches: Set<number>;
  dupVersionIds: Set<number>;
  feature: FeatureHL | null;

  // view controls
  view: View;
  mode3d: Mode3D;
  signal: TerrainSignal;
  activeTab: string;
  query: string;
  edgeFilter: EdgeFilter;

  // camera/viewport focus request (a nonce the views watch to recenter)
  focusTarget: number | null;
  focusNonce: number;

  // transient hover (3D): the node under the cursor. Lives here so only the DOM
  // HUD subscribes — the WebGL scene never re-renders on a hover change.
  hovered: number | null;

  // side-panel data
  runMeta: RunMeta | null;
  stats: import('@/lib/types').GraphStats | null;
  duplicates: DuplicateCluster[];
  sccs: SccComp[];
  barcode: Barcode | null;
  pairs: TdaPair[];

  // compare/diff: a second run loaded into its own slot
  compareRun: number | null;
  compareData: CompareData | null;

  // actions
  setStatus: (text: string, error?: boolean) => void;
  loadRuns: (preferRun?: number) => Promise<void>;
  selectRun: (run: number) => Promise<void>;
  selectNode: (id: number) => Promise<void>;
  highlightSet: (ids: number[], label: string) => void;
  clearSelection: () => void;
  setView: (v: View) => void;
  setMode3d: (m: Mode3D) => void;
  setSignal: (s: TerrainSignal) => void;
  setActiveTab: (tab: string) => void;
  setEdgeFilter: (f: EdgeFilter) => void;
  runSearch: (term: string) => void;
  selectFeature: (pair: number, dim: number) => Promise<void>;
  requestFocus: (id: number) => void;
  setHovered: (id: number | null) => void;
  setCompareRun: (run: number | null) => Promise<void>;
}

function errMsg(e: unknown): string {
  return e instanceof ApiError || e instanceof Error ? e.message : String(e);
}

export const useGraphStore = create<GraphState>((set, get) => ({
  statusText: '',
  statusError: false,
  runs: [],
  run: null,
  nodes: [],
  edges: [],
  index: null,
  pos2d: new Map(),
  world2d: { w: 1, h: 1 },
  sel: null,
  fwd: new Set(),
  impact: new Set(),
  matches: new Set(),
  dupVersionIds: new Set(),
  feature: null,
  view: '2d',
  mode3d: 'layered',
  signal: 'centrality',
  activeTab: 'overview',
  query: '',
  edgeFilter: 'all',
  focusTarget: null,
  focusNonce: 0,
  hovered: null,
  runMeta: null,
  stats: null,
  duplicates: [],
  sccs: [],
  barcode: null,
  pairs: [],
  compareRun: null,
  compareData: null,

  setStatus: (text, error = false) => set({ statusText: text, statusError: error }),

  loadRuns: async (preferRun) => {
    set({ statusText: 'Loading runs…', statusError: false });
    try {
      const runs = await api.runs();
      set({ runs, statusText: '' });
      if (!runs.length) {
        set({ statusText: 'No runs — ingest a lockfile with the CLI first.' });
        return;
      }
      const target = preferRun != null && runs.some((r) => r.run === preferRun) ? preferRun : runs[0].run;
      await get().selectRun(target);
    } catch (e) {
      set({ statusText: errMsg(e), statusError: true });
    }
  },

  selectRun: async (run) => {
    // Picking the current compare run as the primary would diff a run against
    // itself — drop the now-redundant compare slot instead.
    if (get().compareRun === run) set({ compareRun: null, compareData: null });
    set({ statusText: 'Loading graph…', statusError: false, run });
    try {
      const data = await api.exportGraph(run);
      // The user may have switched runs while this was in flight — if so, drop
      // this stale result rather than clobbering the newer run's graph.
      if (get().run !== run) return;
      const nodes = data.graph_nodes || [];
      const edges = data.graph_edges || [];
      const index = buildIndex(nodes, edges);
      const { pos, world } = layout2d(nodes, edges, index);
      set({
        nodes,
        edges,
        index,
        pos2d: pos,
        world2d: world,
        sel: null,
        fwd: new Set(),
        impact: new Set(),
        matches: new Set(),
        feature: null,
        dupVersionIds: new Set(),
        activeTab: 'overview',
        query: '',
      });

      // Side-panel data in parallel; one failure must not blank the others.
      const [meta, stats, dups, sccs, barcode, pairs] = await Promise.allSettled([
        api.run(run),
        api.stats(run),
        api.duplicates(run),
        api.scc(run),
        api.barcode(run),
        api.tdaPairs(run),
      ]);
      if (get().run !== run) return; // a newer run won the race
      const dupVersionIds = new Set<number>();
      const duplicates = dups.status === 'fulfilled' ? dups.value : [];
      for (const c of duplicates) for (const v of c.versions) dupVersionIds.add(v.version_id);
      set({
        runMeta: meta.status === 'fulfilled' ? meta.value : null,
        stats: stats.status === 'fulfilled' ? stats.value : null,
        duplicates,
        dupVersionIds,
        sccs: sccs.status === 'fulfilled' ? sccs.value : [],
        barcode: barcode.status === 'fulfilled' ? barcode.value : null,
        pairs: pairs.status === 'fulfilled' ? pairs.value.pairs : [],
        statusText: '',
      });
    } catch (e) {
      set({ statusText: errMsg(e), statusError: true });
    }
  },

  selectNode: async (id) => {
    const { run } = get();
    if (run == null) return;
    // An explicit node pick drops any TDA feature lens so the selection shows in
    // 3D too (the feature highlight otherwise takes precedence over selection).
    set({ sel: id, feature: null, activeTab: 'node', statusText: 'Tracing dependencies…' });
    try {
      const [nb, imp] = await Promise.all([api.neighborhood(run, id), api.impact(run, id)]);
      const cur = get();
      if (cur.run !== run || cur.sel !== id) return; // selection moved on
      const fwd = new Set<number>([id]);
      for (const n of nb.nodes || []) fwd.add(n.id);
      const impact = new Set<number>([id]);
      for (const n of imp.nodes || []) impact.add(n.id);
      set({ fwd, impact, statusText: '' });
    } catch (e) {
      set({ statusText: errMsg(e), statusError: true });
    }
  },

  highlightSet: (ids, label) => {
    if (!ids.length) {
      set({ statusText: `No occurrences of ${label} in this graph.` });
      return;
    }
    // The treemap shows only the single `sel` ring, not the multi-node fwd set,
    // so a highlight there would be invisible — drop back to the 2D graph.
    set((s) => ({
      view: s.view === 'map' ? '2d' : s.view,
      sel: ids[0],
      fwd: new Set(ids),
      impact: new Set(),
      feature: null,
    }));
    get().requestFocus(ids[0]);
    const short = label.replace(/^pkg:[^/]+\//, '');
    set({ statusText: `${ids.length} occurrence(s) of ${short} highlighted.` });
  },

  clearSelection: () => set({ sel: null, fwd: new Set(), impact: new Set() }),

  setView: (view) => set({ view }),
  setMode3d: (mode3d) => {
    set((s) => ({ mode3d, feature: mode3d === 'tda' ? s.feature : null }));
    if (mode3d === 'tda') set({ activeTab: 'tda' });
  },
  setSignal: (signal) => set({ signal }),
  setActiveTab: (activeTab) => set({ activeTab }),
  setEdgeFilter: (edgeFilter) => set({ edgeFilter }),

  runSearch: (term) => {
    set({ query: term });
    const t = term.trim().toLowerCase();
    if (!t) {
      set({ matches: new Set() });
      return;
    }
    const matches = new Set<number>();
    let first: number | null = null;
    for (const n of get().nodes) {
      if (n.purl.toLowerCase().includes(t)) {
        matches.add(n.id);
        if (first == null) first = n.id;
      }
    }
    set({ matches });
    if (first != null) {
      get().requestFocus(first);
      void get().selectNode(first);
    }
  },

  selectFeature: async (pair, dim) => {
    const { run } = get();
    if (run == null) return;
    set({ statusText: 'Resolving feature generators…', statusError: false });
    try {
      const sup = await api.tdaSupport(run, pair);
      const nodeIds = new Set<number>();
      const edgeKeys = new Set<string>();
      for (const o of sup.objects) {
        if (o.type === 'node') nodeIds.add(o.id);
        else {
          edgeKeys.add(`${o.source}>${o.target}`);
          nodeIds.add(o.source);
          nodeIds.add(o.target);
        }
      }
      const nN = sup.objects.filter((o) => o.type === 'node').length;
      const nE = sup.objects.filter((o) => o.type === 'edge').length;
      set({
        feature: { pair, dim: sup.dim, nodeIds, edgeKeys },
        view: '3d',
        mode3d: 'tda',
        statusText: `H${dim} generator lit: ${nN} node(s), ${nE} edge(s).`,
      });
      // Recenter on the lit generator, like search/duplicate highlight do.
      const firstId = nodeIds.values().next().value;
      if (firstId != null) get().requestFocus(firstId);
    } catch (e) {
      set({ statusText: errMsg(e), statusError: true });
    }
  },

  requestFocus: (id) => set((s) => ({ focusTarget: id, focusNonce: s.focusNonce + 1 })),

  setHovered: (id) => {
    // Skip no-op writes so crossing within one node never notifies subscribers.
    if (get().hovered !== id) set({ hovered: id });
  },

  setCompareRun: async (run) => {
    if (run == null) {
      set({ compareRun: null, compareData: null });
      return;
    }
    set({ compareRun: run });
    try {
      const [data, stats, runMeta, barcode] = await Promise.all([
        api.exportGraph(run),
        api.stats(run).catch(() => null),
        api.run(run).catch(() => null),
        api.barcode(run).catch(() => null),
      ]);
      if (get().compareRun !== run) return; // a newer compare target won the race
      const nodes = data.graph_nodes || [];
      const index = buildIndex(nodes, data.graph_edges || []);
      set({ compareData: { run, nodes, index, stats, runMeta, barcode } });
    } catch (e) {
      set({ statusText: errMsg(e), statusError: true, compareRun: null, compareData: null });
    }
  },
}));

'use client';

/* Two-way sync between a compact querystring and the store, so any view is
 * shareable/reproducible: ?run=&view=&mode=&sig=&ef=&q=&sel=<purl>. The selected
 * node is written as a PURL (snapshot-local ids don't survive re-ingest) and
 * resolved back to an id once the graph loads. */

import { useEffect, useRef } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import type { EdgeFilter, Mode3D, TerrainSignal, View } from './types';

const VIEWS: View[] = ['2d', '3d', 'map'];
const MODES: Mode3D[] = ['layered', 'fiber', 'terrain', 'tda'];
const SIGNALS: TerrainSignal[] = ['centrality', 'reach', 'fanin', 'fanout', 'duplicates', 'depth', 'risk'];
const FILTERS: EdgeFilter[] = ['all', 'runtime', 'nondev', 'flagged'];

export function useUrlSync() {
  const nodes = useGraphStore((s) => s.nodes);
  const hydrated = useRef(false);
  const applied = useRef(false);
  const urlQuery = useRef<string | null>(null);
  const pendingSel = useRef<string | null>(null);

  // Hydrate the store from the URL once, then kick off the initial load.
  useEffect(() => {
    if (hydrated.current) return;
    hydrated.current = true;
    const sp = new URLSearchParams(window.location.search);
    const s = useGraphStore.getState();

    const view = sp.get('view') as View | null;
    if (view && VIEWS.includes(view)) s.setView(view);
    const mode = sp.get('mode') as Mode3D | null;
    if (mode && MODES.includes(mode)) s.setMode3d(mode);
    const sig = sp.get('sig') as TerrainSignal | null;
    if (sig && SIGNALS.includes(sig)) s.setSignal(sig);
    const ef = sp.get('ef') as EdgeFilter | null;
    if (ef && FILTERS.includes(ef)) s.setEdgeFilter(ef);

    urlQuery.current = sp.get('q');
    pendingSel.current = sp.get('sel');
    const run = sp.get('run');
    void s.loadRuns(run ? Number(run) : undefined);
  }, []);

  // Once the graph is loaded, apply the URL's query + selected purl (selectRun
  // clears those, so they must be (re)applied after, not during, hydration).
  useEffect(() => {
    if (applied.current || !nodes.length) return;
    applied.current = true;
    const s = useGraphStore.getState();
    if (urlQuery.current) s.runSearch(urlQuery.current);
    if (pendingSel.current) {
      const purl = pendingSel.current;
      const n = nodes.find((x) => x.purl === purl);
      if (n) void s.selectNode(n.id);
    }
    pendingSel.current = null;
  }, [nodes]);

  // Reflect store changes back into the URL (debounced, no navigation).
  useEffect(() => {
    let t: ReturnType<typeof setTimeout> | undefined;
    const write = () => {
      // Don't write until the deep-link's query/selection have been (re)applied,
      // or a slow graph fetch would transiently rewrite the URL to just ?run=N
      // and drop the not-yet-applied ?sel=/?q=.
      if (!applied.current) return;
      const s = useGraphStore.getState();
      const sp = new URLSearchParams();
      if (s.run != null) sp.set('run', String(s.run));
      if (s.view !== '2d') sp.set('view', s.view);
      if (s.view === '3d' && s.mode3d !== 'layered') sp.set('mode', s.mode3d);
      if (s.view === '3d' && s.mode3d === 'terrain') sp.set('sig', s.signal);
      if (s.edgeFilter !== 'all') sp.set('ef', s.edgeFilter);
      if (s.query) sp.set('q', s.query);
      if (s.sel != null && s.index) {
        const n = s.index.byId.get(s.sel);
        if (n) sp.set('sel', n.purl);
      }
      const qs = sp.toString();
      window.history.replaceState(null, '', qs ? `?${qs}` : window.location.pathname);
    };
    const unsub = useGraphStore.subscribe(() => {
      if (t) clearTimeout(t);
      t = setTimeout(write, 250);
    });
    return () => {
      if (t) clearTimeout(t);
      unsub();
    };
  }, []);
}

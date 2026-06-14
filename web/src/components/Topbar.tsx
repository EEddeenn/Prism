'use client';

import { useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { EDGE_FILTER_LABELS } from '@/lib/edges';
import { SegmentedControl, type SegOption } from '@/components/ui';
import type { EdgeFilter, View } from '@/lib/types';

const VIEW_LABELS: Record<View, string> = { '2d': '2D', '3d': '3D', map: 'Map' };
const EDGE_FILTERS: EdgeFilter[] = ['all', 'runtime', 'nondev', 'flagged'];

const VIEW_OPTIONS: SegOption<View>[] = (['2d', '3d', 'map'] as const).map((v) => ({
  value: v,
  label: VIEW_LABELS[v],
}));
const EDGE_OPTIONS: SegOption<EdgeFilter>[] = EDGE_FILTERS.map((f) => ({
  value: f,
  label: EDGE_FILTER_LABELS[f],
}));

/** A thin vertical rule separating Topbar clusters. */
function Divider() {
  return <span aria-hidden className="self-stretch w-px bg-line mx-1" />;
}

export default function Topbar() {
  const runs = useGraphStore((s) => s.runs);
  const run = useGraphStore((s) => s.run);
  const compareRun = useGraphStore((s) => s.compareRun);
  const setCompareRun = useGraphStore((s) => s.setCompareRun);
  const view = useGraphStore((s) => s.view);
  const edgeFilter = useGraphStore((s) => s.edgeFilter);
  const statusText = useGraphStore((s) => s.statusText);
  const statusError = useGraphStore((s) => s.statusError);
  const matches = useGraphStore((s) => s.matches);
  const query = useGraphStore((s) => s.query);
  const selectRun = useGraphStore((s) => s.selectRun);
  const setView = useGraphStore((s) => s.setView);
  const setEdgeFilter = useGraphStore((s) => s.setEdgeFilter);
  const runSearch = useGraphStore((s) => s.runSearch);
  const loadRuns = useGraphStore((s) => s.loadRuns);
  const [copied, setCopied] = useState(false);

  const copyLink = async () => {
    try {
      await navigator.clipboard.writeText(window.location.href);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard may be unavailable; ignore */
    }
  };

  return (
    <header className="flex flex-wrap items-center gap-3 px-4 py-2 border-b border-line bg-gradient-to-b from-[#181c28] to-panel">
      {/* nav cluster: brand + run pickers */}
      <span className="font-extrabold text-base tracking-wide text-accent">Prism</span>
      <span className="text-mini uppercase tracking-[1px] text-muted -ml-1.5 mr-1">
        dependency topology
      </span>

      <div className="flex items-center gap-1.5">
        <label htmlFor="run-select" className="text-body text-muted">Run</label>
        <select
          id="run-select"
          className="px-2.5 py-1.5 text-body max-w-[240px]"
          value={run ?? ''}
          onChange={(e) => e.target.value && selectRun(Number(e.target.value))}
        >
          {!runs.length && <option value="">(no ingested runs)</option>}
          {runs.map((r) => (
            <option key={r.run} value={r.run} disabled={r.run === compareRun}>
              run {r.run}
              {r.nodes != null ? ` · ${r.nodes}n/${r.edges}e` : ''}
            </option>
          ))}
        </select>
      </div>

      <div className="flex items-center gap-1.5" title="Diff the active run against another run">
        <label htmlFor="compare-select" className="text-body text-muted">vs</label>
        <select
          id="compare-select"
          className="px-2.5 py-1.5 text-body max-w-[180px]"
          value={compareRun ?? ''}
          onChange={(e) => setCompareRun(e.target.value ? Number(e.target.value) : null)}
        >
          <option value="">(no compare)</option>
          {runs.map((r) => (
            <option key={r.run} value={r.run} disabled={r.run === run}>
              run {r.run}
            </option>
          ))}
        </select>
      </div>

      <Divider />

      {/* view + edge config cluster */}
      <SegmentedControl
        options={VIEW_OPTIONS}
        value={view}
        onChange={setView}
        ariaLabel="View mode"
      />

      <div className="flex items-center gap-1.5" title="Edge lens: filter/colour edges by dependency kind">
        <span className="text-body text-muted">Edges</span>
        <SegmentedControl
          options={EDGE_OPTIONS}
          value={edgeFilter}
          onChange={setEdgeFilter}
          ariaLabel="Edge filter"
          size="sm"
        />
      </div>

      <Divider />

      {/* search + actions cluster */}
      <div className="flex items-center gap-1.5 flex-1 min-w-0 basis-[200px] max-w-[280px]">
        <input
          type="search"
          placeholder="Find a package…"
          aria-label="Find a package"
          autoComplete="off"
          spellCheck={false}
          value={query}
          className="w-full min-w-0 px-2.5 py-1.5 text-body placeholder:text-muted"
          onChange={(e) => runSearch(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              runSearch('');
              (e.target as HTMLInputElement).blur();
            }
          }}
        />
        <span className="text-mini text-muted min-w-[28px]">
          {matches.size ? `${matches.size} hit${matches.size === 1 ? '' : 's'}` : ''}
        </span>
      </div>

      <button
        type="button"
        onClick={copyLink}
        title="Copy a deep link to this exact view"
        aria-label="Copy a deep link to this exact view"
        className="px-2.5 py-1.5 text-body font-semibold rounded-md border border-line-2 bg-panel-2 text-ink hover:border-accent hover:bg-panel-3"
      >
        {copied ? 'Copied ✓' : 'Copy link'}
      </button>

      <button
        type="button"
        onClick={() => loadRuns()}
        aria-label="Reload runs"
        className="px-2.5 py-1.5 text-body font-semibold rounded-md border border-line-2 bg-panel-2 text-ink hover:border-accent hover:bg-panel-3"
      >
        Reload
      </button>

      <span
        className={`ml-auto text-body truncate max-w-[36%] ${statusError ? 'text-bad' : 'text-muted'}`}
        role={statusError ? 'alert' : 'status'}
      >
        {statusText}
      </span>
    </header>
  );
}

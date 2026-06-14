'use client';

import { useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { EDGE_FILTER_LABELS } from '@/lib/edges';
import { SegmentedControl, type SegOption } from '@/components/ui';
import Logo from '@/components/Logo';
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

/* Lightweight inline glyphs used by the Topbar action buttons. Each is a
 * 14×14 currentColor SVG so it inherits the surrounding text colour and
 * scales cleanly with text-body. Kept inline to avoid a new icon package. */
function LinkIcon({ className }: { className?: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden className={className}>
      <path d="M10 13a5 5 0 0 0 7.07 0l3.54-3.54a5 5 0 0 0-7.07-7.07l-1.41 1.41" />
      <path d="M14 11a5 5 0 0 0-7.07 0l-3.54 3.54a5 5 0 0 0 7.07 7.07l1.41-1.41" />
    </svg>
  );
}
function CheckIcon({ className }: { className?: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden className={className}>
      <path d="M20 6 9 17l-5-5" />
    </svg>
  );
}
function ReloadIcon({ className }: { className?: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden className={className}>
      <path d="M21 12a9 9 0 1 1-3-6.7" />
      <path d="M21 4v5h-5" />
    </svg>
  );
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
  const [copied, setCopied] = useState(false);
  // Local busy state — guards the Refresh button against concurrent presses
  // and drives its visible spin. Local (not store.statusText) because a
  // node-click sets statusText to 'Tracing…' too, and we don't want to
  // disable Refresh for unrelated activity.
  const [refreshing, setRefreshing] = useState(false);

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
      {/* nav cluster: brand + run pickers. Logo + wordmark form a single
          tight unit (negative gap pulls the word toward the mark), which
          reads as one brand instead of two stacked items. */}
      <div className="flex items-center gap-1.5 mr-1">
        <Logo size={26} className="text-accent" />
        <span className="font-extrabold text-base tracking-wide text-accent">Prism</span>
      </div>

      <div className="flex items-center gap-1.5">
        <label htmlFor="run-select" className="text-body text-muted">Run</label>
        <select
          id="run-select"
          className="px-2.5 py-1.5 text-body max-w-[260px] focus:outline-none focus:border-accent"
          value={run ?? ''}
          onChange={(e) => e.target.value && selectRun(Number(e.target.value))}
        >
          {!runs.length && <option value="">(no ingested runs)</option>}
          {runs.map((r) => (
            <option key={r.run} value={r.run} disabled={r.run === compareRun}>
              {/* Ecosystem first so users can tell cargo / npm / pypi / pnpm
                  runs apart at a glance — the bare numeric id was cryptic
                  with four-system comparisons in the workspace. */}
              {r.ecosystem ? `${r.ecosystem} · ` : ''}run {r.run}
              {r.nodes != null ? ` · ${r.nodes}n/${r.edges}e` : ''}
            </option>
          ))}
        </select>
      </div>

      <div className="flex items-center gap-1.5" title="Diff the active run against another run">
        <label htmlFor="compare-select" className="text-body text-muted">vs</label>
        <select
          id="compare-select"
          className="px-2.5 py-1.5 text-body max-w-[200px] focus:outline-none focus:border-accent"
          value={compareRun ?? ''}
          onChange={(e) => setCompareRun(e.target.value ? Number(e.target.value) : null)}
        >
          <option value="">(no compare)</option>
          {runs.map((r) => (
            <option key={r.run} value={r.run} disabled={r.run === run}>
              {r.ecosystem ? `${r.ecosystem} · ` : ''}run {r.run}
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
      <div className="relative flex items-center gap-1.5 flex-1 min-w-0 basis-[220px] max-w-[300px]">
        <input
          type="search"
          placeholder="Find a package…"
          aria-label="Find a package"
          autoComplete="off"
          spellCheck={false}
          value={query}
          className="w-full min-w-0 px-2.5 py-1.5 pr-9 text-body placeholder:text-muted focus:outline-none focus:border-accent"
          onChange={(e) => runSearch(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              runSearch('');
              (e.target as HTMLInputElement).blur();
            }
          }}
        />
        {/* Command-palette keybinding hint — sits inside the input frame,
            visible only when the input is empty so it doesn't compete with
            the user's typed query. */}
        {!query && (
          <kbd
            aria-hidden
            className="absolute right-2 top-1/2 -translate-y-1/2 px-1.5 py-0.5 text-[10px] font-mono leading-none rounded-badge border border-line-2 bg-panel text-muted pointer-events-none"
          >
            ⌘K
          </kbd>
        )}
        <span className="text-mini text-muted min-w-[28px]">
          {matches.size ? `${matches.size} hit${matches.size === 1 ? '' : 's'}` : ''}
        </span>
      </div>

      <button
        type="button"
        onClick={copyLink}
        title="Copy a deep link to this exact view"
        aria-label="Copy a deep link to this exact view"
        className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-body font-semibold rounded-control border border-line-2 bg-panel-2 text-ink hover:border-accent hover:bg-panel-3"
      >
        {copied ? (
          <>
            <CheckIcon /> Copied
          </>
        ) : (
          <>
            <LinkIcon /> Copy link
          </>
        )}
      </button>

      <button
        type="button"
        onClick={async () => {
          // Skip if already running OR there is no run to refresh.
          // We bypass loadRuns entirely: in the static+WASM model the runs
          // list is fixed by the loaded bundle, so re-fetching it cannot
          // change anything and only flashes a misleading 'Loading runs…'
          // status. Calling selectRun directly with `preserveUi: true`
          // does the only thing this button can meaningfully do — refresh
          // the current run's graph + side-panel data from the WASM
          // singleton, keeping the user's selection, search, panel tab,
          // and TDA feature highlight in place.
          if (refreshing || run == null) return;
          setRefreshing(true);
          try {
            await selectRun(run, { preserveUi: true });
          } finally {
            setRefreshing(false);
          }
        }}
        disabled={refreshing || run == null}
        aria-busy={refreshing}
        title={refreshing ? 'Refreshing…' : run == null ? 'No run to refresh' : 'Refresh view'}
        aria-label={refreshing ? 'Refreshing view' : 'Refresh view'}
        className="inline-flex items-center justify-center w-8 h-8 rounded-control border border-line-2 bg-panel-2 text-ink hover:border-accent hover:bg-panel-3 disabled:opacity-60 disabled:cursor-wait disabled:hover:border-line-2 disabled:hover:bg-panel-2"
      >
        <ReloadIcon className={refreshing ? 'animate-spin' : ''} />
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

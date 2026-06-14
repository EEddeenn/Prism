'use client';

import { useRef, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { EDGE_FILTER_LABELS } from '@/lib/edges';
import { SegmentedControl, type SegOption } from '@/components/ui';
import Logo from '@/components/Logo';
import { useLoadBundle } from '@/lib/useLoadBundle';
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
function UploadIcon({ className }: { className?: string }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden className={className}>
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <path d="m17 8-5-5-5 5" />
      <path d="M12 3v12" />
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

  // Bundle loading from the topbar (separate entry point from the landing
  // dropzone — uses the same shared hook). Errors surface as a destructive
  // tooltip + red border on the button; success replaces the URL and
  // triggers a full reload so the WASM singleton is released cleanly.
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const { status: loadStatus, loadFile } = useLoadBundle();
  const loading = loadStatus.kind === 'decoding';
  const loadError = loadStatus.kind === 'error' ? loadStatus.error : null;

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
          tight unit (negative gap pulls the word toward the mark) and act
          as a "return home" link — clicking either navigates to `/`, which
          drops the `?bundle=…` query so WasmBundleLoader re-renders the
          landing dropzone. Plain <a> (not next/link) for a full reload so
          the WASM singleton is released cleanly. `focusable` opts into the
          global keyboard focus ring. */}
      <a
        href="/"
        title="Return to home"
        aria-label="Prism — return to home"
        className="focusable flex items-center gap-1.5 mr-1 rounded-control hover:opacity-80 transition-opacity"
      >
        <Logo size={26} className="text-accent" />
        <span className="font-extrabold text-base tracking-wide text-accent">Prism</span>
      </a>

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

      {/* Standalone "vs" label — pulled out of the Compare cluster so the
          parent header's gap-3 puts an equal 12 px on BOTH sides. Inside
          a flex cluster with the Compare select (the previous structure)
          it was 12 px from the Run select but only 6 px from the Compare
          select, visually leaning right. htmlFor still ties it to the
          Compare picker for accessibility. */}
      <label
        htmlFor="compare-select"
        title="Diff the active run against another run"
        className="text-body text-muted"
      >
        vs
      </label>

      <select
        id="compare-select"
        title="Diff the active run against another run"
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
      <div className="flex items-center gap-1.5 flex-1 min-w-0 basis-[220px] max-w-[300px]">
        <input
          type="search"
          placeholder="Find a package…"
          aria-label="Find a package"
          autoComplete="off"
          spellCheck={false}
          value={query}
          className="w-full min-w-0 px-2.5 py-1.5 text-body placeholder:text-muted focus:outline-none focus:border-accent"
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

      {/* Load-bundle: opens the OS file picker and routes the chosen
          .prismbundle through the shared hook (same code path the landing
          dropzone uses). Sibling of Copy link so the two principal actions
          live together. The <input> is visually hidden but kept in the DOM
          so the button's onClick can `.click()` it. */}
      <input
        ref={fileInputRef}
        type="file"
        accept=".prismbundle,application/octet-stream"
        className="hidden"
        onChange={(e) => {
          const file = e.target.files?.[0];
          if (file) void loadFile(file);
          // Reset so selecting the same file twice in a row still fires.
          e.target.value = '';
        }}
      />
      <button
        type="button"
        onClick={() => fileInputRef.current?.click()}
        disabled={loading}
        title={loadError ?? (loading ? 'Decoding…' : 'Load a different .prismbundle')}
        aria-label={loading ? 'Decoding bundle' : 'Load a different bundle'}
        className={`inline-flex items-center gap-1.5 px-2.5 py-1.5 text-body font-semibold rounded-control border bg-panel-2 hover:bg-panel-3 disabled:opacity-60 disabled:cursor-wait ${
          loadError ? 'border-bad text-bad hover:border-bad' : 'border-line-2 text-ink hover:border-accent'
        }`}
      >
        <UploadIcon /> {loading ? 'Loading…' : 'Load bundle'}
      </button>

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

      <span
        className={`ml-auto text-body truncate max-w-[36%] ${statusError ? 'text-bad' : 'text-muted'}`}
        role={statusError ? 'alert' : 'status'}
      >
        {statusText}
      </span>
    </header>
  );
}

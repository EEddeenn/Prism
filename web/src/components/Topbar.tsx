'use client';

import { useRef, useState } from 'react';
import { Check, Link2, Upload } from 'lucide-react';
import { useGraphStore } from '@/store/useGraphStore';
import { EDGE_FILTER_LABELS } from '@/lib/edges';
import { SegmentedControl, Button, type SegOption } from '@/design-system';
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
  return <span aria-hidden className="self-stretch w-px bg-iron mx-1" />;
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
    <header className="flex flex-wrap items-center gap-3 px-4 py-2 border-b border-iron bg-charcoal">
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
        <Logo size={26} className="text-bone" />
        <span className="font-semibold text-h3 tracking-wide text-bone">Prism</span>
      </a>

      <div className="flex items-center gap-1.5">
        <label htmlFor="run-select" className="text-data text-archive">Run</label>
        <select
          id="run-select"
          className="px-2.5 py-1.5 text-data max-w-[260px] focus:border-cyan-gray"
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
        className="text-data text-archive"
      >
        vs
      </label>

      <select
        id="compare-select"
        title="Diff the active run against another run"
        className="px-2.5 py-1.5 text-data max-w-[200px] focus:border-cyan-gray"
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
        <span className="text-data text-archive">Edges</span>
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
          className="w-full min-w-0 px-2.5 py-1.5 text-data placeholder:text-archive focus:border-cyan-gray"
          onChange={(e) => runSearch(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              runSearch('');
              (e.target as HTMLInputElement).blur();
            }
          }}
        />
        <span className="font-mono tabular-nums text-meta text-archive min-w-[28px]">
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
      <Button
        variant={loadError ? 'critical' : 'secondary'}
        size="sm"
        onClick={() => fileInputRef.current?.click()}
        disabled={loading}
        title={loadError ?? (loading ? 'Decoding…' : 'Load a different .prismbundle')}
        aria-label={loading ? 'Decoding bundle' : 'Load a different bundle'}
        className="disabled:cursor-wait"
      >
        <Upload aria-hidden className="size-3.5" strokeWidth={1.8} /> {loading ? 'Loading…' : 'Load bundle'}
      </Button>

      <Button
        variant="secondary"
        size="sm"
        onClick={copyLink}
        title="Copy a deep link to this exact view"
        aria-label="Copy a deep link to this exact view"
      >
        {copied ? (
          <>
            <Check aria-hidden className="size-3.5" strokeWidth={1.8} /> Copied
          </>
        ) : (
          <>
            <Link2 aria-hidden className="size-3.5" strokeWidth={1.8} /> Copy link
          </>
        )}
      </Button>

      <span
        className={`ml-auto text-data truncate max-w-[36%] ${statusError ? 'text-bad' : 'text-archive'}`}
        role={statusError ? 'alert' : 'status'}
      >
        {statusText}
      </span>
    </header>
  );
}

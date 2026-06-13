'use client';

import { useGraphStore } from '@/store/useGraphStore';

export default function Topbar() {
  const runs = useGraphStore((s) => s.runs);
  const run = useGraphStore((s) => s.run);
  const view = useGraphStore((s) => s.view);
  const statusText = useGraphStore((s) => s.statusText);
  const statusError = useGraphStore((s) => s.statusError);
  const matches = useGraphStore((s) => s.matches);
  const query = useGraphStore((s) => s.query);
  const selectRun = useGraphStore((s) => s.selectRun);
  const setView = useGraphStore((s) => s.setView);
  const runSearch = useGraphStore((s) => s.runSearch);
  const loadRuns = useGraphStore((s) => s.loadRuns);

  return (
    <header className="flex flex-wrap items-center gap-3 px-4 py-2 border-b border-line bg-gradient-to-b from-[#181c28] to-panel">
      <span className="font-extrabold text-base tracking-wide text-accent">Prism</span>
      <span className="text-[11px] uppercase tracking-[1px] text-muted -ml-1.5 mr-1">
        dependency topology
      </span>

      <div className="flex items-center gap-1.5">
        <label htmlFor="run-select" className="text-xs text-muted">Run</label>
        <select
          id="run-select"
          className="px-2.5 py-1.5 text-sm max-w-[240px]"
          value={run ?? ''}
          onChange={(e) => e.target.value && selectRun(Number(e.target.value))}
        >
          {!runs.length && <option value="">(no ingested runs)</option>}
          {runs.map((r) => (
            <option key={r.run} value={r.run}>
              run {r.run}
              {r.nodes != null ? ` · ${r.nodes}n/${r.edges}e` : ''}
            </option>
          ))}
        </select>
      </div>

      <div className="flex" role="group" aria-label="View mode">
        {(['2d', '3d'] as const).map((v, i) => (
          <button
            key={v}
            type="button"
            onClick={() => setView(v)}
            className={`px-3 py-1.5 text-xs font-bold border border-line-2 ${
              i === 0 ? 'rounded-l-md border-r-0' : 'rounded-r-md'
            } ${
              view === v ? 'bg-accent text-[#06203f] border-accent' : 'bg-panel-2 text-muted'
            }`}
          >
            {v.toUpperCase()}
          </button>
        ))}
      </div>

      <div className="flex items-center gap-1.5 flex-1 min-w-0 basis-[200px] max-w-[280px]">
        <input
          type="search"
          placeholder="Find a package…"
          aria-label="Find a package"
          autoComplete="off"
          spellCheck={false}
          value={query}
          className="w-full min-w-0 px-2.5 py-1.5 text-sm placeholder:text-muted"
          onChange={(e) => runSearch(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              runSearch('');
              (e.target as HTMLInputElement).blur();
            }
          }}
        />
        <span className="text-[11px] text-muted min-w-[28px]">
          {matches.size ? `${matches.size} hit${matches.size === 1 ? '' : 's'}` : ''}
        </span>
      </div>

      <button type="button" onClick={() => loadRuns()} className="px-2.5 py-1.5 text-sm hover:border-accent">
        Reload
      </button>

      <span
        className={`ml-auto text-xs truncate max-w-[36%] ${statusError ? 'text-bad' : 'text-muted'}`}
        role="status"
      >
        {statusText}
      </span>
    </header>
  );
}

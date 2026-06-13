'use client';

import dynamic from 'next/dynamic';
import { useGraphStore } from '@/store/useGraphStore';
import { useUrlSync } from '@/lib/urlState';
import Topbar from '@/components/Topbar';
import Graph2D from '@/components/Graph2D';
import SidePanel from '@/components/SidePanel';
import CommandPalette from '@/components/CommandPalette';
import { PALETTE } from '@/lib/palette';

// Browser-only views (WebGL / sizing) — no SSR.
const Graph3D = dynamic(() => import('@/components/Graph3D'), { ssr: false });
const Treemap = dynamic(() => import('@/components/Treemap'), { ssr: false });

const LEGEND = [
  ['selected', PALETTE.center, false],
  ['depends on', PALETTE.fwd, false],
  ['depended on by', PALETTE.impact, false],
  ['duplicated', PALETTE.node, true],
  ['root', PALETTE.root, false],
] as const;

export default function Page() {
  useUrlSync(); // owns the initial load + two-way URL state

  const view = useGraphStore((s) => s.view);
  const nodes = useGraphStore((s) => s.nodes);
  const runs = useGraphStore((s) => s.runs);
  const run = useGraphStore((s) => s.run);
  const statusText = useGraphStore((s) => s.statusText);

  const loading = statusText.startsWith('Loading');
  const noRuns = !loading && runs.length === 0;
  const emptyGraph = !loading && run != null && nodes.length === 0;

  return (
    <div className="flex flex-col h-full">
      <Topbar />
      <main className="flex flex-1 min-h-0">
        <section className="relative flex-1 overflow-hidden">
          {view === '2d' && <Graph2D />}
          {view === '3d' && <Graph3D />}
          {view === 'map' && <Treemap />}

          {view !== 'map' && (
            <div className="absolute left-3.5 bottom-3.5 z-10 flex flex-wrap gap-x-3 gap-y-1 px-2.5 py-2 max-w-[60%] rounded-lg border border-line bg-panel/85 backdrop-blur-sm text-[11.5px] text-muted">
              {LEGEND.map(([label, color, ring]) => (
                <span key={label} className="inline-flex items-center gap-1.5">
                  <i
                    className="w-2.5 h-2.5 rounded-full inline-block"
                    style={ring ? { background: color, boxShadow: `0 0 0 2px ${PALETTE.dup}` } : { background: color }}
                  />
                  {label}
                </span>
              ))}
            </div>
          )}

          {noRuns && (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-1 text-center pointer-events-none">
              <p className="text-ink">No runs to show.</p>
              <p className="text-muted text-sm">
                Ingest a lockfile with <code className="bg-panel-2 px-1.5 rounded">prism ingest</code>, then reload.
              </p>
              <p className="text-muted text-xs mt-2">Is <code className="bg-panel-2 px-1.5 rounded">prism-serve</code> running on :7831?</p>
            </div>
          )}
          {emptyGraph && (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-1 text-center pointer-events-none">
              <p className="text-ink">This run has no graph nodes.</p>
              <p className="text-muted text-sm">Pick another run, or re-ingest its lockfile.</p>
            </div>
          )}

          <div className="absolute right-3.5 bottom-3.5 z-10 text-[11px] text-muted/70 pointer-events-none select-none">
            ⌘K
          </div>
        </section>
        <SidePanel />
      </main>
      <CommandPalette />
    </div>
  );
}

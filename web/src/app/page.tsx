'use client';

import dynamic from 'next/dynamic';
import { useGraphStore } from '@/store/useGraphStore';
import { useUrlSync } from '@/lib/urlState';
import Topbar from '@/components/Topbar';
import Graph2D from '@/components/Graph2D';
import SidePanel from '@/components/SidePanel';
import CommandPalette from '@/components/CommandPalette';
import BundleDropzone from '@/components/BundleDropzone';
import WasmBundleLoader from '@/components/WasmBundleLoader';
import { PALETTE } from '@/lib/palette';
import { importanceColor } from '@/lib/encoding';
import type { Mode3D, View } from '@/lib/types';

// Browser-only views (WebGL / sizing) — no SSR.
const Graph3D = dynamic(() => import('@/components/Graph3D'), { ssr: false });
const Treemap = dynamic(() => import('@/components/Treemap'), { ssr: false });

type LegendItem = { label: string; color: string; ring?: boolean; ringColor?: string };

const COMMON: LegendItem[] = [
  { label: 'selected', color: PALETTE.center },
  { label: 'depends on', color: PALETTE.fwd },
  { label: 'depended on by', color: PALETTE.impact },
  { label: 'root', color: PALETTE.root },
];

/** The legend mirrors what each view actually draws — 2D rings duplicates,
 *  but 3D renders them as solid pink spheres, and each 3D mode adds its own
 *  vocabulary (fiber pink, terrain height via the HUD, TDA gold). */
function legendFor(view: View, mode: Mode3D): LegendItem[] {
  if (view === '2d') return [...COMMON, { label: 'duplicated', color: PALETTE.node, ring: true }];
  switch (mode) {
    case 'fiber':
      return [
        { label: 'duplicated family', color: PALETTE.dup },
        { label: 'single version', color: PALETTE.muted },
        ...COMMON,
      ];
    case 'terrain':
      return [...COMMON, { label: 'duplicated', color: PALETTE.dup }];
    case 'tda':
      return [
        { label: 'H₀/H₁ generator', color: PALETTE.gen },
        { label: 'H₂ void', color: PALETTE.void },
        ...COMMON,
        { label: 'duplicated', color: PALETTE.dup },
      ];
    case 'layered':
    default:
      return [
        ...COMMON,
        { label: 'duplicated', color: PALETTE.dup },
        { label: 'cycle edge', color: PALETTE.impact },
      ];
  }
}

/** The actual explorer UI. Lives behind the WasmBundleLoader gate so it only
 *  mounts once the singleton workspace is open — that way `useUrlSync()`'s
 *  `loadRuns()` call hits a loaded workspace, not an empty one. */
function Explorer() {
  useUrlSync(); // owns the initial load + two-way URL state

  const view = useGraphStore((s) => s.view);
  const mode3d = useGraphStore((s) => s.mode3d);
  const nodes = useGraphStore((s) => s.nodes);
  const runs = useGraphStore((s) => s.runs);
  const run = useGraphStore((s) => s.run);
  const statusText = useGraphStore((s) => s.statusText);
  const matches = useGraphStore((s) => s.matches);

  const legend = legendFor(view, mode3d);
  if (matches.size > 0) legend.push({ label: 'search match', color: '#252a3a', ring: true, ringColor: PALETTE.accent });
  // Node size + neutral colour encode blast radius everywhere except terrain
  // (whose height legend, top-right, owns the elevation channel).
  const showImportance = view === '2d' || (view === '3d' && mode3d !== 'terrain');
  const importanceGradient = `linear-gradient(to right, ${[0, 0.5, 1]
    .map((t) => importanceColor(t))
    .join(', ')})`;

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
            <div className="absolute left-3.5 bottom-3.5 z-10 flex flex-wrap items-center gap-x-3 gap-y-1 px-2.5 py-2 max-w-[64%] rounded-card border border-line bg-panel/85 backdrop-blur-sm text-mini text-muted">
              {showImportance && (
                <span className="inline-flex items-center gap-1.5" title="Node size and resting colour both encode blast radius (transitive dependents).">
                  <i className="h-2 w-8 rounded-pill inline-block" style={{ background: importanceGradient }} />
                  size · colour = blast radius
                </span>
              )}
              {legend.map(({ label, color, ring, ringColor }) => (
                <span key={label} className="inline-flex items-center gap-1.5">
                  <i
                    className="w-2.5 h-2.5 rounded-pill inline-block"
                    style={ring ? { background: color, boxShadow: `0 0 0 2px ${ringColor ?? PALETTE.dup}` } : { background: color }}
                  />
                  {label}
                </span>
              ))}
            </div>
          )}

          {noRuns && (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 text-center">
              <p className="text-ink">No runs to show.</p>
              <BundleDropzone />
              <p className="text-muted text-xs">
                Or build a new bundle with{' '}
                <code className="bg-panel-2 px-1.5 rounded-badge">prism analyze</code> /{' '}
                <code className="bg-panel-2 px-1.5 rounded-badge">prism export bundle</code> and drop it here.
              </p>
            </div>
          )}
          {emptyGraph && (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-1 text-center pointer-events-none">
              <p className="text-ink">This run has no graph nodes.</p>
              <p className="text-muted text-sm">Pick another run from the topbar.</p>
            </div>
          )}

          {/* The 2D view owns the bottom-right corner with its overview minimap. */}
          {view !== '2d' && (
            <div className="absolute right-3.5 bottom-3.5 z-10 text-mini text-muted/70 pointer-events-none select-none">
              ⌘K
            </div>
          )}
        </section>
        <SidePanel />
      </main>
      <CommandPalette />
    </div>
  );
}

export default function Page() {
  return (
    <WasmBundleLoader>
      <Explorer />
    </WasmBundleLoader>
  );
}

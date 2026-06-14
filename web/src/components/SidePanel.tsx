'use client';

import { useEffect, useRef, useState, type ComponentType } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { TABS, TAB_LABELS, type TabId } from '@/lib/tabs';
import { TabNav, Eyebrow } from '@/design-system';
import OverviewTab from './panel/OverviewTab';
import RiskTab from './panel/RiskTab';
import StructureTab from './panel/StructureTab';
import NodeTab from './panel/NodeTab';
import DuplicatesTab from './panel/DuplicatesTab';
import CyclesTab from './panel/CyclesTab';
import TdaTab from './panel/TdaTab';
import DiffTab from './panel/DiffTab';

const PANELS: Record<TabId, ComponentType> = {
  overview: OverviewTab,
  structure: StructureTab,
  diff: DiffTab,
  risk: RiskTab,
  node: NodeTab,
  duplicates: DuplicatesTab,
  cycles: CyclesTab,
  tda: TdaTab,
};

// Resize constraints. Min keeps the vertical tab rail (128 px) usable plus
// at least ~130 px of content room; max keeps the graph from being squeezed
// off-screen on small laptops.
const MIN_WIDTH = 280;
const MAX_WIDTH = 720;
const DEFAULT_WIDTH = 360;
const STORAGE_KEY = 'prism.sidepanel.width';

function clampWidth(n: number): number {
  return Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, Math.round(n)));
}

function readStoredWidth(): number {
  if (typeof window === 'undefined') return DEFAULT_WIDTH;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_WIDTH;
    const n = Number(raw);
    return Number.isFinite(n) ? clampWidth(n) : DEFAULT_WIDTH;
  } catch {
    return DEFAULT_WIDTH;
  }
}

/** Chevron used by the collapse / expand toggle. Rotates 180° via a Tailwind
 *  transform so the same glyph means "open" and "close" without two SVGs. */
function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      className={`transition-transform ${open ? '' : 'rotate-180'}`}
    >
      <path d="m9 18 6-6-6-6" />
    </svg>
  );
}

export default function SidePanel() {
  // Guarded lookup: a stale URL or persisted store value can carry a tab id
  // that no longer exists. Fall back to overview instead of silently rendering
  // the wrong panel via an unchecked cast.
  const rawActiveTab = useGraphStore((s) => s.activeTab);
  const activeTab: TabId = rawActiveTab in PANELS ? (rawActiveTab as TabId) : 'overview';
  const setActiveTab = useGraphStore((s) => s.setActiveTab);
  const duplicates = useGraphStore((s) => s.duplicates);
  const sccs = useGraphStore((s) => s.sccs);
  const pairs = useGraphStore((s) => s.pairs);
  const compareRun = useGraphStore((s) => s.compareRun);

  // Local collapsed state — when true, the panel shrinks to a thin vertical
  // strip with just the toggle, returning the panel width to the graph for
  // users who want to focus on the topology. Local (not in the URL or store)
  // because the right answer is screen-size dependent and shouldn't bleed
  // into shared deep links.
  const [collapsed, setCollapsed] = useState(false);

  // Resizable width — drag the left-edge handle to change. Stored locally
  // in localStorage so the user's choice survives reloads. Hydrate AFTER
  // mount so static-export pre-render and first client paint agree on the
  // default width (avoids an SSR/CSR mismatch warning).
  const [width, setWidth] = useState<number>(DEFAULT_WIDTH);
  useEffect(() => {
    setWidth(readStoredWidth());
  }, []);

  // Drag bookkeeping — startX and startWidth captured at pointerdown so the
  // move handler computes width as a delta from the gesture origin (more
  // stable than reading current clientX vs current width every move).
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  const onResizeDown = (e: React.PointerEvent<HTMLDivElement>) => {
    // Left mouse / primary touch only — middle/right click never resizes.
    if (e.button !== 0) return;
    e.preventDefault();
    dragRef.current = { startX: e.clientX, startWidth: width };
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onResizeMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragRef.current) return;
    // Handle sits on the LEFT edge; dragging right narrows the panel,
    // dragging left widens it. So new width = startWidth - delta.
    const delta = e.clientX - dragRef.current.startX;
    setWidth(clampWidth(dragRef.current.startWidth - delta));
  };

  const onResizeUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragRef.current) return;
    dragRef.current = null;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      /* capture may already be released after a pointercancel */
    }
    // Persist on gesture end so the storage write happens once, not on
    // every mousemove.
    try {
      window.localStorage.setItem(STORAGE_KEY, String(width));
    } catch {
      /* quota / private mode / disabled storage — silently ignore */
    }
  };

  const counts: Partial<Record<TabId, number>> = {
    duplicates: duplicates.length,
    cycles: sccs.filter((c) => c.is_cycle).length,
    tda: pairs.length,
    diff: compareRun != null ? 1 : 0,
  };

  const ActivePanel = PANELS[activeTab];
  const titleLabel = TAB_LABELS[activeTab] ?? 'Inspector';

  if (collapsed) {
    // Collapsed strip: a single button column hugging the right edge with
    // the chevron oriented to invite expansion. Keeps a visible separator
    // between the graph and the (now-hidden) inspector.
    return (
      <aside className="flex flex-col w-9 shrink-0 bg-charcoal border-l border-iron min-h-0">
        <button
          type="button"
          onClick={() => setCollapsed(false)}
          title="Show inspector panel"
          aria-label="Show inspector panel"
          aria-expanded={false}
          className="w-full h-9 flex items-center justify-center text-archive hover:text-bone hover:bg-model-black transition-colors border-b border-iron"
        >
          <Chevron open={false} />
        </button>
      </aside>
    );
  }

  return (
    <aside
      className="relative flex flex-col shrink-0 bg-charcoal border-l border-iron min-h-0"
      style={{ width }}
    >
      {/* Drag-to-resize handle. A 6-px-wide invisible target on the very
          left edge so it's easy to grab without intruding visually; on
          hover/drag a 1-px accent line lights up where the panel's left
          border already sits. The role + aria attributes expose the same
          semantic an ARIA separator does for screen readers; keyboard
          resize is a follow-up. */}
      <div
        onPointerDown={onResizeDown}
        onPointerMove={onResizeMove}
        onPointerUp={onResizeUp}
        onPointerCancel={onResizeUp}
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize inspector panel"
        aria-valuemin={MIN_WIDTH}
        aria-valuemax={MAX_WIDTH}
        aria-valuenow={width}
        title="Drag to resize"
        className="absolute left-0 top-0 bottom-0 w-1.5 -ml-px cursor-col-resize z-20 hover:bg-cyan-gray/40 active:bg-cyan-gray/60 transition-colors"
      />

      {/* Panel header: a slim row identifying the surface ("Inspector") and
          surfacing the active tab title, plus the collapse toggle. Gives the
          right rail a clear identity instead of dropping straight into tabs. */}
      <div className="flex items-center justify-between gap-2 px-3.5 h-9 border-b border-iron bg-model-black/40">
        <div className="flex items-baseline gap-2 min-w-0">
          <Eyebrow>Inspector</Eyebrow>
          <span className="text-data text-bone truncate" title={titleLabel}>
            {titleLabel}
          </span>
        </div>
        <button
          type="button"
          onClick={() => setCollapsed(true)}
          title="Hide inspector panel"
          aria-label="Hide inspector panel"
          aria-expanded
          className="shrink-0 w-6 h-6 flex items-center justify-center rounded-control text-archive hover:text-bone hover:bg-model-black transition-colors"
        >
          <Chevron open />
        </button>
      </div>

      {/* Vertical tab rail + content. The rail eats ~128 px so the eight
          tab labels can render in full ("Overview", "Duplicates", …) rather
          than fight over ~45 px each in a horizontal strip — and the
          inspector gains visible "what's here" navigation at a glance. */}
      <div className="flex flex-1 min-h-0">
        <TabNav
          tabs={TABS}
          active={activeTab}
          onSelect={(id) => setActiveTab(id)}
          counts={counts}
          orientation="vertical"
        />
        <div
          id={`tabpanel-${activeTab}`}
          role="tabpanel"
          aria-labelledby={`tab-${activeTab}`}
          className="flex-1 min-w-0 p-3.5 overflow-y-auto min-h-0 prism-scrollbar"
        >
          <ActivePanel />
        </div>
      </div>
    </aside>
  );
}

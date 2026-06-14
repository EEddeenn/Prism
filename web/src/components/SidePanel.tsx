'use client';

import { useState, type ComponentType } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { TABS, TAB_LABELS, type TabId } from '@/lib/tabs';
import { TabNav } from '@/components/ui';
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
  // strip with just the toggle, returning the 360 px of width to the graph
  // for users who want to focus on the topology. Local (not in the URL or
  // store) because the right answer is screen-size dependent and shouldn't
  // bleed into shared deep links.
  const [collapsed, setCollapsed] = useState(false);

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
      <aside className="flex flex-col w-9 shrink-0 bg-panel border-l border-line min-h-0">
        <button
          type="button"
          onClick={() => setCollapsed(false)}
          title="Show inspector panel"
          aria-label="Show inspector panel"
          aria-expanded={false}
          className="w-full h-9 flex items-center justify-center text-muted hover:text-ink hover:bg-panel-2 transition-colors border-b border-line"
        >
          <Chevron open={false} />
        </button>
      </aside>
    );
  }

  return (
    <aside className="flex flex-col w-[360px] shrink-0 bg-panel border-l border-line min-h-0">
      {/* Panel header: a slim row identifying the surface ("Inspector") and
          surfacing the active tab title, plus the collapse toggle. Gives the
          right rail a clear identity instead of dropping straight into tabs. */}
      <div className="flex items-center justify-between gap-2 px-3.5 h-9 border-b border-line bg-panel-2/40">
        <div className="flex items-baseline gap-2 min-w-0">
          <span className="text-mini uppercase tracking-[1.5px] text-muted">Inspector</span>
          <span className="text-body text-ink truncate" title={titleLabel}>
            {titleLabel}
          </span>
        </div>
        <button
          type="button"
          onClick={() => setCollapsed(true)}
          title="Hide inspector panel"
          aria-label="Hide inspector panel"
          aria-expanded
          className="shrink-0 w-6 h-6 flex items-center justify-center rounded text-muted hover:text-ink hover:bg-panel-3 transition-colors"
        >
          <Chevron open />
        </button>
      </div>

      <TabNav tabs={TABS} active={activeTab} onSelect={(id) => setActiveTab(id)} counts={counts} />
      <div
        id={`tabpanel-${activeTab}`}
        role="tabpanel"
        aria-labelledby={`tab-${activeTab}`}
        className="p-3.5 overflow-y-auto min-h-0 prism-scrollbar"
      >
        <ActivePanel />
      </div>
    </aside>
  );
}

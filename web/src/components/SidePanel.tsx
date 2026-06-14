'use client';

import type { ComponentType } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { TABS, type TabId } from '@/lib/tabs';
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

  const counts: Partial<Record<TabId, number>> = {
    duplicates: duplicates.length,
    cycles: sccs.filter((c) => c.is_cycle).length,
    tda: pairs.length,
    diff: compareRun != null ? 1 : 0,
  };

  const ActivePanel = PANELS[activeTab];

  return (
    <aside className="flex flex-col w-[360px] shrink-0 bg-panel border-l border-line min-h-0">
      <TabNav tabs={TABS} active={activeTab} onSelect={(id) => setActiveTab(id)} counts={counts} />
      <div
        id={`tabpanel-${activeTab}`}
        role="tabpanel"
        aria-labelledby={`tab-${activeTab}`}
        className="p-3.5 overflow-y-auto min-h-0"
      >
        <ActivePanel />
      </div>
    </aside>
  );
}

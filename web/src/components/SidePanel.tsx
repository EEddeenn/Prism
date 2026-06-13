'use client';

import { useGraphStore } from '@/store/useGraphStore';
import OverviewTab from './panel/OverviewTab';
import RiskTab from './panel/RiskTab';
import NodeTab from './panel/NodeTab';
import DuplicatesTab from './panel/DuplicatesTab';
import CyclesTab from './panel/CyclesTab';
import TdaTab from './panel/TdaTab';

const TABS = ['overview', 'risk', 'node', 'duplicates', 'cycles', 'tda'] as const;
const LABELS: Record<(typeof TABS)[number], string> = {
  overview: 'Overview',
  risk: 'Risk',
  node: 'Node',
  duplicates: 'Duplicates',
  cycles: 'Cycles',
  tda: 'TDA',
};

export default function SidePanel() {
  const activeTab = useGraphStore((s) => s.activeTab);
  const setActiveTab = useGraphStore((s) => s.setActiveTab);
  const duplicates = useGraphStore((s) => s.duplicates);
  const sccs = useGraphStore((s) => s.sccs);
  const pairs = useGraphStore((s) => s.pairs);

  const badge: Record<string, number> = {
    duplicates: duplicates.length,
    cycles: sccs.filter((c) => c.is_cycle).length,
    tda: pairs.length,
  };

  return (
    <aside className="flex flex-col w-[360px] shrink-0 bg-panel border-l border-line min-h-0">
      <nav className="flex border-b border-line bg-panel sticky top-0 z-10" role="tablist">
        {TABS.map((t) => (
          <button
            key={t}
            role="tab"
            onClick={() => setActiveTab(t)}
            className={`flex-1 px-1 py-2.5 text-xs font-semibold border-b-2 bg-transparent rounded-none ${
              activeTab === t ? 'text-ink border-accent' : 'text-muted border-transparent hover:bg-panel-2'
            }`}
          >
            {LABELS[t]}
            {badge[t] ? (
              <span
                className={`ml-1.5 inline-block min-w-[16px] px-1.5 rounded-lg text-[10px] leading-[15px] ${
                  activeTab === t ? 'bg-accent text-[#06203f]' : 'bg-panel-3 text-muted'
                }`}
              >
                {badge[t]}
              </span>
            ) : null}
          </button>
        ))}
      </nav>

      <div className="p-3.5 overflow-y-auto min-h-0">
        {activeTab === 'overview' && <OverviewTab />}
        {activeTab === 'risk' && <RiskTab />}
        {activeTab === 'node' && <NodeTab />}
        {activeTab === 'duplicates' && <DuplicatesTab />}
        {activeTab === 'cycles' && <CyclesTab />}
        {activeTab === 'tda' && <TdaTab />}
      </div>
    </aside>
  );
}

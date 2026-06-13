'use client';

import { useMemo } from 'react';
import { useGraphStore } from '@/store/useGraphStore';

export default function DuplicatesTab() {
  const clusters = useGraphStore((s) => s.duplicates);
  const nodes = useGraphStore((s) => s.nodes);
  const highlightSet = useGraphStore((s) => s.highlightSet);

  const purlToId = useMemo(() => {
    const m = new Map<string, number>();
    for (const n of nodes) m.set(n.purl, n.id);
    return m;
  }, [nodes]);

  if (!clusters.length) {
    return <p className="text-muted text-sm">No package family appears at more than one version. 🎉</p>;
  }

  return (
    <div>
      <p className="text-muted text-sm mb-2">
        {clusters.length} famil{clusters.length === 1 ? 'y' : 'ies'} present at multiple versions.
      </p>
      {clusters.map((c) => {
        const ids = c.versions.map((v) => purlToId.get(v.purl)).filter((x): x is number => x != null);
        return (
          <div key={c.package_id} className="border border-line rounded-lg mb-2.5 bg-panel-2 overflow-hidden">
            <div
              onClick={() => highlightSet(ids, c.package_purl)}
              className="flex items-center gap-2 px-2.5 py-2 cursor-pointer hover:bg-panel-3"
            >
              <span className="font-mono text-xs truncate">{c.package_purl.replace(/^pkg:[^/]+\//, '')}</span>
              <span className="ml-auto shrink-0 text-[11px] text-warn bg-[#20180c] border border-[#4a3a1e] rounded-xl px-2 py-px">
                {c.version_count} versions
              </span>
            </div>
            <div className="px-2.5 pb-2.5 pt-0.5">
              {c.versions.map((v) => (
                <div key={v.version_id} className="font-mono text-[11px] text-muted py-0.5">
                  {v.purl.split('@')[1] ?? v.purl}
                </div>
              ))}
            </div>
          </div>
        );
      })}
    </div>
  );
}

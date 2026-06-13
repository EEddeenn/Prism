'use client';

import { useGraphStore } from '@/store/useGraphStore';
import { shortLabel } from '@/lib/palette';

export default function CyclesTab() {
  const sccs = useGraphStore((s) => s.sccs);
  const selectNode = useGraphStore((s) => s.selectNode);

  const cycles = sccs.filter((c) => c.is_cycle);

  if (!sccs.length) {
    return <p className="text-muted text-sm">Pick a run.</p>;
  }
  if (!cycles.length) {
    return (
      <div className="px-2.5 py-2 rounded-lg text-[12.5px] bg-[#0f2419] border border-[#1d4a32] text-ok">
        Acyclic ✓ — {sccs.length} strongly-connected components, each a single node.
      </div>
    );
  }

  return (
    <div>
      <div className="px-2.5 py-2 rounded-lg text-[12.5px] mb-2.5 bg-[#2a1414] border border-[#5a2424] text-bad">
        {cycles.length} dependency cycle{cycles.length === 1 ? '' : 's'} found.
      </div>
      {cycles.map((c, i) => (
        <div key={i} className="border border-line rounded-lg px-2.5 py-2 mb-2">
          <div className="text-muted text-xs mb-1">{c.members.length} members:</div>
          {c.members.map((m) => (
            <div
              key={m.id}
              title={m.purl}
              onClick={() => selectNode(m.id)}
              className="font-mono text-[11.5px] py-px cursor-pointer hover:text-accent"
            >
              {shortLabel(m.purl)}
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

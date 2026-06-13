'use client';

import { useGraphStore } from '@/store/useGraphStore';
import { shortLabel } from '@/lib/palette';
import type { GraphNode } from '@/lib/types';

function PeerList({ title, ids }: { title: string; ids: number[] }) {
  const byId = useGraphStore((s) => s.index?.byId);
  const selectNode = useGraphStore((s) => s.selectNode);

  const seen = new Set<number>();
  const items = ids
    .filter((id) => !seen.has(id) && seen.add(id))
    .map((id) => byId?.get(id))
    .filter((n): n is GraphNode => !!n)
    .sort((a, b) => a.depth - b.depth || a.purl.localeCompare(b.purl));

  return (
    <div>
      <h3 className="mt-4 mb-1.5 text-[11px] uppercase tracking-wide text-muted">
        {title} <span className="font-normal">({items.length})</span>
      </h3>
      {!items.length ? (
        <p className="text-muted text-xs">
          {title === 'Dependencies' ? 'A leaf — depends on nothing.' : 'A root — nothing depends on it.'}
        </p>
      ) : (
        <ul className="list-none m-0 p-0">
          {items.map((n) => (
            <li
              key={n.id}
              title={n.purl}
              onClick={() => selectNode(n.id)}
              className="flex items-center gap-2 px-2 py-1.5 my-[3px] rounded-md bg-panel-2 cursor-pointer border border-transparent hover:border-accent hover:bg-panel-3"
            >
              <span className="font-mono text-[11.5px] truncate">{shortLabel(n.purl)}</span>
              <span className="ml-auto text-[10.5px] text-muted shrink-0">d{n.depth}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export default function NodeTab() {
  const sel = useGraphStore((s) => s.sel);
  const index = useGraphStore((s) => s.index);
  const dup = useGraphStore((s) => s.dupVersionIds);

  if (sel == null || !index) {
    return <p className="text-muted text-sm">Click a node in the graph.</p>;
  }
  const node = index.byId.get(sel);
  if (!node) return <p className="text-muted text-sm">Node not found.</p>;

  const rows: [string, string][] = [
    ['node id', String(node.id)],
    ['version id', String(node.version_id)],
    ['package id', node.package_id != null ? String(node.package_id) : '—'],
    ['depth', String(node.depth)],
    ['duplicated', dup.has(node.version_id) ? 'yes' : 'no'],
  ];

  return (
    <div>
      <div className="font-mono text-[12.5px] text-accent-2 break-all mb-2">{node.purl}</div>
      {rows.map(([k, v]) => (
        <div key={k} className="flex justify-between gap-2.5 my-[3px]">
          <span className="text-muted">{k}</span>
          <span className="tabular-nums">{v}</span>
        </div>
      ))}
      <PeerList title="Dependencies" ids={index.out.get(node.id) ?? []} />
      <PeerList title="Dependents" ids={index.inn.get(node.id) ?? []} />
    </div>
  );
}

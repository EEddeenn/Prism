'use client';

import { useGraphStore } from '@/store/useGraphStore';
import { shortLabel } from '@/lib/palette';
import { kindName, kindColor, flagNames } from '@/lib/edges';
import type { GraphNode } from '@/lib/types';
import type { GraphIndex } from '@/lib/graph';
import CandidateStrip from '@/components/CandidateStrip';

/** BFS upward from `sel` over `inn` to the nearest depth-0 root, then
 *  reconstruct the shortest path root -> ... -> sel. Returns node ids
 *  ordered root..node, or null if no root is reachable upward. */
function pathToRoot(index: GraphIndex, sel: number): number[] | null {
  if (index.rootIds.has(sel)) return [sel];

  const prev = new Map<number, number>(); // child -> parent-toward-root
  const queue: number[] = [sel];
  const seen = new Set<number>([sel]);
  let hit: number | null = null;

  while (queue.length) {
    const cur = queue.shift()!;
    if (index.rootIds.has(cur)) {
      hit = cur;
      break;
    }
    for (const up of index.inn.get(cur) ?? []) {
      if (seen.has(up)) continue;
      seen.add(up);
      prev.set(up, cur);
      queue.push(up);
    }
  }

  if (hit == null) return null;
  // Walk back down from the root toward sel using prev (root -> ... -> sel).
  const path: number[] = [];
  let cur: number | undefined = hit;
  while (cur != null) {
    path.push(cur);
    cur = prev.get(cur);
  }
  return path; // already ordered root..sel
}

function Crumbs({ index, sel }: { index: GraphIndex; sel: number }) {
  const selectNode = useGraphStore((s) => s.selectNode);

  if (index.rootIds.has(sel)) {
    return (
      <div className="mb-2 text-[11px] text-muted">
        This node <span className="text-accent-2">is a root</span>.
      </div>
    );
  }

  const path = pathToRoot(index, sel);
  if (!path || path.length < 2) {
    return (
      <div className="mb-2 text-[11px] text-muted">No path to a root (orphan).</div>
    );
  }

  return (
    <div className="mb-2 flex flex-wrap items-center gap-x-1 gap-y-0.5 text-[11px]">
      {path.map((id, i) => {
        const n = index.byId.get(id);
        const label = n ? shortLabel(n.purl) : `#${id}`;
        const isLast = i === path.length - 1;
        return (
          <span key={id} className="flex items-center gap-x-1">
            {i > 0 && <span className="text-line-2 select-none">›</span>}
            <span
              title={n?.purl}
              onClick={() => selectNode(id)}
              className={
                'font-mono cursor-pointer ' +
                (isLast ? 'text-accent-2' : 'text-muted hover:text-accent')
              }
            >
              {label}
            </span>
          </span>
        );
      })}
    </div>
  );
}

const FLAG_CHIP: Record<string, string> = {
  'peer-unmet': 'text-bad bg-[#2a1414] border-[#5a2424]',
  overridden: 'text-warn bg-[#20180c] border-[#4a3a1e]',
  'optional-omitted': 'text-muted bg-panel-2 border-line',
  'back-edge': 'text-muted bg-panel-2 border-line',
};

/** Incoming dependents, each annotated with its edge kind tag + smell chips,
 *  resolved from edgeMeta key `${dependentId}>${sel}`. */
function Provenance({ index, sel }: { index: GraphIndex; sel: number }) {
  const selectNode = useGraphStore((s) => s.selectNode);

  const seen = new Set<number>();
  const items = (index.inn.get(sel) ?? [])
    .filter((id) => !seen.has(id) && seen.add(id))
    .map((id) => {
      const n = index.byId.get(id);
      const meta = index.edgeMeta.get(`${id}>${sel}`);
      return n ? { node: n, meta } : null;
    })
    .filter((x): x is { node: GraphNode; meta: { kind: number; flags: number } | undefined } => !!x)
    .sort((a, b) => a.node.depth - b.node.depth || a.node.purl.localeCompare(b.node.purl));

  return (
    <div>
      <h3 className="mt-4 mb-1.5 text-[11px] uppercase tracking-wide text-muted">
        Provenance <span className="font-normal">({items.length})</span>
      </h3>
      {!items.length ? (
        <p className="text-muted text-xs">Nothing depends on this node.</p>
      ) : (
        <ul className="list-none m-0 p-0">
          {items.map(({ node, meta }) => {
            const flags = meta ? flagNames(meta.flags) : [];
            return (
              <li
                key={node.id}
                title={node.purl}
                onClick={() => selectNode(node.id)}
                className="flex flex-wrap items-center gap-1.5 px-2 py-1.5 my-[3px] rounded-md bg-panel-2 cursor-pointer border border-transparent hover:border-accent hover:bg-panel-3"
              >
                <span className="font-mono text-[11.5px] truncate">{shortLabel(node.purl)}</span>
                {meta && (
                  <span
                    className="shrink-0 rounded px-1.5 py-px text-[10px] font-mono border"
                    style={{ color: kindColor(meta.kind), borderColor: kindColor(meta.kind) }}
                  >
                    {kindName(meta.kind)}
                  </span>
                )}
                {flags.map((f) => (
                  <span
                    key={f}
                    className={
                      'shrink-0 rounded-xl px-1.5 py-px text-[10px] border ' +
                      (FLAG_CHIP[f] ?? FLAG_CHIP['optional-omitted'])
                    }
                  >
                    {f}
                  </span>
                ))}
                <span className="ml-auto text-[10.5px] text-muted shrink-0">d{node.depth}</span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

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
  const requestFocus = useGraphStore((s) => s.requestFocus);

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
      <Crumbs index={index} sel={sel} />

      <div className="flex items-start gap-2 mb-2">
        <div className="font-mono text-[12.5px] text-accent-2 break-all flex-1">{node.purl}</div>
        <button
          onClick={() => requestFocus(sel)}
          className="shrink-0 rounded-md px-2 py-1 text-[11px] bg-panel-2 border border-line text-muted hover:border-accent hover:text-accent"
        >
          Recenter
        </button>
      </div>

      {rows.map(([k, v]) => (
        <div key={k} className="flex justify-between gap-2.5 my-[3px]">
          <span className="text-muted">{k}</span>
          <span className="tabular-nums">{v}</span>
        </div>
      ))}

      <h3 className="mt-4 mb-1.5 text-[11px] uppercase tracking-wide text-muted">Version headroom</h3>
      <CandidateStrip nodePurl={node.purl} />

      <PeerList title="Dependencies" ids={index.out.get(node.id) ?? []} />
      <PeerList title="Dependents" ids={index.inn.get(node.id) ?? []} />
      <Provenance index={index} sel={sel} />
    </div>
  );
}

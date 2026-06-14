'use client';

import { useEffect, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { api, ApiError } from '@/lib/api';
import { shortLabel } from '@/lib/palette';
import { kindName, flagNames } from '@/lib/edges';
import { edgeKindTone, flagTone } from '@/lib/tone';
import type { GraphNode, Explanation } from '@/lib/types';
import type { GraphIndex } from '@/lib/graph';
import CandidateStrip from '@/components/CandidateStrip';
import {
  SectionHeader,
  ListRow,
  Row,
  Badge,
  Button,
  Callout,
  PanelState,
  PathTrail,
  PurlLabel,
} from '@/design-system';

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
      <div className="mb-2 text-data-s text-archive">
        This node <span className="text-node-match">is a root</span>.
      </div>
    );
  }

  const path = pathToRoot(index, sel);
  if (!path || path.length < 2) {
    return <div className="mb-2 text-data-s text-archive">No path to a root (orphan).</div>;
  }

  return (
    <PathTrail
      ariaLabel="path to root"
      className="mb-2"
      items={path.map((id) => {
        const n = index.byId.get(id);
        const label = n ? shortLabel(n.purl) : `#${id}`;
        return { label, purl: n?.purl, onSelect: () => selectNode(id) };
      })}
    />
  );
}

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
      <SectionHeader count={items.length}>Provenance</SectionHeader>
      {!items.length ? (
        <p className="text-archive text-data-s">Nothing depends on this node.</p>
      ) : (
        <ul className="list-none m-0 p-0 flex flex-col gap-[3px]">
          {items.map(({ node, meta }) => {
            const flags = meta ? flagNames(meta.flags) : [];
            const label = shortLabel(node.purl);
            return (
              <li key={node.id}>
                <ListRow
                  onSelect={() => selectNode(node.id)}
                  ariaLabel={`Select ${label}`}
                  title={node.purl}
                  className="flex items-center flex-wrap gap-1.5"
                >
                  <span className="font-mono text-data-s truncate">{label}</span>
                  {meta && (
                    <Badge
                      mono
                      tone={edgeKindTone(kindName(meta.kind))}
                      title={`kind: ${kindName(meta.kind)}`}
                      className="shrink-0"
                    >
                      {kindName(meta.kind)}
                    </Badge>
                  )}
                  {flags.map((f) => (
                    <Badge key={f} tone={flagTone(f)} className="shrink-0">
                      {f}
                    </Badge>
                  ))}
                  <span className="ml-auto text-meta text-archive shrink-0">d{node.depth}</span>
                </ListRow>
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
      <SectionHeader count={items.length}>{title}</SectionHeader>
      {!items.length ? (
        <p className="text-archive text-data-s">
          {title === 'Dependencies'
            ? 'A leaf — depends on nothing.'
            : 'A root — nothing depends on it.'}
        </p>
      ) : (
        <ul className="list-none m-0 p-0 flex flex-col gap-[3px]">
          {items.map((n) => {
            const label = shortLabel(n.purl);
            return (
              <li key={n.id}>
                <ListRow
                  onSelect={() => selectNode(n.id)}
                  ariaLabel={`Select ${label}`}
                  title={n.purl}
                  className="flex items-center"
                >
                  <span className="font-mono text-data-s truncate">{label}</span>
                  <span className="ml-auto text-meta text-archive shrink-0">d{n.depth}</span>
                </ListRow>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

/**
 * "Why is this here?" — server-computed provenance for the selected node:
 * runtime reachability, nearest first-party owner, the edges that introduce it,
 * and a few shortest root → target chains. Lazily fetched per [run, sel].
 */
function WhyHere({ run, sel }: { run: number; sel: number }) {
  const selectNode = useGraphStore((s) => s.selectNode);

  const [why, setWhy] = useState<Explanation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let live = true;
    setLoading(true);
    setError(null);
    setWhy(null);
    api
      .why(run, sel)
      .then((r) => {
        if (!live) return;
        setWhy(r);
      })
      .catch((e) => {
        if (!live) return;
        setError(e instanceof ApiError ? e.message : 'failed to load provenance');
      })
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [run, sel]);

  return (
    <div>
      <SectionHeader>Why is this here?</SectionHeader>
      {error ? (
        <Callout tone="bad" role="alert">{error}</Callout>
      ) : loading && !why ? (
        <PanelState kind="loading">Tracing provenance…</PanelState>
      ) : !why ? (
        <PanelState kind="empty">No provenance available.</PanelState>
      ) : (
        <div className="flex flex-col gap-2">
          {/* Reachability status */}
          <div className="flex flex-wrap items-center gap-1.5 text-data-s">
            {why.production_reachable ? (
              <Badge tone="good">runtime-reachable</Badge>
            ) : why.non_production_only ? (
              <Badge tone="warn">dev/build-only</Badge>
            ) : (
              <Badge tone="neutral">unreachable from roots</Badge>
            )}
          </div>

          {/* Nearest first-party owner */}
          {why.nearest_owner && (
            <div className="text-data-s text-archive">
              nearest first-party owner:{' '}
              <PurlLabel
                purl={why.nearest_owner.purl}
                onSelect={() => selectNode(why.nearest_owner!.id)}
                ariaLabel={`Select ${shortLabel(why.nearest_owner.purl)}`}
                className="text-bone hover:text-cyan-gray"
              />{' '}
              · {why.owner_distance} {why.owner_distance === 1 ? 'hop' : 'hops'}
            </div>
          )}

          {/* Introducers */}
          <div>
            <div className="text-meta text-archive mb-1">pulled in by</div>
            {why.introducers.length === 0 ? (
              <p className="text-archive text-data-s">
                This is a root / first-party package (no parents).
              </p>
            ) : (
              <ul className="list-none m-0 p-0 flex flex-col gap-[3px]">
                {why.introducers.map(({ node, kinds }) => {
                  const label = shortLabel(node.purl);
                  return (
                    <li key={node.id}>
                      <ListRow
                        onSelect={() => selectNode(node.id)}
                        ariaLabel={`Select ${label}`}
                        title={node.purl}
                        className="flex items-center flex-wrap gap-1.5"
                      >
                        <span className="font-mono text-data-s truncate">{label}</span>
                        {kinds.map((k) => (
                          <Badge key={k} mono tone={edgeKindTone(k)} className="shrink-0">
                            {k}
                          </Badge>
                        ))}
                        <span className="ml-auto text-meta text-archive shrink-0">d{node.depth}</span>
                      </ListRow>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>

          {/* Shortest root → target chains */}
          {why.paths.length > 0 && (
            <div>
              <div className="text-meta text-archive mb-1">shortest paths from a root</div>
              <ul className="list-none m-0 p-0 flex flex-col gap-1">
                {why.paths.slice(0, 3).map((path, pi) => (
                  <li key={pi} className="px-2.5 py-1.5 rounded-control bg-model-black">
                    <PathTrail
                      sep="→"
                      ariaLabel="shortest path from a root"
                      items={path.map((n) => ({
                        label: shortLabel(n.purl),
                        purl: n.purl,
                        onSelect: () => selectNode(n.id),
                      }))}
                    />
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default function NodeTab() {
  const sel = useGraphStore((s) => s.sel);
  const run = useGraphStore((s) => s.run);
  const index = useGraphStore((s) => s.index);
  const dup = useGraphStore((s) => s.dupVersionIds);
  const requestFocus = useGraphStore((s) => s.requestFocus);

  if (sel == null || !index) {
    return <PanelState kind="prompt">Click a node in the graph.</PanelState>;
  }
  const node = index.byId.get(sel);
  if (!node) return <PanelState kind="empty">Node not found.</PanelState>;

  const duplicated = dup.has(node.version_id);
  const rows: { label: string; value: string; tone?: string }[] = [
    { label: 'node id', value: String(node.id) },
    { label: 'version id', value: String(node.version_id) },
    { label: 'package id', value: node.package_id != null ? String(node.package_id) : '—' },
    { label: 'depth', value: String(node.depth) },
    { label: 'duplicated', value: duplicated ? 'yes' : 'no', tone: duplicated ? 'text-warn' : undefined },
  ];

  return (
    <div>
      <Crumbs index={index} sel={sel} />

      <div className="flex items-start gap-2 mb-2">
        <div className="font-mono text-h4 text-node-match break-all flex-1">{node.purl}</div>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => requestFocus(sel)}
          className="shrink-0"
        >
          Recenter
        </Button>
      </div>

      {rows.map(({ label, value, tone }) => (
        <Row key={label} label={label} value={value} valueTone={tone} />
      ))}

      {run != null && <WhyHere key={`${run}:${sel}`} run={run} sel={sel} />}

      <SectionHeader>Version headroom</SectionHeader>
      <CandidateStrip nodePurl={node.purl} />

      <PeerList title="Dependencies" ids={index.out.get(node.id) ?? []} />
      <PeerList title="Dependents" ids={index.inn.get(node.id) ?? []} />
      <Provenance index={index} sel={sel} />
    </div>
  );
}

'use client';

import { useEffect, useState } from 'react';
import { api } from '@/lib/api';
import { PALETTE, shortLabel } from '@/lib/palette';
import type { CandidatesResp } from '@/lib/types';

/** Version "headroom" number-line for one package, shown inside the Node tab.
 *
 * The strip lays out every known version of a package as a row of cells,
 * oldest (left) → newest (right). Cells inside a satisfying range read as
 * accent; excluded cells are hatched + muted (with the reason on hover); the
 * currently-resolved version gets a bright ring. It answers "how much room is
 * there to move, and why is the rest off-limits?" at a glance. */
export default function CandidateStrip({ nodePurl }: { nodePurl: string }) {
  const at = nodePurl.lastIndexOf('@');
  const basePurl = at >= 0 ? nodePurl.slice(0, at) : nodePurl;
  const current = nodePurl;

  const [data, setData] = useState<CandidatesResp | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(false);
    setData(null);
    api
      .candidates(basePurl)
      .then((resp) => {
        if (!cancelled) setData(resp);
      })
      .catch(() => {
        if (!cancelled) setError(true);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [basePurl]);

  if (loading) {
    return <div className="mt-2 text-[10.5px] text-muted">version room…</div>;
  }
  // Fail quietly: a muted dash, nothing alarming.
  if (error || !data) {
    return <div className="mt-2 text-[10.5px] text-muted">—</div>;
  }

  const { total_versions, candidates, ranges, exclusions, constraint } = data;

  // Map ordinal -> version purl (for hover titles + the current label).
  const purlByOrdinal = new Map<number, string>();
  for (const c of candidates) purlByOrdinal.set(c.ordinal, c.purl);

  // Ordinal of the currently-resolved version (the node we're inspecting).
  let currentOrdinal = -1;
  for (const c of candidates) {
    if (c.purl === current) {
      currentOrdinal = c.ordinal;
      break;
    }
  }

  // Compact line when there's nothing to move between.
  if (total_versions <= 1) {
    return (
      <div className="mt-2 text-[10.5px] text-muted">
        only version{constraint && <span className="font-mono text-muted"> · {constraint}</span>}
      </div>
    );
  }

  // Precompute per-ordinal membership so each cell is an O(1) lookup. `ranges`
  // are half-open [start, end); the satisfying band equals the ordinals the
  // backend already enumerated in `candidates`, so derive from those to be exact.
  const inRange = new Set<number>(candidates.map((c) => c.ordinal));
  for (const r of ranges) {
    for (let o = r.start; o < r.end; o++) inRange.add(o);
  }
  const excludedReason = new Map<number, string>();
  for (const e of exclusions) excludedReason.set(e.ordinal, e.reason);

  const caption =
    currentOrdinal >= 0
      ? `version ${currentOrdinal + 1} of ${total_versions}`
      : `${total_versions} versions`;

  return (
    <div className="mt-2">
      <div className="flex items-baseline gap-1.5 text-[10.5px]">
        <span className="text-muted">{caption}</span>
        {constraint && <span className="ml-auto font-mono text-muted truncate">{constraint}</span>}
      </div>

      {/* Oldest → newest, left → right. */}
      <div className="mt-1 flex gap-px overflow-hidden rounded bg-panel-3 p-px">
        {Array.from({ length: total_versions }, (_, o) => {
          const ver = purlByOrdinal.get(o);
          const reason = excludedReason.get(o);
          const isCurrent = o === currentOrdinal;
          const satisfies = inRange.has(o);

          const title = reason
            ? `${ver ? shortLabel(ver) : `v#${o}`} — ${reason}`
            : ver
              ? shortLabel(ver)
              : `version ${o + 1}`;

          // Fill: excluded → hatched + muted (off-limits), satisfying → accent,
          // otherwise a flat panel cell. The current version gets a bright ring.
          const background = reason
            ? 'repeating-linear-gradient(45deg, #1d212e 0 2px, #2a2f3c 2px 4px)'
            : satisfies
              ? PALETTE.accent
              : '#252a3a';

          return (
            <div
              key={o}
              title={title}
              className="h-3 flex-1 min-w-[3px]"
              style={{
                background,
                opacity: reason ? 0.6 : 1,
                boxShadow: isCurrent ? `inset 0 0 0 2px ${PALETTE.center}` : undefined,
              }}
            />
          );
        })}
      </div>

      {/* Current version label, anchored under the strip. */}
      {currentOrdinal >= 0 && (
        <div className="mt-0.5 font-mono text-[10px]" style={{ color: PALETTE.center }}>
          {shortLabel(current)}
        </div>
      )}
    </div>
  );
}

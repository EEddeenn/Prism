'use client';

import { useEffect, useMemo, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { diffRuns, type FamilyDiff, type DiffStatus } from '@/lib/diff';
import { shortLabel } from '@/lib/palette';
import { api, ApiError } from '@/lib/api';
import type { TopologyDistance, MetricDelta } from '@/lib/types';
import {
  MetricPill,
  SectionHeader,
  RowButton,
  Sparkline,
  DivergingBar,
  Callout,
  PanelState,
  type MeterTone,
} from '@/components/ui';

const ROW_CAP = 50;

// Per-section accent: added reads as ok/green, removed as bad/red, changed as
// warn/amber. `text` is a literal class (Tailwind can't see template strings).
const SECTION: Record<DiffStatus, { title: string; text: string }> = {
  added: { title: 'Added', text: 'text-ok' },
  removed: { title: 'Removed', text: 'text-bad' },
  changed: { title: 'Changed', text: 'text-warn' },
};

/** Direction glyph for a changed family: a single arrow, with an aria-label so the
 *  meaning isn't conveyed by color/title alone. */
function Direction({ direction }: { direction: number }) {
  const cfg =
    direction > 0
      ? { glyph: '↑', cls: 'text-ok', label: 'version increased' }
      : direction < 0
        ? { glyph: '↓', cls: 'text-warn', label: 'version decreased' }
        : { glyph: '↔', cls: 'text-muted', label: 'version set changed' };
  return (
    <span className={`mx-1 ${cfg.cls}`} role="img" aria-label={cfg.label} title={cfg.label}>
      {cfg.glyph}
    </span>
  );
}

/** One family row: monospace name, version delta, blast tag. Click highlights its
 *  A-run ids; families that exist only in the compare run render disabled. */
function DiffRow({ row, onPick }: { row: FamilyDiff; onPick: (r: FamilyDiff) => void }) {
  const onlyInB = row.status === 'added' || row.ids.length === 0;
  const clickable = !onlyInB;

  // Version delta: changed shows aVersions → bVersions with a direction arrow; added/removed
  // show just the side that exists.
  const delta =
    row.status === 'changed' ? (
      <span className="font-mono text-mini text-muted truncate">
        <span className="text-ink">{row.aVersions.join(', ')}</span>
        <Direction direction={row.direction} />
        <span className="text-ink">{row.bVersions.join(', ')}</span>
      </span>
    ) : row.status === 'added' ? (
      <span className="font-mono text-mini text-ok truncate">{row.bVersions.join(', ')}</span>
    ) : (
      <span className="font-mono text-mini text-bad truncate">{row.aVersions.join(', ')}</span>
    );

  return (
    <RowButton
      onSelect={() => onPick(row)}
      disabled={!clickable}
      ariaLabel={`${shortLabel(row.name)}, blast ${row.blast}`}
      title={
        clickable
          ? 'Highlight these nodes in the active run'
          : 'This family exists only in the compare run'
      }
    >
      <div className="min-w-0 flex-1">
        <div className="font-mono text-body truncate text-ink" title={row.name}>{shortLabel(row.name)}</div>
        <div className="mt-0.5 flex items-center gap-1.5">
          {delta}
          {onlyInB && <span className="text-micro text-muted shrink-0">(only in B)</span>}
        </div>
      </div>
      <span className="shrink-0">
        <MetricPill
          label="blast"
          value={row.blast}
          title="max transitive dependents (blast radius)"
        />
      </span>
    </RowButton>
  );
}

function Section({
  status,
  rows,
  onPick,
}: {
  status: DiffStatus;
  rows: FamilyDiff[];
  onPick: (r: FamilyDiff) => void;
}) {
  const cfg = SECTION[status];
  const shown = rows.slice(0, ROW_CAP);
  const more = rows.length - shown.length;

  return (
    <section className="mb-3">
      <SectionHeader count={rows.length}>
        <span className={cfg.text}>{cfg.title}</span>
      </SectionHeader>
      {rows.length === 0 ? (
        // Empty is a neutral outcome — don't announce "Removed: none" as an alert.
        <p className="text-muted text-mini">No {cfg.title.toLowerCase()} families.</p>
      ) : (
        <div className="flex flex-col gap-0.5">
          {shown.map((r) => (
            <DiffRow key={r.key} row={r} onPick={onPick} />
          ))}
          {more > 0 && (
            <p className="px-2 py-1 text-mini text-muted tabular-nums">
              … {more} more (capped at {ROW_CAP}; refine the diff to see the rest)
            </p>
          )}
        </div>
      )}
    </section>
  );
}

/** Format a signed percent integer with an explicit + / − (true minus glyph), or
 *  "0%" when it rounds to nothing. change_pct is already a percent integer. */
function signedPct(n: number): string {
  const r = Math.round(n);
  if (r > 0) return `+${r}%`;
  if (r < 0) return `−${Math.abs(r)}%`;
  return '0%';
}

/** Tone for a single dimension's change: growth reads warn (or bad when large),
 *  shrink reads ok, near-zero muted. Mirrors the diff's added/removed sentiment. */
function deltaTone(changePct: number): MeterTone {
  const r = Math.round(changePct);
  if (r === 0) return 'muted';
  if (r > 0) return r >= 50 ? 'bad' : 'warn';
  return 'ok';
}

// Literal text classes per tone — Tailwind can't see template strings (see the
// SECTION map above for the same constraint).
const TONE_TEXT: Record<MeterTone, string> = {
  accent: 'text-accent',
  ok: 'text-ok',
  warn: 'text-warn',
  bad: 'text-bad',
  muted: 'text-muted',
};

/** One structural dimension: name, base → head values, a diverging bar (base left,
 *  head right) and a signed change label. */
function DistanceRow({ d }: { d: MetricDelta }) {
  const tone = deltaTone(d.change_pct);
  return (
    <div className="rounded-lg bg-panel-2 border border-line px-2.5 py-2">
      <div className="flex items-baseline justify-between gap-2">
        <span className="font-mono text-mini text-ink truncate" title={d.metric}>
          {d.metric}
        </span>
        <span className={`text-mini font-semibold tabular-nums shrink-0 ${TONE_TEXT[tone]}`}>
          {signedPct(d.change_pct)}
        </span>
      </div>
      <DivergingBar
        a={d.base}
        b={d.head}
        leftTone="muted"
        rightTone={tone}
        ariaLabel={`${d.metric}: base ${d.base}, head ${d.head}, ${signedPct(d.change_pct)}`}
      />
      <div className="mt-1 flex items-baseline gap-1 text-micro text-muted tabular-nums">
        <span className="text-ink">{d.base}</span>
        <span aria-hidden>→</span>
        <span className="text-ink">{d.head}</span>
      </div>
    </div>
  );
}

/** Topology distance: the active run (base) vs. the compare run (head), same
 *  orientation as the family diff above. Lazily fetched; degrades to empty. */
function TopologyDistanceSection({ base, head }: { base: number; head: number }) {
  const [dist, setDist] = useState<TopologyDistance | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let live = true;
    setLoading(true);
    setError(null);
    setDist(null); // clear the prior comparison so a refetch shows loading, not stale deltas
    api
      .distance(base, head)
      .then((d) => {
        if (!live) return;
        setDist(d);
      })
      .catch((e) => {
        if (!live) return;
        setError(e instanceof ApiError ? e.message : 'failed to load topology distance');
      })
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [base, head]);

  return (
    <section className="mb-3">
      <SectionHeader>Topology distance</SectionHeader>
      {error ? (
        <Callout tone="bad" role="alert">{error}</Callout>
      ) : loading && !dist ? (
        <PanelState kind="loading">Measuring topology distance…</PanelState>
      ) : !dist ? null : (
        <div className="flex flex-col gap-2">
          <div className="rounded-lg bg-panel-2 border border-line px-2.5 py-2">
            <div className="text-head font-semibold tabular-nums text-ink">
              topology distance: {Math.round(dist.distance_pct)}%
            </div>
            <p className="mt-0.5 text-micro text-muted">
              mean absolute change across structural dimensions; 0% = identical shape (can
              exceed 100% for very different runs).
            </p>
          </div>
          {dist.deltas.length === 0 ? (
            <PanelState kind="empty">No per-dimension deltas reported.</PanelState>
          ) : (
            <div className="flex flex-col gap-1">
              {dist.deltas.map((d) => (
                <DistanceRow key={d.metric} d={d} />
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  );
}

export default function DiffTab() {
  const run = useGraphStore((s) => s.run);
  const nodes = useGraphStore((s) => s.nodes);
  const index = useGraphStore((s) => s.index);
  const compareData = useGraphStore((s) => s.compareData);
  const compareRun = useGraphStore((s) => s.compareRun);
  const highlightSet = useGraphStore((s) => s.highlightSet);

  const diff = useMemo(() => {
    if (!index || !compareData) return null;
    return diffRuns(nodes, index, compareData.nodes, compareData.index);
  }, [nodes, index, compareData]);

  if (!compareData || !diff) {
    return (
      <PanelState kind="prompt">
        Pick a &lsquo;vs&rsquo; run in the top bar to diff two resolutions.
      </PanelState>
    );
  }

  const addedN = diff.added.length;
  const removedN = diff.removed.length;
  const changedN = diff.changed.length;

  // changes-by-depth sparkline, normalized to the busiest depth.
  const depthRows = [...diff.byDepth.entries()].sort((a, b) => a[0] - b[0]);

  const onPick = (r: FamilyDiff) => {
    if (r.ids.length) highlightSet(r.ids, r.name);
  };

  return (
    <div>
      {/* Summary header */}
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-sm mb-3">
        <span className="text-ok font-semibold tabular-nums">+{addedN} added</span>
        <span className="text-muted">·</span>
        <span className="text-bad font-semibold tabular-nums">−{removedN} removed</span>
        <span className="text-muted">·</span>
        <span className="text-warn font-semibold tabular-nums">~{changedN} changed</span>
        <span className="text-muted">·</span>
        <span className="text-muted tabular-nums">{diff.unchangedCount} unchanged</span>
      </div>

      <p className="text-muted text-xs mb-3">
        Active run vs. run <span className="font-mono text-ink">{compareRun ?? compareData.run}</span>. Click a
        changed/removed family to highlight its nodes in the active run.
      </p>

      {/* Changes by depth */}
      {depthRows.length > 0 && (
        <div className="mb-3 rounded-lg bg-panel-2 border border-line px-2.5 py-2">
          <SectionHeader className="mt-0">Changes by depth</SectionHeader>
          <Sparkline
            data={depthRows.map(([, count]) => count)}
            ariaLabel={depthRows
              .map(([depth, count]) => `depth ${depth}: ${count} change${count === 1 ? '' : 's'}`)
              .join('; ')}
          />
          <div className="flex gap-px mt-0.5">
            {depthRows.map(([depth, count]) => (
              <div
                key={depth}
                className="flex-1 min-w-0 text-center text-micro text-muted tabular-nums leading-none"
                title={`depth ${depth}: ${count} change${count === 1 ? '' : 's'}`}
              >
                {depth}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Topology distance: structural shape delta, active (base) → compare (head),
          same orientation as the family diff. Compare-only, like the rest of the tab. */}
      {run != null && compareRun != null && (
        <TopologyDistanceSection base={run} head={compareRun} />
      )}

      {/* Sections, most impactful first within each (rows are pre-sorted by blast). */}
      <Section status="added" rows={diff.added} onPick={onPick} />
      <Section status="removed" rows={diff.removed} onPick={onPick} />
      <Section status="changed" rows={diff.changed} onPick={onPick} />
    </div>
  );
}

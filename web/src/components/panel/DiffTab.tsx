'use client';

import { useEffect, useMemo, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { diffRuns, type FamilyDiff, type DiffStatus } from '@/lib/diff';
import { shortLabel } from '@/lib/palette';
import { api, ApiError } from '@/lib/api';
import type { TopologyDistance, MetricDelta } from '@/lib/types';
import {
  Card,
  DataCard,
  DeltaTag,
  DivergingBar,
  HeroFigure,
  MetricPill,
  MonoLabel,
  PanelState,
  Section,
  SectionHeader,
  ListRow,
  Sparkline,
  TruncationNote,
} from '@/design-system';
import { deltaTone, toneText, type Tone } from '@/lib/tone';

const ROW_CAP = 50;

// Per-section sentiment: added reads good (green), removed bad (red), changed
// warn (amber). One Tone each, resolved to a text class via toneText.
const SECTION: Record<DiffStatus, { title: string; tone: Tone }> = {
  added: { title: 'Added', tone: 'good' },
  removed: { title: 'Removed', tone: 'bad' },
  changed: { title: 'Changed', tone: 'warn' },
};

/** Direction glyph for a changed family: a single arrow, with an aria-label so the
 *  meaning isn't conveyed by color/title alone. */
function Direction({ direction }: { direction: number }) {
  const cfg =
    direction > 0
      ? { glyph: '↑', tone: 'good' as Tone, label: 'version increased' }
      : direction < 0
        ? { glyph: '↓', tone: 'warn' as Tone, label: 'version decreased' }
        : { glyph: '↔', tone: 'neutral' as Tone, label: 'version set changed' };
  return (
    <span className={`mx-1 ${toneText[cfg.tone]}`} role="img" aria-label={cfg.label} title={cfg.label}>
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
      <span className="font-mono text-data-s text-archive truncate">
        <span className="text-bone">{row.aVersions.join(', ')}</span>
        <Direction direction={row.direction} />
        <span className="text-bone">{row.bVersions.join(', ')}</span>
      </span>
    ) : row.status === 'added' ? (
      <MonoLabel text={row.bVersions.join(', ')} className="text-data-s text-good truncate" />
    ) : (
      <MonoLabel text={row.aVersions.join(', ')} className="text-data-s text-bad truncate" />
    );

  return (
    <ListRow
      onSelect={() => onPick(row)}
      disabled={!clickable}
      ariaLabel={`${shortLabel(row.name)}, blast ${row.blast}`}
      title={
        clickable
          ? 'Highlight these nodes in the active run'
          : 'This family exists only in the compare run'
      }
    >
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <MonoLabel
            text={shortLabel(row.name)}
            full={row.name}
            className="block text-data text-bone"
          />
          <div className="mt-0.5 flex items-center gap-1.5">
            {delta}
            {onlyInB && <span className="text-meta text-archive shrink-0">(only in B)</span>}
          </div>
        </div>
        <span className="shrink-0">
          <MetricPill
            label="blast"
            value={row.blast}
            title="max transitive dependents (blast radius)"
          />
        </span>
      </div>
    </ListRow>
  );
}

function DiffSection({
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
    <Section className="mb-3">
      <SectionHeader count={rows.length}>
        <span className={toneText[cfg.tone]}>{cfg.title}</span>
      </SectionHeader>
      {rows.length === 0 ? (
        // Empty is a neutral outcome — don't announce "Removed: none" as an alert.
        <p className="text-archive text-data-s">No {cfg.title.toLowerCase()} families.</p>
      ) : (
        <div className="flex flex-col gap-0.5">
          {shown.map((r) => (
            <DiffRow key={r.key} row={r} onPick={onPick} />
          ))}
          {more > 0 && (
            <TruncationNote
              shown={shown.length}
              total={rows.length}
              more={more}
              className="px-2 py-1"
            />
          )}
        </div>
      )}
    </Section>
  );
}

/** One structural dimension: name, base → head values, a diverging bar (base left,
 *  head right) and a signed change label. */
function DistanceRow({ d }: { d: MetricDelta }) {
  // Growth in a structural dimension reads as drift away from the baseline (bad);
  // shrink reads good; near-zero neutral — the shared drift sentiment.
  const tone = deltaTone(d.change_pct);
  return (
    <Card className="px-2.5 py-2">
      <div className="flex items-baseline justify-between gap-2">
        <MonoLabel text={d.metric} className="text-data-s text-bone" />
        <DeltaTag
          pct={d.change_pct}
          ariaLabel={`${d.metric}: base ${d.base}, head ${d.head}, changed ${Math.round(d.change_pct)} percent`}
          className="shrink-0"
        />
      </div>
      <DivergingBar
        a={d.base}
        b={d.head}
        leftTone="neutral"
        rightTone={tone}
        ariaLabel={`${d.metric}: base ${d.base}, head ${d.head}`}
      />
      <div className="mt-1 flex items-baseline gap-1 text-meta text-archive tabular-nums">
        <span className="font-mono text-bone">{d.base}</span>
        <span aria-hidden>→</span>
        <span className="font-mono text-bone">{d.head}</span>
      </div>
    </Card>
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
    <Section className="mb-3">
      <SectionHeader>Topology distance</SectionHeader>
      {error ? (
        <PanelState kind="error">{error}</PanelState>
      ) : loading && !dist ? (
        <PanelState kind="loading">Measuring topology distance…</PanelState>
      ) : !dist ? null : (
        <div className="flex flex-col gap-2">
          <Card className="px-2.5 py-2">
            <HeroFigure
              value={Math.round(dist.distance_pct)}
              unit="%"
              tone={deltaTone(dist.distance_pct)}
              label="topology distance"
              sub="mean absolute change across structural dimensions; 0% = identical shape (can exceed 100% for very different runs)."
            />
          </Card>
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
    </Section>
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
      <div className="mb-3 flex flex-wrap items-baseline gap-x-2 gap-y-1 text-data">
        <span className="font-mono tabular-nums text-good">+{addedN} added</span>
        <span aria-hidden className="text-archive">·</span>
        <span className="font-mono tabular-nums text-bad">−{removedN} removed</span>
        <span aria-hidden className="text-archive">·</span>
        <span className="font-mono tabular-nums text-warn">~{changedN} changed</span>
        <span aria-hidden className="text-archive">·</span>
        <span className="font-mono tabular-nums text-archive">{diff.unchangedCount} unchanged</span>
      </div>

      <p className="mb-3 text-data-s text-archive">
        Active run vs. run{' '}
        <span className="font-mono tabular-nums text-bone">{compareRun ?? compareData.run}</span>. Click a
        changed/removed family to highlight its nodes in the active run.
      </p>

      {/* Changes by depth */}
      {depthRows.length > 0 && (
        <DataCard eyebrow="Changes by depth" className="mb-3">
          <Sparkline
            data={depthRows.map(([, count]) => count)}
            ariaLabel={depthRows
              .map(([depth, count]) => `depth ${depth}: ${count} change${count === 1 ? '' : 's'}`)
              .join('; ')}
          />
          <div className="mt-0.5 flex gap-px">
            {depthRows.map(([depth, count]) => (
              <div
                key={depth}
                className="min-w-0 flex-1 text-center font-mono text-meta text-archive tabular-nums leading-none"
                title={`depth ${depth}: ${count} change${count === 1 ? '' : 's'}`}
              >
                {depth}
              </div>
            ))}
          </div>
        </DataCard>
      )}

      {/* Topology distance: structural shape delta, active (base) → compare (head),
          same orientation as the family diff. Compare-only, like the rest of the tab. */}
      {run != null && compareRun != null && (
        <TopologyDistanceSection base={run} head={compareRun} />
      )}

      {/* Sections, most impactful first within each (rows are pre-sorted by blast). */}
      <DiffSection status="added" rows={diff.added} onPick={onPick} />
      <DiffSection status="removed" rows={diff.removed} onPick={onPick} />
      <DiffSection status="changed" rows={diff.changed} onPick={onPick} />
    </div>
  );
}

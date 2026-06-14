'use client';

/* A full-pane squarified treemap — a third top-level view alongside 2D/3D.
 *
 * One rectangle per node; area is proportional to a chosen "reach" signal (how
 * much of the graph hangs off this package), so the heaviest dependencies read
 * as the largest tiles. Tiles are coloured by depth and share the 2D/3D shared
 * selection: clicking a tile selects its node and asks the other views to focus
 * it. The squarify layout is implemented in-file (no d3 dependency) and renders
 * as absolutely-positioned divs, which stays cheap for ~900 tiles. */

import { useEffect, useMemo, useRef, useState } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import type { NodeMetrics } from '@/lib/graph';
import { PALETTE, rampHex, shortLabel } from '@/lib/palette';
import { Badge, PanelState, SectionHeader, SegmentedControl, type SegOption } from '@/components/ui';

// The signals you can weight tile area by. Each maps to a field on NodeMetrics;
// `revReach` (blast radius / transitive dependents) is the default.
type AreaSignal = 'revReach' | 'fwdReach' | 'outDeg' | 'dupes';

const SIGNALS: { id: AreaSignal; label: string; title: string }[] = [
  { id: 'revReach', label: 'blast', title: 'Transitive dependents (blast radius)' },
  { id: 'fwdReach', label: 'reach', title: 'Transitive dependencies' },
  { id: 'outDeg', label: 'fan-out', title: 'Direct dependencies' },
  { id: 'dupes', label: 'dupes', title: 'Family version pressure (famSize − 1)' },
];

const SIGNAL_OPTIONS: SegOption<AreaSignal>[] = SIGNALS.map((s) => ({
  value: s.id,
  label: s.label,
  title: s.title,
}));

function signalOf(m: NodeMetrics, sig: AreaSignal): number {
  switch (sig) {
    case 'revReach': return m.revReach;
    case 'fwdReach': return m.fwdReach;
    case 'outDeg': return m.outDeg;
    case 'dupes': return m.famSize - 1;
  }
}

// --- squarified treemap ----------------------------------------------------

interface Item {
  id: number;
  value: number; // area weight (already ≥ 1)
}
interface Tile {
  id: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Worst aspect ratio of a row of areas laid along a side of length `len`. */
function worst(row: number[], len: number, sum: number): number {
  if (row.length === 0 || len === 0) return Infinity;
  const s2 = sum * sum;
  const l2 = len * len;
  let rmax = -Infinity;
  let rmin = Infinity;
  for (const r of row) {
    if (r > rmax) rmax = r;
    if (r < rmin) rmin = r;
  }
  return Math.max((l2 * rmax) / s2, s2 / (l2 * rmin));
}

/**
 * Squarify (Bruls, Huizing & van Wijk). Lays `items` — pre-scaled so their
 * values sum to the rectangle's area — into tiles that stay close to square.
 * Items should be sorted descending by value for the best aspect ratios.
 */
function squarify(items: Item[], x0: number, y0: number, w: number, h: number): Tile[] {
  const out: Tile[] = [];
  let x = x0;
  let y = y0;
  let rw = w;
  let rh = h;
  let i = 0;

  while (i < items.length) {
    const len = Math.min(rw, rh);
    const row: number[] = [];
    const rowItems: Item[] = [];
    let rowSum = 0;
    let best = Infinity;

    // Grow the current row while it improves (lowers) the worst aspect ratio.
    while (i < items.length) {
      const v = items[i].value;
      const next = worst([...row, v], len, rowSum + v);
      if (row.length > 0 && next > best) break;
      row.push(v);
      rowItems.push(items[i]);
      rowSum += v;
      best = worst(row, len, rowSum);
      i++;
    }

    // Place the finished row along the shorter side, then shrink the rectangle.
    if (rw >= rh) {
      const colW = rh > 0 ? rowSum / rh : 0;
      let yy = y;
      for (const it of rowItems) {
        const th = rowSum > 0 ? (it.value / rowSum) * rh : 0;
        out.push({ id: it.id, x, y: yy, w: colW, h: th });
        yy += th;
      }
      x += colW;
      rw -= colW;
    } else {
      const rowH = rw > 0 ? rowSum / rw : 0;
      let xx = x;
      for (const it of rowItems) {
        const tw = rowSum > 0 ? (it.value / rowSum) * rw : 0;
        out.push({ id: it.id, x: xx, y, w: tw, h: rowH });
        xx += tw;
      }
      y += rowH;
      rh -= rowH;
    }
  }

  return out;
}

// --- component -------------------------------------------------------------

const GAP = 1; // px gap between tiles (the dark bg shows through)

// Cap on how many of the largest tiles join the keyboard tab order. The long
// tail keeps tabIndex=-1 so a screen-reader / keyboard user isn't forced to
// step through ~900 tiles to leave the map.
const FOCUSABLE_COUNT = 80;

/** A single legend swatch + label, used for the contextual role chips. */
function LegendChip({ color, label }: { color: string; label: string }) {
  return (
    <span className="flex items-center gap-1.5 text-micro text-muted whitespace-nowrap">
      <span
        className="h-2.5 w-2.5 rounded-[3px] border"
        style={{ borderColor: color, boxShadow: `inset 0 0 0 1.5px ${color}` }}
        aria-hidden
      />
      {label}
    </span>
  );
}

export default function Treemap() {
  const nodes = useGraphStore((s) => s.nodes);
  const index = useGraphStore((s) => s.index);
  const sel = useGraphStore((s) => s.sel);
  const fwd = useGraphStore((s) => s.fwd);
  const impact = useGraphStore((s) => s.impact);
  const matches = useGraphStore((s) => s.matches);
  const dupVersionIds = useGraphStore((s) => s.dupVersionIds);
  const selectNode = useGraphStore((s) => s.selectNode);
  const clearSelection = useGraphStore((s) => s.clearSelection);
  const requestFocus = useGraphStore((s) => s.requestFocus);

  const [signal, setSignal] = useState<AreaSignal>('revReach');
  const [size, setSize] = useState({ w: 0, h: 0 });
  const hostRef = useRef<HTMLDivElement>(null);

  // Track the pane size with a ResizeObserver so the layout fills the container.
  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const maxDepth = useMemo(() => {
    let m = 0;
    for (const n of nodes) if (n.depth > m) m = n.depth;
    return m;
  }, [nodes]);

  // Build area-weighted items (min area 1 so every node stays clickable), then
  // squarify into the measured pane.
  const tiles = useMemo<Tile[]>(() => {
    const { w, h } = size;
    if (!index || w <= 0 || h <= 0 || nodes.length === 0) return [];

    const items: Item[] = nodes.map((n) => {
      const m = index.metrics.get(n.id);
      const raw = m ? signalOf(m, signal) : 0;
      return { id: n.id, value: Math.max(1, raw) };
    });
    items.sort((a, b) => b.value - a.value);

    const total = items.reduce((s, it) => s + it.value, 0);
    if (total <= 0) return [];

    // Scale values so their sum equals the available pixel area.
    const area = w * h;
    const scale = area / total;
    const scaled = items.map((it) => ({ id: it.id, value: it.value * scale }));

    return squarify(scaled, 0, 0, w, h);
  }, [index, nodes, signal, size]);

  // Exact (unscaled) signal value per node, for the hover title.
  const rawValue = (id: number): number => {
    const m = index?.metrics.get(id);
    return m ? signalOf(m, signal) : 0;
  };

  const byId = index?.byId;
  const rootIds = index?.rootIds;
  const approximate = !!index?.approximate;
  const signalsThatNeedReach = signal === 'revReach' || signal === 'fwdReach';

  // Which contextual neighbourhood is active drives both per-tile dimming and
  // the legend's role chips. Selection takes priority over search; dup/root are
  // always-on. Mirrors the precedence used by the 2D/3D views.
  const hasSel = sel != null;
  const searchActive = !hasSel && matches.size > 0;

  const SIGNAL_META = SIGNALS.find((s) => s.id === signal)!;

  return (
    <div className="absolute inset-0 bg-bg overflow-hidden">
      {/* in-component signal toggle (mirrors the Controls3D button-group idiom) */}
      <div className="absolute left-3.5 top-3.5 z-10 flex flex-col gap-2 p-2.5 rounded-xl border border-line bg-panel/90 shadow-panel backdrop-blur-sm">
        <SectionHeader className="mt-0 mb-0">Tile area</SectionHeader>
        <SegmentedControl
          options={SIGNAL_OPTIONS}
          value={signal}
          onChange={setSignal}
          ariaLabel="Tile area signal"
          size="sm"
          className="w-full [&>button]:flex-1"
        />
        <p className="text-micro leading-snug text-muted max-w-[280px]">
          {nodes.length} tiles · area ∝ {SIGNALS.find((s) => s.id === signal)!.title.toLowerCase()} · colour = depth
        </p>
      </div>

      {/* approximate-data badge: reach signals fell back to degree proxies */}
      {approximate && signalsThatNeedReach && (
        <div className="absolute right-3.5 top-3.5 z-10">
          <Badge
            tone="warn"
            mono
            title="Graph too large for exact transitive reach; values are degree proxies."
          >
            ≈ approximate
          </Badge>
        </div>
      )}

      <div
        ref={hostRef}
        className="absolute inset-0"
        onClick={(e) => {
          // Tiles call stopPropagation, so this fires only on a click that
          // landed on the host's own background (the inter-tile gaps + any
          // padding) — treat as a "deselect" gesture, mirroring 2D / 3D.
          if (e.target === e.currentTarget && sel != null) clearSelection();
        }}
      >
        {tiles.map((t, ti) => {
          const n = byId?.get(t.id);
          if (!n) return null;

          const innerW = Math.max(0, t.w - GAP);
          const innerH = Math.max(0, t.h - GAP);
          const fill = rampHex(maxDepth > 0 ? n.depth / maxDepth : 0);
          // Only label tiles roomy enough to read the short label without clipping.
          const showLabel = innerW >= 46 && innerH >= 16;

          // --- shared highlight state (same precedence as the 2D/3D views) ---
          const selected = sel === t.id;
          const inFwd = hasSel && fwd.has(t.id);
          const inImpact = hasSel && impact.has(t.id);
          const isMatch = searchActive && matches.has(t.id);
          const isDup = dupVersionIds.has(n.version_id);
          const isRoot = !!rootIds?.has(t.id);

          // Dim anything outside the active neighbourhood / search results.
          let opacity = 1;
          if (hasSel && !inFwd && !inImpact) opacity = 0.25;
          else if (searchActive && !isMatch) opacity = 0.25;

          // Compose rings as a layered box-shadow. CSS paints the FIRST shadow in
          // the list on top, so the contextual selection/search ring is pushed
          // first (takes priority over the always-on dup/root markers, which
          // overlap the same inset edge), then dup/root, then the faint dark edge.
          const shadows: string[] = [];
          // Contextual rings (highest priority — painted on top).
          if (selected) {
            shadows.push(`inset 0 0 0 2px ${PALETTE.center}`, `0 0 0 1px ${PALETTE.center}`);
          } else if (inFwd) {
            shadows.push(`inset 0 0 0 2px ${PALETTE.fwd}`);
          } else if (inImpact) {
            shadows.push(`inset 0 0 0 2px ${PALETTE.impact}`);
          } else if (isMatch) {
            shadows.push(`inset 0 0 0 2px ${PALETTE.accent}`);
          }
          // Always-on markers (beneath the contextual ring): dup pink, root purple.
          if (isDup) shadows.push(`inset 0 0 0 1.5px ${PALETTE.dup}`);
          if (isRoot) shadows.push(`inset 0 0 0 1.5px ${PALETTE.root}`);
          // Faint dark inner edge keeps adjacent same-depth tiles legible.
          shadows.push('inset 0 0 0 1px rgba(0,0,0,0.28)');

          // Lift highlighted tiles above their neighbours so rings aren't clipped.
          let zIndex = 1;
          if (selected) zIndex = 6;
          else if (inFwd || inImpact || isMatch) zIndex = 4;
          else if (isDup || isRoot) zIndex = 2;

          // Keyboard focus only for the largest tiles (tiles are size-descending),
          // so the long tail doesn't bloat the tab order.
          const focusable = ti < FOCUSABLE_COUNT;

          const activate = () => {
            void selectNode(t.id);
            requestFocus(t.id);
          };

          return (
            <div
              key={t.id}
              role="button"
              tabIndex={focusable ? 0 : -1}
              aria-label={`${shortLabel(n.purl)} — ${SIGNAL_META.label} ${rawValue(t.id)}, depth ${n.depth}`}
              title={`${n.purl}\n${SIGNAL_META.label} = ${rawValue(t.id)} · depth ${n.depth}`}
              onClick={(e) => {
                // Stop the click from bubbling to the host container's
                // empty-space-deselect handler — a tile activation
                // immediately followed by a deselect would clear the
                // selection we just installed.
                e.stopPropagation();
                activate();
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  activate();
                }
              }}
              className="absolute box-border cursor-pointer overflow-hidden flex items-start outline-none focus-visible:ring-2 focus-visible:ring-accent"
              style={{
                left: t.x,
                top: t.y,
                width: innerW,
                height: innerH,
                background: fill,
                opacity,
                boxShadow: shadows.join(', '),
                zIndex,
              }}
            >
              {showLabel && (
                <span
                  className="px-1 pt-0.5 text-micro leading-tight font-mono truncate w-full pointer-events-none"
                  style={{ color: 'rgba(8,10,14,0.92)' }}
                >
                  {shortLabel(n.purl)}
                </span>
              )}
            </div>
          );
        })}
      </div>

      {/* In-component legend (page.tsx legend is hidden in the map view). Depth
          ramp on top; neighbourhood role chips shown only when relevant. */}
      {nodes.length > 0 && (
        <div className="absolute left-3.5 bottom-3.5 z-10 flex flex-col gap-2 p-2.5 rounded-xl border border-line bg-panel/90 shadow-panel backdrop-blur-sm">
          <div className="flex items-center gap-2">
            <span className="text-micro text-muted">shallow</span>
            <span
              className="h-2.5 w-24 rounded-full"
              style={{
                background: `linear-gradient(to right, ${rampHex(0)}, ${rampHex(0.25)}, ${rampHex(0.5)}, ${rampHex(0.75)}, ${rampHex(1)})`,
              }}
              aria-hidden
            />
            <span className="text-micro text-muted">deep</span>
          </div>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 max-w-[280px]">
            {hasSel && (
              <>
                <LegendChip color={PALETTE.center} label="selected" />
                <LegendChip color={PALETTE.fwd} label="depends-on" />
                <LegendChip color={PALETTE.impact} label="depended-on-by" />
              </>
            )}
            {searchActive && <LegendChip color={PALETTE.accent} label="search-match" />}
            {dupVersionIds.size > 0 && <LegendChip color={PALETTE.dup} label="duplicated" />}
            {!!rootIds?.size && <LegendChip color={PALETTE.root} label="root" />}
          </div>
        </div>
      )}

      {nodes.length === 0 && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
          <PanelState kind="empty">No graph nodes to map.</PanelState>
        </div>
      )}
    </div>
  );
}

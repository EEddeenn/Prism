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

// The signals you can weight tile area by. Each maps to a field on NodeMetrics;
// `revReach` (blast radius / transitive dependents) is the default.
type AreaSignal = 'revReach' | 'fwdReach' | 'outDeg' | 'dupes';

const SIGNALS: { id: AreaSignal; label: string; title: string }[] = [
  { id: 'revReach', label: 'blast', title: 'Transitive dependents (blast radius)' },
  { id: 'fwdReach', label: 'reach', title: 'Transitive dependencies' },
  { id: 'outDeg', label: 'fan-out', title: 'Direct dependencies' },
  { id: 'dupes', label: 'dupes', title: 'Family version pressure (famSize − 1)' },
];

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

export default function Treemap() {
  const nodes = useGraphStore((s) => s.nodes);
  const index = useGraphStore((s) => s.index);
  const sel = useGraphStore((s) => s.sel);
  const selectNode = useGraphStore((s) => s.selectNode);
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
  const approximate = !!index?.approximate;
  const signalsThatNeedReach = signal === 'revReach' || signal === 'fwdReach';

  return (
    <div className="absolute inset-0 bg-bg overflow-hidden">
      {/* in-component signal toggle (mirrors the Controls3D button-group idiom) */}
      <div className="absolute left-3.5 top-3.5 z-10 flex flex-col gap-2 p-2.5 rounded-xl border border-line bg-panel/90 shadow-panel backdrop-blur-sm">
        <span className="text-[10.5px] uppercase tracking-[1px] text-muted">Tile area</span>
        <div className="flex">
          {SIGNALS.map((s, i) => (
            <button
              key={s.id}
              type="button"
              title={s.title}
              onClick={() => setSignal(s.id)}
              className={`flex-1 px-2 py-1.5 text-[11.5px] font-semibold border border-line-2 ${
                i === 0 ? 'rounded-l-lg' : i === SIGNALS.length - 1 ? 'rounded-r-lg border-l-0' : 'border-l-0'
              } ${signal === s.id ? 'bg-panel-3 text-ink border-accent' : 'bg-panel-2 text-muted'}`}
            >
              {s.label}
            </button>
          ))}
        </div>
        <p className="text-[10.5px] leading-snug text-muted max-w-[280px]">
          {nodes.length} tiles · area ∝ {SIGNALS.find((s) => s.id === signal)!.title.toLowerCase()} · colour = depth
        </p>
      </div>

      {/* approximate-data badge: reach signals fell back to degree proxies */}
      {approximate && signalsThatNeedReach && (
        <div
          className="absolute right-3.5 top-3.5 z-10 px-2.5 py-1.5 rounded-lg border border-warn/60 bg-panel/90 shadow-panel backdrop-blur-sm text-[11px] font-semibold text-warn font-mono"
          title="Graph too large for exact transitive reach; values are degree proxies."
        >
          ≈ approximate
        </div>
      )}

      <div ref={hostRef} className="absolute inset-0">
        {tiles.map((t) => {
          const n = byId?.get(t.id);
          if (!n) return null;

          const innerW = Math.max(0, t.w - GAP);
          const innerH = Math.max(0, t.h - GAP);
          const selected = sel === t.id;
          const fill = rampHex(maxDepth > 0 ? n.depth / maxDepth : 0);
          // Only label tiles roomy enough to read the short label without clipping.
          const showLabel = innerW >= 46 && innerH >= 16;

          return (
            <div
              key={t.id}
              role="button"
              tabIndex={-1}
              title={`${n.purl}\n${SIGNALS.find((s) => s.id === signal)!.label} = ${rawValue(t.id)} · depth ${n.depth}`}
              onClick={() => {
                void selectNode(t.id);
                requestFocus(t.id);
              }}
              className="absolute box-border cursor-pointer overflow-hidden flex items-start"
              style={{
                left: t.x,
                top: t.y,
                width: innerW,
                height: innerH,
                background: fill,
                // A bright ring for the shared selection; a faint dark inner edge
                // otherwise, so adjacent same-depth tiles stay distinguishable.
                boxShadow: selected
                  ? `inset 0 0 0 2px ${PALETTE.center}, 0 0 0 1px ${PALETTE.center}`
                  : 'inset 0 0 0 1px rgba(0,0,0,0.28)',
                zIndex: selected ? 5 : 1,
              }}
            >
              {showLabel && (
                <span
                  className="px-1 pt-0.5 text-[10px] leading-tight font-mono truncate w-full pointer-events-none"
                  style={{ color: 'rgba(8,10,14,0.92)' }}
                >
                  {shortLabel(n.purl)}
                </span>
              )}
            </div>
          );
        })}
      </div>

      {nodes.length === 0 && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
          <p className="text-muted text-sm">No graph nodes to map.</p>
        </div>
      )}
    </div>
  );
}

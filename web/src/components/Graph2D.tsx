'use client';

/* The 2D layered SVG view. Pan/zoom mutates the viewport <g> transform via a ref
 * (no React re-render on drag); React only re-renders when the graph or the
 * shared selection changes. Clicking a node selects it; hovering reports it to
 * the store (which re-renders only the DOM HoverCard, never this SVG). Node size
 * + neutral colour both encode IMPORTANCE (blast radius) via the shared scale.
 *
 * Big-graph performance: the scene is split into a STATIC layer (neutral edges +
 * neutral nodes, memoized on graph data only) and a thin HIGHLIGHT OVERLAY
 * (selection cones + match accents). A selection click only re-renders the
 * overlay (typically <200 elements) instead of every edge/node (often 10k+).
 * Hover and click use event delegation off the viewport, so 3k+ nodes share
 * one pointer listener instead of three each. */

import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { PALETTE, shortLabel, mixHex } from '@/lib/palette';
import { edgeVisible, isFlagged, kindColor } from '@/lib/edges';
import { importanceColor, importanceT, radius2D } from '@/lib/encoding';
import { Hud } from '@/design-system';
import HoverCard from '@/components/HoverCard';
import type { MetricStats, NodeMetrics } from '@/lib/graph';

const MIN_K = 0.05, MAX_K = 6, LABEL_ALL_BELOW = 70, LABEL_ZOOM = 1.5;
// Fixed-corner overview minimap (bottom-right).
const MINI_W = 150, MINI_H = 110, MINI_PAD = 4;
// Neutral arrowhead: a mid neutral derived from the edge/archive tokens so it
// reads as instrument chrome (not a hardcoded blue-gray) over the dark field.
const ARROW_FILL = mixHex(PALETTE.edge, PALETTE.muted, 0.45);
const ZERO_STATS: MetricStats = {
  maxRevReach: 0, maxFwdReach: 0, maxInDeg: 0, maxOutDeg: 0, maxFamSize: 0, maxDepth: 0,
};

type EdgeGeo = {
  source: number; target: number;
  ax: number; ay: number; bx: number; by: number;
  visible: boolean; flagged: boolean;
  neutralStroke: string; neutralWidth: number; neutralOpacity: number;
};

type NodeBase = {
  id: number; vid: number;
  px: number; py: number; r: number; t: number;
  isRoot: boolean; dupNode: boolean; purl: string;
};

export default function Graph2D() {
  const nodes = useGraphStore((s) => s.nodes);
  const edges = useGraphStore((s) => s.edges);
  const pos = useGraphStore((s) => s.pos2d);
  const world = useGraphStore((s) => s.world2d);
  const sel = useGraphStore((s) => s.sel);
  const fwd = useGraphStore((s) => s.fwd);
  const impact = useGraphStore((s) => s.impact);
  const matches = useGraphStore((s) => s.matches);
  const dup = useGraphStore((s) => s.dupVersionIds);
  const edgeFilter = useGraphStore((s) => s.edgeFilter);
  const index = useGraphStore((s) => s.index);
  const rootIds = index?.rootIds;
  const metrics = index?.metrics;
  const stats = index?.stats ?? ZERO_STATS;
  const selectNode = useGraphStore((s) => s.selectNode);
  const clearSelection = useGraphStore((s) => s.clearSelection);
  const setHovered = useGraphStore((s) => s.setHovered);
  const focusNonce = useGraphStore((s) => s.focusNonce);
  const focusTarget = useGraphStore((s) => s.focusTarget);

  const svgRef = useRef<SVGSVGElement>(null);
  const vpRef = useRef<SVGGElement>(null);
  const miniSvgRef = useRef<SVGSVGElement>(null);
  const miniRectRef = useRef<SVGRectElement>(null);
  const T = useRef({ x: 0, y: 0, k: 1 });
  const drag = useRef({ down: false, moved: 0, lastX: 0, lastY: 0, dragging: false, wasDrag: false });
  // Delegated-hover tracker: lets one pointermove on the viewport report the
  // node under the cursor without 3k per-node enter/leave listeners.
  const lastHover = useRef<number | null>(null);

  const hasSel = sel != null;
  const searching = !hasSel && matches.size > 0;
  const small = nodes.length <= LABEL_ALL_BELOW;

  const size = useCallback(() => {
    const svg = svgRef.current;
    return { w: svg?.clientWidth || 800, h: svg?.clientHeight || 600 };
  }, []);

  // Minimap geometry: fit world.w/world.h into the padded box, then centre it.
  const boxW = MINI_W - MINI_PAD * 2, boxH = MINI_H - MINI_PAD * 2;
  const miniScale = Math.min(boxW / Math.max(1, world.w), boxH / Math.max(1, world.h));
  const miniOffX = MINI_PAD + (boxW - world.w * miniScale) / 2;
  const miniOffY = MINI_PAD + (boxH - world.h * miniScale) / 2;

  // Move the minimap's viewport rectangle to mirror the live main view. Kept in a
  // ref + setAttribute so it tracks pan/zoom without a React re-render (same as vp).
  const syncMini = useCallback(() => {
    const r = miniRectRef.current;
    if (!r) return;
    const { x, y, k } = T.current;
    const { w: vw, h: vh } = size();
    // Visible world rect → minimap-local coords.
    const wx = -x / k, wy = -y / k, ww = vw / k, wh = vh / k;
    r.setAttribute('x', String(miniOffX + wx * miniScale));
    r.setAttribute('y', String(miniOffY + wy * miniScale));
    r.setAttribute('width', String(Math.max(0, ww * miniScale)));
    r.setAttribute('height', String(Math.max(0, wh * miniScale)));
  }, [size, miniOffX, miniOffY, miniScale]);

  const apply = useCallback(() => {
    const { x, y, k } = T.current;
    vpRef.current?.setAttribute('transform', `translate(${x} ${y}) scale(${k})`);
    svgRef.current?.classList.toggle('zoomed', k >= LABEL_ZOOM);
    syncMini();
  }, [syncMini]);

  const fit = useCallback(() => {
    if (!nodes.length) return;
    const { w: vw, h: vh } = size();
    const pad = 40;
    const k = Math.max(MIN_K, Math.min(MAX_K, Math.min(
      (vw - pad * 2) / Math.max(1, world.w),
      (vh - pad * 2) / Math.max(1, world.h),
      1.4,
    )));
    T.current = { k, x: (vw - world.w * k) / 2, y: (vh - world.h * k) / 2 };
    apply();
  }, [nodes.length, world, size, apply]);

  // Fit on mount and whenever a new graph loads.
  useEffect(() => {
    fit();
  }, [nodes, world, fit]);

  // Recenter when something requests focus (search, duplicate highlight, …).
  // Depend on `pos` too so a focus that arrives BEFORE the layout finishes (or a
  // layout that changes after focus) still ends with the camera on the node, not
  // its stale coordinates. `apply` is stable across renders so it doesn't churn.
  useEffect(() => {
    if (focusTarget == null) return;
    const p = pos.get(focusTarget);
    if (!p) return;
    const { w, h } = size();
    T.current.x = w / 2 - p.x * T.current.k;
    T.current.y = h / 2 - p.y * T.current.k;
    apply();
  }, [focusNonce, focusTarget, pos, size, apply]);

  const zoomAt = (px: number, py: number, factor: number) => {
    const k0 = T.current.k;
    const k1 = Math.max(MIN_K, Math.min(MAX_K, k0 * factor));
    T.current.x = px - (px - T.current.x) * (k1 / k0);
    T.current.y = py - (py - T.current.y) * (k1 / k0);
    T.current.k = k1;
    apply();
  };
  const zoomCenter = (factor: number) => {
    const { w, h } = size();
    zoomAt(w / 2, h / 2, factor);
  };

  // === Precomputed per-graph data ====================================
  // Heavy math (Math.hypot, metric lookups, importance maps) runs ONCE per
  // graph swap, not on every selection click.

  const edgeGeo = useMemo<(EdgeGeo | null)[]>(() => {
    const out: (EdgeGeo | null)[] = new Array(edges.length);
    for (let i = 0; i < edges.length; i++) {
      const e = edges[i];
      const a = pos.get(e.source);
      const b = pos.get(e.target);
      if (!a || !b) { out[i] = null; continue; }
      const dx = b.x - a.x, dy = b.y - a.y;
      const len = Math.hypot(dx, dy) || 1;
      const ux = dx / len, uy = dy / len;
      const sm = metrics?.get(e.source) as NodeMetrics | undefined;
      const tm = metrics?.get(e.target) as NodeMetrics | undefined;
      const sr = sm ? radius2D(importanceT(sm, stats)) : 10;
      const tr = tm ? radius2D(importanceT(tm, stats)) : 10;
      const tt = tm ? importanceT(tm, stats) : 0;
      const visible = edgeVisible(e, edgeFilter);
      const flagged = isFlagged(e);
      const w = 1 + 1.6 * tt;
      out[i] = {
        source: e.source, target: e.target,
        ax: a.x + ux * sr, ay: a.y + uy * sr,
        bx: b.x - ux * (tr + 2), by: b.y - uy * (tr + 2),
        visible, flagged,
        // Neutral stroke/width/opacity bake in kind colour + flagged emphasis +
        // edge-filter dimming. Selection-driven highlight rides in a separate
        // overlay layer so a sel change does NOT invalidate this cache.
        neutralStroke: visible ? kindColor(e.kind) : PALETTE.edge,
        neutralWidth: visible ? (flagged ? Math.max(1.8, w) : w) : 1.3,
        neutralOpacity: visible ? (flagged ? 0.95 : 0.5) : 0.05,
      };
    }
    return out;
  }, [edges, pos, metrics, stats, edgeFilter]);

  const nodeBase = useMemo<(NodeBase | null)[]>(() => {
    return nodes.map((n) => {
      const p = pos.get(n.id);
      if (!p) return null;
      const m = metrics?.get(n.id) as NodeMetrics | undefined;
      const t = m ? importanceT(m, stats) : 0;
      const r = m ? radius2D(t) : 10;
      return {
        id: n.id, vid: n.version_id,
        px: p.x, py: p.y, r, t,
        isRoot: rootIds?.has(n.id) ?? false,
        dupNode: dup.has(n.version_id),
        purl: n.purl,
      };
    });
  }, [nodes, pos, metrics, stats, rootIds, dup]);

  // === Static layer (memoized on graph data only) ====================
  // 7k+ <line>s and 3k+ <g.nodeg>s — rebuilt only when the graph or filter
  // changes. Selection clicks no longer touch these.

  const neutralEdges = useMemo(() => {
    const out: React.ReactNode[] = [];
    for (let i = 0; i < edgeGeo.length; i++) {
      const g = edgeGeo[i];
      if (!g) continue;
      out.push(
        <line
          key={i}
          x1={g.ax} y1={g.ay} x2={g.bx} y2={g.by}
          stroke={g.neutralStroke}
          strokeWidth={g.neutralWidth}
          strokeOpacity={g.neutralOpacity}
          markerEnd="url(#arrow2d)"
        />,
      );
    }
    return out;
  }, [edgeGeo]);

  const neutralNodes = useMemo(() => {
    const out: React.ReactNode[] = [];
    for (const b of nodeBase) {
      if (!b) continue;
      const fill = b.isRoot ? PALETTE.root : importanceColor(b.t);
      const ring = b.dupNode ? PALETTE.dup : PALETTE.bg;
      const ringW = b.dupNode ? 2.5 : 1.5;
      // `data-id` powers the delegated pointer handler on the viewport. Per-node
      // listeners are gone — one listener handles hover + click for the whole
      // graph, saving thousands of event registrations.
      out.push(
        <g key={b.id} className="nodeg" data-id={b.id}>
          <circle cx={b.px} cy={b.py} r={b.r} fill={fill} stroke={ring} strokeWidth={ringW} />
          <text x={b.px + b.r + 4} y={b.py + 4}>{shortLabel(b.purl)}</text>
        </g>,
      );
    }
    return out;
  }, [nodeBase]);

  // === Highlight overlay (depends on selection / matches) ============
  // Drawn ON TOP of the dimmed static layer; usually only hundreds of
  // elements even on a graph of thousands.

  const highlightEdges = useMemo(() => {
    if (!hasSel) return null;
    const out: React.ReactNode[] = [];
    for (let i = 0; i < edgeGeo.length; i++) {
      const g = edgeGeo[i];
      if (!g || !g.visible) continue;
      let stroke: string | null = null;
      let dash: string | undefined;
      if (fwd.has(g.source) && fwd.has(g.target)) {
        stroke = PALETTE.fwd;
      } else if (impact.has(g.source) && impact.has(g.target)) {
        stroke = PALETTE.impact;
        dash = '5 3';
      }
      if (!stroke) continue;
      out.push(
        <line
          key={i}
          x1={g.ax} y1={g.ay} x2={g.bx} y2={g.by}
          stroke={stroke} strokeWidth={2} strokeOpacity={1}
          strokeDasharray={dash} markerEnd="url(#arrow2d)"
        />,
      );
    }
    return out;
  }, [edgeGeo, hasSel, fwd, impact]);

  const highlightNodes = useMemo(() => {
    if (!hasSel && matches.size === 0) return null;
    const out: React.ReactNode[] = [];
    for (const b of nodeBase) {
      if (!b) continue;
      // Determine the node's selection role.
      let role: 'sel' | 'fwd' | 'impact' | null = null;
      if (hasSel) {
        if (b.id === sel) role = 'sel';
        else if (fwd.has(b.id)) role = 'fwd';
        else if (impact.has(b.id)) role = 'impact';
      }
      const isMatch = matches.has(b.id);
      if (!role && !isMatch) continue;

      // Fill follows the role; falls back to the neutral hue when only the
      // match accent applies, so a searched-but-unselected node keeps its
      // importance colour under the accent ring.
      const fill =
        role === 'sel' ? PALETTE.center :
        role === 'fwd' ? PALETTE.fwd :
        role === 'impact' ? PALETTE.impact :
        (b.isRoot ? PALETTE.root : importanceColor(b.t));

      // Ring precedence: match accent > dup ring > neutral.
      const ring = isMatch ? PALETTE.accent : (b.dupNode ? PALETTE.dup : PALETTE.bg);
      const ringW = isMatch ? 3 : (b.dupNode ? 2.5 : 1.5);

      // A search hit that's NOT in the active selection's cone stays dimmed —
      // the accent ring signals "still a match", the low opacity signals "not
      // relevant to what you're looking at". Matches the original behaviour.
      const op = hasSel && !role && isMatch ? 0.2 : 1;

      out.push(
        <g key={b.id} className="nodeg on" data-id={b.id} opacity={op === 1 ? undefined : op}>
          <circle cx={b.px} cy={b.py} r={b.r} fill={fill} stroke={ring} strokeWidth={ringW} />
          <text className="on" x={b.px + b.r + 4} y={b.py + 4}>{shortLabel(b.purl)}</text>
        </g>,
      );
    }
    return out;
  }, [nodeBase, hasSel, sel, fwd, impact, matches]);

  // === Pan / wheel handlers ==========================================

  const onPointerDown = (e: React.PointerEvent) => {
    drag.current = { down: true, moved: 0, lastX: e.clientX, lastY: e.clientY, dragging: false, wasDrag: false };
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (d.down) {
      const dx = e.clientX - d.lastX, dy = e.clientY - d.lastY;
      d.lastX = e.clientX; d.lastY = e.clientY;
      d.moved += Math.abs(dx) + Math.abs(dy);
      if (!d.dragging && d.moved > 4) {
        d.dragging = true;
        try { svgRef.current?.setPointerCapture(e.pointerId); } catch { /* ignore */ }
        // Dropping any sticky hover the moment a drag begins matches the
        // user's mental model: panning is not hovering.
        if (lastHover.current !== null) { lastHover.current = null; setHovered(null); }
      }
      if (d.dragging) { T.current.x += dx; T.current.y += dy; apply(); }
      return;
    }
    // Not panning: delegated hover. `closest('g.nodeg')` walks up at most a few
    // levels (circle/text → nodeg), so this stays cheap per move event.
    const target = e.target as Element;
    const ng = target.closest?.('g.nodeg') as SVGGElement | null;
    const id = ng?.dataset.id != null ? Number(ng.dataset.id) : null;
    if (id !== lastHover.current) {
      lastHover.current = id;
      setHovered(id);
    }
  };

  const onPointerUp = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d.down) return;
    d.down = false;
    d.wasDrag = d.dragging;
    if (d.dragging) try { svgRef.current?.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
    d.dragging = false;
  };

  const onPointerLeave = () => {
    if (lastHover.current !== null) {
      lastHover.current = null;
      setHovered(null);
    }
  };

  const onClick = (e: React.MouseEvent) => {
    // A click at the end of a drag is just a pan release, not a selection.
    if (drag.current.wasDrag) { drag.current.wasDrag = false; return; }
    const target = e.target as Element;
    const ng = target.closest?.('g.nodeg') as SVGGElement | null;
    if (ng?.dataset.id != null) {
      e.stopPropagation();
      void selectNode(Number(ng.dataset.id));
    } else if (hasSel) {
      // Click landed on empty SVG background (or a non-node decoration —
      // edges, layer rules, labels). Treat as a "deselect" gesture so
      // users have an in-view way out of a selection, mirroring the
      // Command Palette's "Reset selection" action.
      clearSelection();
    }
  };

  const onWheel = (e: React.WheelEvent) => {
    const rect = svgRef.current!.getBoundingClientRect();
    zoomAt(e.clientX - rect.left, e.clientY - rect.top, e.deltaY < 0 ? 1.12 : 1 / 1.12);
  };

  // Re-sync the minimap viewport rect when its geometry changes (new graph/world).
  useEffect(() => { syncMini(); }, [syncMini, nodes]);

  // Recenter the main view on the clicked minimap point (also handles click-drag).
  const miniDrag = useRef(false);
  const miniJump = (e: React.PointerEvent<SVGSVGElement>) => {
    const rect = miniSvgRef.current!.getBoundingClientRect();
    const mx = e.clientX - rect.left, my = e.clientY - rect.top;
    const worldX = (mx - miniOffX) / miniScale;
    const worldY = (my - miniOffY) / miniScale;
    const { w, h } = size();
    const k = T.current.k;
    T.current.x = w / 2 - worldX * k;
    T.current.y = h / 2 - worldY * k;
    apply();
  };
  const onMiniDown = (e: React.PointerEvent<SVGSVGElement>) => {
    e.stopPropagation();
    miniDrag.current = true;
    try { miniSvgRef.current?.setPointerCapture(e.pointerId); } catch { /* ignore */ }
    miniJump(e);
  };
  const onMiniMove = (e: React.PointerEvent<SVGSVGElement>) => {
    if (!miniDrag.current) return;
    e.stopPropagation();
    miniJump(e);
  };
  const onMiniUp = (e: React.PointerEvent<SVGSVGElement>) => {
    if (!miniDrag.current) return;
    miniDrag.current = false;
    try { miniSvgRef.current?.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
  };

  // Clear a sticky hover/cursor when the view unmounts (e.g. switching to 3D).
  useEffect(() => () => { setHovered(null); }, [setHovered]);

  // Static minimap dots: one cheap <circle> per node, faintly tinted by importance.
  // Depends only on the layout/metrics, NOT on pan/zoom or selection, so it's
  // memoized once per graph and the viewport rect rides on top imperatively.
  // Radius is in WORLD units inside the scaled <g>, so divide by miniScale to keep
  // dots ~1px on screen even for big (heavily down-scaled) graphs.
  const miniDotR = Math.max(0.5, 0.9 / Math.max(1e-6, miniScale));
  const miniDots = useMemo(() => (
    nodes.map((n) => {
      const p = pos.get(n.id);
      if (!p) return null;
      const m = metrics?.get(n.id) as NodeMetrics | undefined;
      const fill = m ? importanceColor(importanceT(m, stats)) : PALETTE.edge;
      return <circle key={n.id} cx={p.x} cy={p.y} r={miniDotR} fill={fill} />;
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  ), [nodes, pos, metrics, stats, miniDotR]);

  // Viewport-group class drives the dimming of the static layer when a
  // selection or search is active. The overlay layer rides on top at full
  // opacity, so the highlighted set still pops.
  const vpClass = [hasSel ? 'has-sel' : '', searching ? 'searching' : ''].filter(Boolean).join(' ');

  return (
    <div className="absolute inset-0 dot-grid">
      <svg
        ref={svgRef}
        className={`g2d w-full h-full block touch-none ${small ? 'small' : ''}`}
        style={{ cursor: 'grab' }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onPointerLeave={onPointerLeave}
        onClick={onClick}
        onWheel={onWheel}
      >
        <defs>
          <marker id="arrow2d" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="5.5" markerHeight="5.5"
            markerUnits="userSpaceOnUse" orient="auto-start-reverse">
            <path d="M 0 0 L 10 5 L 0 10 z" fill={ARROW_FILL} />
          </marker>
        </defs>
        <g ref={vpRef} className={vpClass}>
          <g className="edges-neutral">{neutralEdges}</g>
          {highlightEdges && <g className="edges-overlay">{highlightEdges}</g>}
          <g className="nodes-neutral">{neutralNodes}</g>
          {highlightNodes && <g className="nodes-overlay">{highlightNodes}</g>}
        </g>
      </svg>

      <HoverCard />

      {/* On-canvas zoom / fit cluster (mirrors the 3D Reset affordance). */}
      <Hud className="absolute right-3.5 top-3.5 z-10 flex flex-col divide-y divide-graphite overflow-hidden">
        <button type="button" aria-label="Zoom in" title="Zoom in"
          onClick={() => zoomCenter(1.3)}
          className="w-7 h-7 grid place-items-center text-archive hover:text-bone hover:bg-iron text-h4 ease-instrument duration-150">+</button>
        <button type="button" aria-label="Zoom out" title="Zoom out"
          onClick={() => zoomCenter(1 / 1.3)}
          className="w-7 h-7 grid place-items-center text-archive hover:text-bone hover:bg-iron text-h4 ease-instrument duration-150">−</button>
        <button type="button" aria-label="Fit graph to view" title="Fit"
          onClick={fit}
          className="w-7 h-7 grid place-items-center text-archive hover:text-bone hover:bg-iron text-data ease-instrument duration-150">⊡</button>
      </Hud>

      {/* Overview minimap (bottom-right): static dots + a live viewport rectangle.
          Click / drag to recenter the main view on that world point. */}
      {nodes.length > 0 && (
        <Hud className="absolute right-3.5 bottom-3.5 z-10 overflow-hidden">
          <svg
            ref={miniSvgRef}
            width={MINI_W}
            height={MINI_H}
            className="block touch-none"
            style={{ cursor: 'crosshair' }}
            onPointerDown={onMiniDown}
            onPointerMove={onMiniMove}
            onPointerUp={onMiniUp}
            onPointerCancel={onMiniUp}
          >
            <g transform={`translate(${miniOffX} ${miniOffY}) scale(${miniScale})`}>
              {miniDots}
            </g>
            <rect
              ref={miniRectRef}
              x={0}
              y={0}
              width={0}
              height={0}
              fill={PALETTE.accent}
              fillOpacity={0.12}
              stroke={PALETTE.accent}
              strokeWidth={1}
              pointerEvents="none"
            />
          </svg>
        </Hud>
      )}
    </div>
  );
}

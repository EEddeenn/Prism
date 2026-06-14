'use client';

/* The 2D layered SVG view. Pan/zoom mutates the viewport <g> transform via a ref
 * (no React re-render on drag); React only re-renders when the graph or the
 * shared selection changes. Clicking a node selects it; hovering reports it to
 * the store (which re-renders only the DOM HoverCard, never this SVG). Node size
 * + neutral colour both encode IMPORTANCE (blast radius) via the shared scale. */

import { useCallback, useEffect, useMemo, useRef } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { PALETTE, shortLabel } from '@/lib/palette';
import { edgeVisible, isFlagged, kindColor } from '@/lib/edges';
import { importanceColor, importanceT, radius2D } from '@/lib/encoding';
import HoverCard from '@/components/HoverCard';
import type { MetricStats, NodeMetrics } from '@/lib/graph';

const MIN_K = 0.05, MAX_K = 6, LABEL_ALL_BELOW = 70, LABEL_ZOOM = 1.5;
// Fixed-corner overview minimap (bottom-right).
const MINI_W = 150, MINI_H = 110, MINI_PAD = 4;
const ZERO_STATS: MetricStats = {
  maxRevReach: 0, maxFwdReach: 0, maxInDeg: 0, maxOutDeg: 0, maxFamSize: 0, maxDepth: 0,
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
  const setHovered = useGraphStore((s) => s.setHovered);
  const focusNonce = useGraphStore((s) => s.focusNonce);
  const focusTarget = useGraphStore((s) => s.focusTarget);

  const svgRef = useRef<SVGSVGElement>(null);
  const vpRef = useRef<SVGGElement>(null);
  const miniSvgRef = useRef<SVGSVGElement>(null);
  const miniRectRef = useRef<SVGRectElement>(null);
  const T = useRef({ x: 0, y: 0, k: 1 });
  const drag = useRef({ down: false, moved: 0, lastX: 0, lastY: 0, dragging: false, wasDrag: false });

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

  const onPointerDown = (e: React.PointerEvent) => {
    drag.current = { down: true, moved: 0, lastX: e.clientX, lastY: e.clientY, dragging: false, wasDrag: false };
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d.down) return;
    const dx = e.clientX - d.lastX, dy = e.clientY - d.lastY;
    d.lastX = e.clientX; d.lastY = e.clientY;
    d.moved += Math.abs(dx) + Math.abs(dy);
    if (!d.dragging && d.moved > 4) {
      d.dragging = true;
      try { svgRef.current?.setPointerCapture(e.pointerId); } catch { /* ignore */ }
    }
    if (d.dragging) { T.current.x += dx; T.current.y += dy; apply(); }
  };
  const onPointerUp = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d.down) return;
    d.down = false;
    d.wasDrag = d.dragging;
    if (d.dragging) try { svgRef.current?.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
    d.dragging = false;
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

  const hasSel = sel != null;
  const searching = !hasSel && matches.size > 0;
  const small = nodes.length <= LABEL_ALL_BELOW;

  // Importance-driven node radius (blast radius), with a fixed fallback.
  const radiusOf = (id: number): number => {
    const m = metrics?.get(id) as NodeMetrics | undefined;
    return m ? radius2D(importanceT(m, stats)) : 10;
  };

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
        onWheel={onWheel}
      >
        <defs>
          <marker id="arrow2d" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="5.5" markerHeight="5.5"
            markerUnits="userSpaceOnUse" orient="auto-start-reverse">
            <path d="M 0 0 L 10 5 L 0 10 z" fill="#5b6478" />
          </marker>
        </defs>
        <g ref={vpRef}>
          {edges.map((e, i) => {
            const a = pos.get(e.source), b = pos.get(e.target);
            if (!a || !b) return null;
            const visible = edgeVisible(e, edgeFilter);
            // Terminate each edge at the node boundary so the arrowhead sits at
            // the rim regardless of the now-variable node radius.
            const dx = b.x - a.x, dy = b.y - a.y;
            const len = Math.hypot(dx, dy) || 1;
            const ux = dx / len, uy = dy / len;
            const sr = radiusOf(e.source), tr = radiusOf(e.target);
            const ax = a.x + ux * sr, ay = a.y + uy * sr;
            const bx = b.x - ux * (tr + 2), by = b.y - uy * (tr + 2);

            let stroke = PALETTE.edge, width = 1.3, opacity = 1, dash: string | undefined;
            if (!visible) {
              opacity = 0.05; // filtered out by the edge lens
            } else if (hasSel) {
              if (fwd.has(e.source) && fwd.has(e.target)) { stroke = PALETTE.fwd; width = 2; }
              else if (impact.has(e.source) && impact.has(e.target)) { stroke = PALETTE.impact; width = 2; dash = '5 3'; }
              else opacity = 0.14;
            } else {
              // Neutral state: colour by dependency kind; brighten flagged smells;
              // width tracks the target's blast radius so the backbone stands out.
              stroke = kindColor(e.kind);
              const tm = metrics?.get(e.target);
              const w = tm ? 1 + 1.6 * importanceT(tm, stats) : 1.3;
              if (isFlagged(e)) { opacity = 0.95; width = Math.max(1.8, w); } else { opacity = 0.5; width = w; }
            }
            return (
              <line key={`${e.source}>${e.target}:${i}`} x1={ax} y1={ay} x2={bx} y2={by}
                stroke={stroke} strokeWidth={width} strokeOpacity={opacity}
                strokeDasharray={dash} markerEnd="url(#arrow2d)" />
            );
          })}
          {nodes.map((n) => {
            const p = pos.get(n.id);
            if (!p) return null;
            const m = metrics?.get(n.id) as NodeMetrics | undefined;
            const t = m ? importanceT(m, stats) : 0;
            const r = m ? radius2D(t) : 10;
            // Neutral fill encodes importance; roots + selection roles override.
            let fill = rootIds?.has(n.id) ? PALETTE.root : importanceColor(t);
            let ring = '#0b0d12', ringW = 1.5, op = 1, on = false;
            if (dup.has(n.version_id)) { ring = PALETTE.dup; ringW = 2.5; }
            if (hasSel) {
              if (n.id === sel) { fill = PALETTE.center; on = true; }
              else if (fwd.has(n.id)) { fill = PALETTE.fwd; on = true; }
              else if (impact.has(n.id)) { fill = PALETTE.impact; on = true; }
              else op = 0.2;
            } else if (searching && !matches.has(n.id)) {
              op = 0.22; // an active search dims everything that doesn't match
            }
            if (matches.has(n.id)) { ring = PALETTE.accent; ringW = 3; on = true; }
            return (
              <g
                key={n.id}
                className="nodeg"
                opacity={op}
                onPointerEnter={() => { if (!drag.current.dragging) setHovered(n.id); }}
                onPointerLeave={() => setHovered(null)}
                onClick={(ev) => { ev.stopPropagation(); if (!drag.current.wasDrag) selectNode(n.id); }}
              >
                <circle cx={p.x} cy={p.y} r={r} fill={fill} stroke={ring} strokeWidth={ringW} />
                <text className={on ? 'on' : ''} x={p.x + r + 4} y={p.y + 4}>{shortLabel(n.purl)}</text>
              </g>
            );
          })}
        </g>
      </svg>

      <HoverCard />

      {/* On-canvas zoom / fit cluster (mirrors the 3D Reset affordance). */}
      <div className="absolute right-3.5 top-3.5 z-10 flex flex-col gap-1">
        <button type="button" aria-label="Zoom in" title="Zoom in"
          onClick={() => zoomCenter(1.3)}
          className="w-7 h-7 grid place-items-center rounded-md border border-line-2 bg-panel/90 text-muted hover:border-accent hover:text-ink text-head">+</button>
        <button type="button" aria-label="Zoom out" title="Zoom out"
          onClick={() => zoomCenter(1 / 1.3)}
          className="w-7 h-7 grid place-items-center rounded-md border border-line-2 bg-panel/90 text-muted hover:border-accent hover:text-ink text-head">−</button>
        <button type="button" aria-label="Fit graph to view" title="Fit"
          onClick={fit}
          className="w-7 h-7 grid place-items-center rounded-md border border-line-2 bg-panel/90 text-muted hover:border-accent hover:text-ink text-body">⊡</button>
      </div>

      {/* Overview minimap (bottom-right): static dots + a live viewport rectangle.
          Click / drag to recenter the main view on that world point. */}
      {nodes.length > 0 && (
        <div className="absolute right-3.5 bottom-3.5 z-10 rounded-md border border-line overflow-hidden bg-panel/85">
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
        </div>
      )}
    </div>
  );
}

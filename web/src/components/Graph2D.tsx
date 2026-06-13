'use client';

/* The 2D layered SVG view. Pan/zoom mutates the viewport <g> transform via a ref
 * (no React re-render on drag); React only re-renders when the graph or the
 * shared selection changes. Clicking a node selects it in the shared store. */

import { useCallback, useEffect, useRef } from 'react';
import { useGraphStore } from '@/store/useGraphStore';
import { PALETTE, shortLabel } from '@/lib/palette';
import { edgeVisible, isFlagged, kindColor } from '@/lib/edges';

const MIN_K = 0.05, MAX_K = 6, LABEL_ALL_BELOW = 70, LABEL_ZOOM = 1.5;

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
  const rootIds = useGraphStore((s) => s.index?.rootIds);
  const selectNode = useGraphStore((s) => s.selectNode);
  const focusNonce = useGraphStore((s) => s.focusNonce);
  const focusTarget = useGraphStore((s) => s.focusTarget);

  const svgRef = useRef<SVGSVGElement>(null);
  const vpRef = useRef<SVGGElement>(null);
  const T = useRef({ x: 0, y: 0, k: 1 });
  const drag = useRef({ down: false, moved: 0, lastX: 0, lastY: 0, dragging: false, wasDrag: false });

  const size = useCallback(() => {
    const svg = svgRef.current;
    return { w: svg?.clientWidth || 800, h: svg?.clientHeight || 600 };
  }, []);

  const apply = useCallback(() => {
    const { x, y, k } = T.current;
    vpRef.current?.setAttribute('transform', `translate(${x} ${y}) scale(${k})`);
    svgRef.current?.classList.toggle('zoomed', k >= LABEL_ZOOM);
  }, []);

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
  useEffect(() => {
    if (focusTarget == null) return;
    const p = pos.get(focusTarget);
    if (!p) return;
    const { w, h } = size();
    T.current.x = w / 2 - p.x * T.current.k;
    T.current.y = h / 2 - p.y * T.current.k;
    apply();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusNonce]);

  const zoomAt = (px: number, py: number, factor: number) => {
    const k0 = T.current.k;
    const k1 = Math.max(MIN_K, Math.min(MAX_K, k0 * factor));
    T.current.x = px - (px - T.current.x) * (k1 / k0);
    T.current.y = py - (py - T.current.y) * (k1 / k0);
    T.current.k = k1;
    apply();
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

  const hasSel = sel != null;
  const small = nodes.length <= LABEL_ALL_BELOW;

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
          <marker id="arrow2d" viewBox="0 0 10 10" refX="17" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
            <path d="M 0 0 L 10 5 L 0 10 z" fill="#4b5366" />
          </marker>
        </defs>
        <g ref={vpRef}>
          {edges.map((e, i) => {
            const a = pos.get(e.source), b = pos.get(e.target);
            if (!a || !b) return null;
            const visible = edgeVisible(e, edgeFilter);
            let stroke = PALETTE.edge, width = 1.3, opacity = 1;
            if (!visible) {
              opacity = 0.05; // filtered out by the edge lens
            } else if (hasSel) {
              if (fwd.has(e.source) && fwd.has(e.target)) { stroke = PALETTE.fwd; width = 2; }
              else if (impact.has(e.source) && impact.has(e.target)) { stroke = PALETTE.impact; width = 2; }
              else opacity = 0.18;
            } else {
              // Neutral state: colour by dependency kind; brighten flagged smells.
              stroke = kindColor(e.kind);
              if (isFlagged(e)) { opacity = 0.95; width = 1.8; } else opacity = 0.55;
            }
            return (
              <line key={i} x1={a.x} y1={a.y} x2={b.x} y2={b.y}
                stroke={stroke} strokeWidth={width} strokeOpacity={opacity} markerEnd="url(#arrow2d)" />
            );
          })}
          {nodes.map((n) => {
            const p = pos.get(n.id);
            if (!p) return null;
            let fill = PALETTE.node, ring = '#0b0d12', ringW = 1.5, op = 1, on = false;
            if (rootIds?.has(n.id)) fill = PALETTE.root;
            if (dup.has(n.version_id)) { ring = PALETTE.dup; ringW = 2.5; }
            if (hasSel) {
              if (n.id === sel) { fill = PALETTE.center; on = true; }
              else if (fwd.has(n.id)) { fill = PALETTE.fwd; on = true; }
              else if (impact.has(n.id)) { fill = PALETTE.impact; on = true; }
              else op = 0.22;
            }
            if (matches.has(n.id)) { ring = PALETTE.accent; ringW = 3; on = true; }
            return (
              <g
                key={n.id}
                className="nodeg"
                opacity={op}
                onClick={(ev) => { ev.stopPropagation(); if (!drag.current.wasDrag) selectNode(n.id); }}
              >
                <circle cx={p.x} cy={p.y} r={10} fill={fill} stroke={ring} strokeWidth={ringW} />
                <text className={on ? 'on' : ''} x={p.x + 14} y={p.y + 4}>{shortLabel(n.purl)}</text>
                <title>{`${n.purl}  (node ${n.id}, depth ${n.depth})`}</title>
              </g>
            );
          })}
        </g>
      </svg>
    </div>
  );
}

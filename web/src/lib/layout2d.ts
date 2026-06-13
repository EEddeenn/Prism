/* Deterministic 2D layered layout (component-tiled, barycenter-ordered) — the
 * same algorithm the original explorer used: split into connected components,
 * lay each out in BFS levels wrapped into grid bands (so a wide-shallow npm tree
 * becomes a block, not a line), then shelf-pack the component blocks. Pure
 * function of the export, so reloads are pixel-stable. O(n+m). Also serves as the
 * stable ground plane for the 3D terrain view. */

import type { GraphEdge, GraphNode } from './types';
import type { GraphIndex } from './graph';

export interface Layout2D {
  pos: Map<number, { x: number; y: number }>;
  world: { w: number; h: number };
}

interface Block {
  ids: number[];
  localPos: Map<number, { x: number; y: number }>;
  w: number;
  h: number;
  ox: number;
  oy: number;
}

export function layout2d(nodes: GraphNode[], edges: GraphEdge[], index: GraphIndex): Layout2D {
  const CELL_W = 150, CELL_H = 44, BAND_GAP = 120, BLOCK_PAD = 56, PAD = 40;
  const TARGET_ASPECT = 1.8, COMP_TARGET_ASPECT = 1.6, R = 10;
  const { byId, inn, rootIds } = index;

  if (nodes.length === 0) {
    return { pos: new Map(), world: { w: 1, h: 1 } };
  }

  // Undirected adjacency + connected components (deterministic order).
  const undirected = new Map<number, number[]>();
  for (const n of nodes) undirected.set(n.id, []);
  for (const e of edges) {
    if (undirected.has(e.source) && undirected.has(e.target)) {
      undirected.get(e.source)!.push(e.target);
      undirected.get(e.target)!.push(e.source);
    }
  }
  const compOf = new Map<number, number>();
  const components: number[][] = [];
  for (const n of nodes) {
    if (compOf.has(n.id)) continue;
    const compId = components.length;
    const ids: number[] = [];
    const stack = [n.id];
    compOf.set(n.id, compId);
    while (stack.length) {
      const cur = stack.pop()!;
      ids.push(cur);
      for (const nb of undirected.get(cur)!) {
        if (!compOf.has(nb)) {
          compOf.set(nb, compId);
          stack.push(nb);
        }
      }
    }
    components.push(ids);
  }

  const blocks: Block[] = [];
  for (const compIds of components) {
    let src = -1;
    for (const id of compIds) {
      if (rootIds.has(id) && (src === -1 || id < src)) src = id;
    }
    if (src === -1) for (const id of compIds) if (src === -1 || id < src) src = id;

    const level = new Map<number, number>();
    const queue = [src];
    level.set(src, 0);
    let qh = 0;
    while (qh < queue.length) {
      const cur = queue[qh++];
      const lv = level.get(cur)!;
      for (const nb of undirected.get(cur)!) {
        if (!level.has(nb)) {
          level.set(nb, lv + 1);
          queue.push(nb);
        }
      }
    }
    for (const id of compIds) if (!level.has(id)) level.set(id, byId.get(id)!.depth);

    const localPos = new Map<number, { x: number; y: number }>();
    if (compIds.length === 1) {
      localPos.set(compIds[0], { x: 0, y: 0 });
      blocks.push({ ids: [...compIds], localPos, w: 2 * R, h: 2 * R, ox: 0, oy: 0 });
      continue;
    }

    const byLvl = new Map<number, number[]>();
    for (const id of compIds) {
      const lv = level.get(id)!;
      if (!byLvl.has(lv)) byLvl.set(lv, []);
      byLvl.get(lv)!.push(id);
    }
    const levels = Array.from(byLvl.keys()).sort((a, b) => a - b);
    const Lc = levels.length;
    const Nc = compIds.length;
    let maxLayer = 0;
    for (const lv of levels) maxLayer = Math.max(maxLayer, byLvl.get(lv)!.length);

    let bandRows = Math.round(Math.sqrt((Nc * CELL_W) / (TARGET_ASPECT * CELL_H)));
    const loClamp = Math.min(4, maxLayer);
    if (bandRows < loClamp) bandRows = loClamp;
    if (bandRows > maxLayer) bandRows = maxLayer;

    let estW = Lc * (CELL_W + BAND_GAP);
    for (const lv of levels) estW += (Math.ceil(byLvl.get(lv)!.length / bandRows) - 1) * CELL_W;
    const wantRows = Math.ceil(estW / (TARGET_ASPECT * CELL_H));
    bandRows = Math.max(bandRows, Math.min(wantRows, maxLayer));

    const rowOf = new Map<number, number>();
    const idxInLvl = new Map<number, number>();
    let bandX = 0;
    let minx = Infinity, miny = Infinity, maxx = -Infinity, maxy = -Infinity;
    for (const lv of levels) {
      const nodesAtLvl = byLvl.get(lv)!;
      nodesAtLvl.forEach((id, i) => idxInLvl.set(id, i));
      const bary = (id: number): number => {
        const parents = inn.get(id) || [];
        let sum = 0, cnt = 0;
        for (const p of parents) {
          if (rowOf.has(p)) {
            sum += rowOf.get(p)!;
            cnt++;
          }
        }
        return cnt ? sum / cnt : idxInLvl.get(id)!;
      };
      nodesAtLvl.sort((a, b) => bary(a) - bary(b) || a - b);
      const count = nodesAtLvl.length;
      const rows = Math.max(1, Math.min(bandRows, count));
      const subCols = Math.ceil(count / rows);
      nodesAtLvl.forEach((id, i) => {
        const sc = Math.floor(i / rows);
        const sr = i % rows;
        const lx = bandX + sc * CELL_W;
        const ly = sr * CELL_H;
        localPos.set(id, { x: lx, y: ly });
        rowOf.set(id, sc * rows + sr);
        if (lx < minx) minx = lx;
        if (ly < miny) miny = ly;
        if (lx > maxx) maxx = lx;
        if (ly > maxy) maxy = ly;
      });
      bandX += subCols * CELL_W + BAND_GAP;
    }
    for (const p of localPos.values()) {
      p.x -= minx;
      p.y -= miny;
    }
    blocks.push({
      ids: [...compIds],
      localPos,
      w: maxx - minx + 2 * R,
      h: maxy - miny + 2 * R,
      ox: 0,
      oy: 0,
    });
  }

  let worldW = 0, worldH = 0;
  if (blocks.length === 1) {
    blocks[0].ox = 0;
    blocks[0].oy = 0;
    worldW = blocks[0].w;
    worldH = blocks[0].h;
  } else {
    blocks.sort(
      (a, b) => b.w * b.h - a.w * a.h || b.h - a.h || b.w - a.w || a.ids[0] - b.ids[0],
    );
    let widest = 0, paddedArea = 0;
    for (const blk of blocks) {
      if (blk.w > widest) widest = blk.w;
      paddedArea += (blk.w + BLOCK_PAD) * (blk.h + BLOCK_PAD);
    }
    const targetW = Math.max(widest, Math.sqrt(paddedArea * COMP_TARGET_ASPECT));
    let cx = 0, cy = 0, shelfH = 0;
    for (const blk of blocks) {
      if (cx > 0 && cx + blk.w > targetW) {
        cx = 0;
        cy += shelfH + BLOCK_PAD;
        shelfH = 0;
      }
      blk.ox = cx;
      blk.oy = cy;
      cx += blk.w + BLOCK_PAD;
      if (blk.h > shelfH) shelfH = blk.h;
      if (cx - BLOCK_PAD > worldW) worldW = cx - BLOCK_PAD;
      if (cy + shelfH > worldH) worldH = cy + shelfH;
    }
  }

  const pos = new Map<number, { x: number; y: number }>();
  for (const blk of blocks) {
    for (const id of blk.ids) {
      const p = blk.localPos.get(id)!;
      pos.set(id, { x: PAD + blk.ox + p.x, y: PAD + blk.oy + p.y });
    }
  }
  return { pos, world: { w: worldW + 2 * PAD, h: worldH + 2 * PAD } };
}

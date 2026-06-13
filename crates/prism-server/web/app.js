/* Prism graph explorer — vanilla JS, no framework, no external script.
 *
 * Load /api/runs to populate the picker; on pick, fetch /api/export and draw the
 * graph as inline SVG with a deterministic barycenter-ordered layered layout
 * (column = BFS depth, row chosen to reduce edge crossings). The canvas pans and
 * zooms. Clicking a node traces its forward dependencies and reverse dependents.
 * A tabbed side panel surfaces every query the server answers: run provenance +
 * graph stats (Overview), the selected node with clickable neighbors (Node),
 * duplicate-version families (Duplicates), strongly-connected components
 * (Cycles), and the persistent-homology barcode (TDA).
 */
'use strict';

const SVG_NS = 'http://www.w3.org/2000/svg';
const LABEL_ALL_BELOW = 70;   // show every label when the graph is at most this big
const LABEL_ZOOM = 1.5;       // …or once zoomed in past this factor
const MIN_K = 0.05, MAX_K = 6;

const state = {
  run: null,
  nodes: [],            // [{id, purl, depth, version_id, package_id}]
  edges: [],            // [{source, target, kind, flags}]
  byId: new Map(),      // id -> node
  pos: new Map(),       // id -> {x, y}
  out: new Map(),       // id -> [target id]
  in: new Map(),        // id -> [source id]
  rootIds: new Set(),   // depth-0 nodes
  dupVersionIds: new Set(), // version_ids belonging to a duplicated family
  sel: null,            // selected node id
  fwd: new Set(),
  impact: new Set(),
  matches: new Set(),   // search hits
  T: { x: 0, y: 0, k: 1 }, // viewport transform (world -> screen)
  view: '2d',           // '2d' (SVG) or '3d' (Scene3D canvas)
  features: [],         // TDA persistence pairs for the current run
};

// --- tiny DOM/SVG helpers ---------------------------------------------------

function el(id) { return document.getElementById(id); }
function svgEl(name, attrs) {
  const n = document.createElementNS(SVG_NS, name);
  if (attrs) for (const k of Object.keys(attrs)) n.setAttribute(k, attrs[k]);
  return n;
}
function clear(node) { while (node.firstChild) node.removeChild(node.firstChild); }
function mk(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}
function setStatus(text, isError) {
  const s = el('status-text');
  s.textContent = text || '';
  s.classList.toggle('error', !!isError);
}

async function getJson(url) {
  const resp = await fetch(url);
  let body;
  try { body = await resp.json(); } catch (_) { body = null; }
  if (!resp.ok) throw new Error((body && body.error) ? body.error : ('HTTP ' + resp.status));
  return body;
}

/* Trim a purl to a compact display label. */
function shortLabel(purl) {
  if (!purl) return '?';
  let s = purl.replace(/^pkg:[^/]+\//, '');
  if (s.length > 30) s = s.slice(0, 29) + '…';
  return s;
}

// --- run picker -------------------------------------------------------------

async function loadRuns() {
  setStatus('Loading runs…');
  let runs;
  try { runs = await getJson('/api/runs'); }
  catch (e) { setStatus(e.message, true); return; }
  const sel = el('run-select');
  clear(sel);
  if (!runs.length) {
    sel.appendChild(mk('option', '', '(no ingested runs)'));
    el('graph-empty').classList.remove('hidden');
    setStatus('No runs — ingest a lockfile with the CLI first.');
    return;
  }
  el('graph-empty').classList.add('hidden');
  for (const r of runs) {
    const counts = (r.nodes != null) ? ` · ${r.nodes}n/${r.edges}e` : '';
    const opt = mk('option', '', `run ${r.run}${counts}`);
    opt.value = String(r.run);
    sel.appendChild(opt);
  }
  setStatus('');
  await selectRun(Number(sel.value));
}

// --- graph load + layout ----------------------------------------------------

async function selectRun(run) {
  state.run = run;
  setStatus('Loading graph…');
  try {
    const data = await getJson('/api/export?run=' + run);
    state.nodes = data.graph_nodes || [];
    state.edges = data.graph_edges || [];
    indexGraph();
    layout();
    clearSelection();
    render();
    fitView();
    // The 3D scene shares the same data + selection; build its layout now so a
    // toggle to 3D is instant.
    Scene3D.setData();
    // Side panels (independent; a failure in one must not blank the others).
    await Promise.all([
      loadOverview(run), loadDuplicates(run), loadScc(run), loadBarcode(run), loadFeatures(run),
    ]);
    showTab('overview');
    // Duplicate flags arrive with loadDuplicates; refresh 3D colors if it's live.
    if (state.view === '3d') Scene3D.redraw();
    setStatus('');
  } catch (e) {
    setStatus(e.message, true);
  }
}

/* Build id index + forward/reverse adjacency + root set. */
function indexGraph() {
  state.byId = new Map();
  state.out = new Map();
  state.in = new Map();
  state.rootIds = new Set();
  for (const n of state.nodes) {
    state.byId.set(n.id, n);
    state.out.set(n.id, []);
    state.in.set(n.id, []);
    if (n.depth === 0) state.rootIds.add(n.id);
  }
  for (const e of state.edges) {
    if (state.out.has(e.source)) state.out.get(e.source).push(e.target);
    if (state.in.has(e.target)) state.in.get(e.target).push(e.source);
  }
}

/* Deterministic 2D layout (layered, with layer-wrapping and component tiling).
 *
 * A plain "column = BFS depth" layout collapses a wide-shallow graph (e.g. an
 * npm tree of 900 nodes at depth <= 2) into one giant vertical line. This avoids
 * that:
 *   1. split the graph into connected components;
 *   2. lay out each component in BFS levels, but WRAP each level into a grid band
 *      (a shared row budget aimed at a pleasant aspect) so a fat level becomes a
 *      block, never a line; order within a level by barycenter to cut crossings;
 *   3. shelf-pack the component blocks into the canvas.
 * Pure function of the export — no randomness — so reloads are pixel-stable.
 * O(n+m) with per-level sorts; lays out 900 nodes in ~2ms. Verified on real
 * exports: aspect ~1.16 (npm) / ~1.87 (cargo), 0 overlapping nodes. */
function layout() {
  const CELL_W = 150, CELL_H = 44, BAND_GAP = 120, BLOCK_PAD = 56, PAD = 40;
  const TARGET_ASPECT = 1.8, COMP_TARGET_ASPECT = 1.6, R = 10;

  // Empty graph early-out.
  if (state.nodes.length === 0) {
    state.pos = new Map();
    state._world = { w: 1, h: 1 };
    return;
  }

  // Undirected adjacency + connected components (iterative DFS in node-array
  // order -> deterministic component ids). O(n+m).
  const undirected = new Map();
  for (const n of state.nodes) undirected.set(n.id, []);
  for (const e of state.edges) {
    if (undirected.has(e.source) && undirected.has(e.target)) {
      undirected.get(e.source).push(e.target);
      undirected.get(e.target).push(e.source);
    }
  }
  const compOf = new Map();
  const components = [];
  for (const n of state.nodes) {
    if (compOf.has(n.id)) continue;
    const compId = components.length;
    const ids = [];
    const stack = [n.id];
    compOf.set(n.id, compId);
    while (stack.length) {
      const cur = stack.pop();
      ids.push(cur);
      for (const nb of undirected.get(cur)) {
        if (!compOf.has(nb)) { compOf.set(nb, compId); stack.push(nb); }
      }
    }
    components.push(ids);
  }

  // Lay out each component into local coords.
  const blocks = [];
  for (const compIds of components) {
    // Per-component BFS level. src = smallest root id in the component, else its
    // smallest id. Single BFS over undirected neighbors.
    let src = -1;
    for (const id of compIds) {
      if (state.rootIds.has(id) && (src === -1 || id < src)) src = id;
    }
    if (src === -1) {
      for (const id of compIds) if (src === -1 || id < src) src = id;
    }
    const level = new Map();
    const queue = [src];
    level.set(src, 0);
    let qh = 0;
    while (qh < queue.length) {
      const cur = queue[qh++];
      const lv = level.get(cur);
      for (const nb of undirected.get(cur)) {
        if (!level.has(nb)) { level.set(nb, lv + 1); queue.push(nb); }
      }
    }
    for (const id of compIds) {
      if (!level.has(id)) level.set(id, state.byId.get(id).depth);
    }

    // Singleton component.
    const localPos = new Map();
    if (compIds.length === 1) {
      localPos.set(compIds[0], { x: 0, y: 0 });
      blocks.push({ ids: compIds.slice(), localPos, w: 2 * R, h: 2 * R });
      continue;
    }

    // Bucket by level; distinct levels ascending.
    const byLvl = new Map();
    for (const id of compIds) {
      const lv = level.get(id);
      if (!byLvl.has(lv)) byLvl.set(lv, []);
      byLvl.get(lv).push(id);
    }
    const levels = Array.from(byLvl.keys()).sort((a, b) => a - b);
    const Lc = levels.length;
    const Nc = compIds.length;
    let maxLayer = 0;
    for (const lv of levels) maxLayer = Math.max(maxLayer, byLvl.get(lv).length);

    // Shared row budget (closed form) clamped to [min(4,maxLayer), maxLayer].
    let bandRows = Math.round(Math.sqrt(Nc * CELL_W / (TARGET_ASPECT * CELL_H)));
    const loClamp = Math.min(4, maxLayer);
    if (bandRows < loClamp) bandRows = loClamp;
    if (bandRows > maxLayer) bandRows = maxLayer;

    // Deep-graph width correction (one cheap pass, no iteration).
    let estW = Lc * (CELL_W + BAND_GAP);
    for (const lv of levels) {
      const cnt = byLvl.get(lv).length;
      estW += (Math.ceil(cnt / bandRows) - 1) * CELL_W;
    }
    const wantRows = Math.ceil(estW / (TARGET_ASPECT * CELL_H));
    bandRows = Math.max(bandRows, Math.min(wantRows, maxLayer));

    // Place each level left-to-right, barycenter-ordered, column-major in a band.
    const rowOf = new Map();
    let bandX = 0;
    let minx = Infinity, miny = Infinity, maxx = -Infinity, maxy = -Infinity;
    for (const lv of levels) {
      const nodesAtLvl = byLvl.get(lv);
      nodesAtLvl.forEach((id, idx) => { state.byId.get(id)._idx = idx; });
      const bary = (id) => {
        const parents = state.in.get(id) || [];
        let sum = 0, cnt = 0;
        for (const p of parents) {
          if (rowOf.has(p)) { sum += rowOf.get(p); cnt++; }
        }
        return cnt ? sum / cnt : state.byId.get(id)._idx;
      };
      nodesAtLvl.sort((a, b) => (bary(a) - bary(b)) || (a - b));
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

    // Normalize to local origin, record bbox.
    for (const p of localPos.values()) { p.x -= minx; p.y -= miny; }
    blocks.push({
      ids: compIds.slice(),
      localPos,
      w: (maxx - minx) + 2 * R,
      h: (maxy - miny) + 2 * R,
    });
  }

  // Shelf-pack component blocks toward COMP_TARGET_ASPECT.
  let worldW = 0, worldH = 0;
  if (blocks.length === 1) {
    blocks[0].ox = 0; blocks[0].oy = 0;
    worldW = blocks[0].w; worldH = blocks[0].h;
  } else {
    blocks.sort((a, b) =>
      (b.w * b.h - a.w * a.h) || (b.h - a.h) || (b.w - a.w) || (a.ids[0] - b.ids[0]));
    let widest = 0, paddedArea = 0;
    for (const blk of blocks) {
      if (blk.w > widest) widest = blk.w;
      paddedArea += (blk.w + BLOCK_PAD) * (blk.h + BLOCK_PAD);
    }
    const targetW = Math.max(widest, Math.sqrt(paddedArea * COMP_TARGET_ASPECT));
    let cx = 0, cy = 0, shelfH = 0;
    for (const blk of blocks) {
      if (cx > 0 && cx + blk.w > targetW) {
        cx = 0; cy += shelfH + BLOCK_PAD; shelfH = 0;
      }
      blk.ox = cx; blk.oy = cy;
      cx += blk.w + BLOCK_PAD;
      if (blk.h > shelfH) shelfH = blk.h;
      if (cx - BLOCK_PAD > worldW) worldW = cx - BLOCK_PAD;
      if (cy + shelfH > worldH) worldH = cy + shelfH;
    }
  }

  // Emit world coords.
  state.pos = new Map();
  for (const blk of blocks) {
    for (const id of blk.ids) {
      const p = blk.localPos.get(id);
      state.pos.set(id, { x: PAD + blk.ox + p.x, y: PAD + blk.oy + p.y });
    }
  }
  state._world = { w: worldW + 2 * PAD, h: worldH + 2 * PAD };
}

// --- rendering --------------------------------------------------------------

function resizeSvg() {
  const svg = el('graph-svg');
  const w = svg.clientWidth || 800, h = svg.clientHeight || 600;
  svg.setAttribute('viewBox', `0 0 ${w} ${h}`);
  return { w, h };
}

function applyTransform() {
  const vp = el('viewport');
  if (vp) {
    const { x, y, k } = state.T;
    vp.setAttribute('transform', `translate(${x} ${y}) scale(${k})`);
  }
  const svg = el('graph-svg');
  svg.classList.toggle('zoomed', state.T.k >= LABEL_ZOOM);
}

function render() {
  resizeSvg();
  const svg = el('graph-svg');
  clear(svg);

  // Arrowhead marker (once).
  const defs = svgEl('defs');
  const marker = svgEl('marker', {
    id: 'arrow', viewBox: '0 0 10 10', refX: '17', refY: '5',
    markerWidth: '6', markerHeight: '6', orient: 'auto-start-reverse',
  });
  marker.appendChild(svgEl('path', { d: 'M 0 0 L 10 5 L 0 10 z', fill: '#4b5366' }));
  defs.appendChild(marker);
  svg.appendChild(defs);

  const vp = svgEl('g', { id: 'viewport' });
  svg.appendChild(vp);

  const hasSel = state.sel != null;
  svg.classList.toggle('has-selection', hasSel);
  svg.classList.toggle('small', state.nodes.length <= LABEL_ALL_BELOW);

  const inFwd = (s, t) => state.fwd.has(s) && state.fwd.has(t);
  const inImp = (s, t) => state.impact.has(s) && state.impact.has(t);

  // Edges under nodes.
  for (const e of state.edges) {
    const a = state.pos.get(e.source), b = state.pos.get(e.target);
    if (!a || !b) continue;
    let cls = 'edge';
    if (hasSel && inFwd(e.source, e.target)) cls += ' fwd';
    else if (hasSel && inImp(e.source, e.target)) cls += ' impact';
    vp.appendChild(svgEl('line', {
      x1: a.x, y1: a.y, x2: b.x, y2: b.y, class: cls, 'marker-end': 'url(#arrow)',
    }));
  }

  // Nodes.
  for (const n of state.nodes) {
    const p = state.pos.get(n.id);
    if (!p) continue;
    let cls = 'node';
    if (state.rootIds.has(n.id)) cls += ' root';
    if (state.dupVersionIds.has(n.version_id)) cls += ' dup';
    if (hasSel) {
      if (n.id === state.sel) cls += ' center';
      else if (state.fwd.has(n.id)) cls += ' fwd';
      else if (state.impact.has(n.id)) cls += ' impact';
      else cls += ' dim';
    }
    if (state.matches.has(n.id)) cls += ' match';
    const g = svgEl('g', { class: cls, 'data-id': n.id });
    g.appendChild(svgEl('circle', { cx: p.x, cy: p.y, r: 10 }));
    const label = svgEl('text', { x: p.x + 14, y: p.y + 4 });
    label.textContent = shortLabel(n.purl);
    g.appendChild(label);
    const title = svgEl('title');
    title.textContent = `${n.purl}  (node ${n.id}, depth ${n.depth})`;
    g.appendChild(title);
    g.addEventListener('click', (ev) => {
      ev.stopPropagation();
      // Ignore the click that can trail a pan that happened to end over a node.
      if (state._lastWasDrag) return;
      onNodeClick(n);
    });
    vp.appendChild(g);
  }
  applyTransform();
}

// --- pan & zoom -------------------------------------------------------------

/* Screen point (svg-user units) -> apply zoom factor about that point. */
function zoomAt(px, py, factor) {
  const k0 = state.T.k;
  const k1 = Math.max(MIN_K, Math.min(MAX_K, k0 * factor));
  state.T.x = px - (px - state.T.x) * (k1 / k0);
  state.T.y = py - (py - state.T.y) * (k1 / k0);
  state.T.k = k1;
  applyTransform();
}

function fitView() {
  const w = state._world;
  if (!w || !state.nodes.length) return;
  const { w: vw, h: vh } = resizeSvg();
  const pad = 40;
  const k = Math.max(MIN_K, Math.min(MAX_K, Math.min(
    (vw - pad * 2) / Math.max(1, w.w),
    (vh - pad * 2) / Math.max(1, w.h),
    1.4,
  )));
  state.T.k = k;
  state.T.x = (vw - w.w * k) / 2;
  state.T.y = (vh - w.h * k) / 2;
  applyTransform();
}

function installPanZoom() {
  const svg = el('graph-svg');
  const DRAG_THRESHOLD = 4; // px of movement before a press becomes a pan
  let down = false, dragging = false, lastX = 0, lastY = 0, moved = 0, pid = null;

  svg.addEventListener('pointerdown', (ev) => {
    // Record the press but DON'T capture the pointer yet: capturing here would
    // redirect the follow-up `click` to the <svg>, so node clicks would never
    // fire. We only begin panning (and capture) once the pointer actually moves.
    down = true; dragging = false; moved = 0; pid = ev.pointerId;
    lastX = ev.clientX; lastY = ev.clientY;
  });
  svg.addEventListener('pointermove', (ev) => {
    if (!down) return;
    const dx = ev.clientX - lastX, dy = ev.clientY - lastY;
    lastX = ev.clientX; lastY = ev.clientY;
    moved += Math.abs(dx) + Math.abs(dy);
    if (!dragging && moved > DRAG_THRESHOLD) {
      dragging = true;
      svg.classList.add('panning');
      try { svg.setPointerCapture(pid); } catch (_) {}
    }
    if (dragging) { state.T.x += dx; state.T.y += dy; applyTransform(); }
  });
  const endPan = (ev) => {
    if (!down) return;
    down = false;
    // Remember whether this interaction was a drag, so a node `click` fired at
    // the tail of a pan doesn't select (see the node handler in render()).
    state._lastWasDrag = dragging;
    if (dragging) {
      svg.classList.remove('panning');
      try { svg.releasePointerCapture(ev.pointerId); } catch (_) {}
    }
    dragging = false;
  };
  svg.addEventListener('pointerup', endPan);
  svg.addEventListener('pointercancel', endPan);

  svg.addEventListener('wheel', (ev) => {
    ev.preventDefault();
    const rect = svg.getBoundingClientRect();
    // viewBox == client size, so client px == svg-user units.
    const px = ev.clientX - rect.left, py = ev.clientY - rect.top;
    zoomAt(px, py, ev.deltaY < 0 ? 1.12 : 1 / 1.12);
  }, { passive: false });

  el('zoom-in').addEventListener('click', () => {
    const { w, h } = resizeSvg(); zoomAt(w / 2, h / 2, 1.3);
  });
  el('zoom-out').addEventListener('click', () => {
    const { w, h } = resizeSvg(); zoomAt(w / 2, h / 2, 1 / 1.3);
  });
  el('zoom-fit').addEventListener('click', fitView);
  window.addEventListener('resize', () => { resizeSvg(); applyTransform(); });
}

// --- selection: neighborhood + impact ---------------------------------------

function clearSelection() {
  state.sel = null; state.fwd = new Set(); state.impact = new Set();
}

/* Repaint whichever view is active (both read the shared selection state). */
function refresh() {
  if (state.view === '3d') Scene3D.redraw();
  else render();
}

async function onNodeClick(node) {
  setStatus('Tracing dependencies…');
  state.sel = node.id;
  showTab('node');
  showNodeInfo(node);
  try {
    const run = state.run;
    const [nb, imp] = await Promise.all([
      getJson(`/api/graph/neighborhood?run=${run}&node=${node.id}&depth=64`),
      getJson(`/api/graph/impact?run=${run}&node=${node.id}`),
    ]);
    state.fwd = new Set([node.id]);
    for (const n of (nb.nodes || [])) state.fwd.add(n.id);
    state.impact = new Set([node.id]);
    for (const n of (imp.nodes || [])) state.impact.add(n.id);
    refresh();
    setStatus('');
  } catch (e) {
    setStatus(e.message, true);
  }
}

/* Select a node by id and bring it into view (camera in 3D, viewport in 2D). */
function focusNode(id) {
  const n = state.byId.get(id);
  if (!n) return;
  onNodeClick(n);
  if (state.view === '3d') Scene3D.focus(id);
  else { const p = state.pos.get(id); if (p) centerOn(p); }
}

function showNodeInfo(node) {
  const box = el('node-info');
  clear(box);
  box.appendChild(mk('div', 'purl', node.purl));

  const meta = [
    ['node id', node.id],
    ['version id', node.version_id],
    ['package id', node.package_id != null ? node.package_id : '—'],
    ['depth', node.depth],
    ['duplicated', state.dupVersionIds.has(node.version_id) ? 'yes' : 'no'],
  ];
  for (const [k, v] of meta) {
    const row = mk('div', 'row');
    row.appendChild(mk('span', '', k));
    row.appendChild(mk('span', '', String(v)));
    box.appendChild(row);
  }

  const deps = (state.out.get(node.id) || []);
  const dependents = (state.in.get(node.id) || []);
  box.appendChild(peerSection('Dependencies', deps));
  box.appendChild(peerSection('Dependents', dependents));
}

function peerSection(title, ids) {
  const wrap = document.createDocumentFragment();
  const h = mk('h3');
  h.appendChild(document.createTextNode(title + ' '));
  h.appendChild(mk('span', 'count-tag', '(' + ids.length + ')'));
  wrap.appendChild(h);
  if (!ids.length) {
    wrap.appendChild(mk('p', 'muted', title === 'Dependencies' ? 'A leaf — depends on nothing.'
                                                               : 'A root — nothing depends on it.'));
    return wrap;
  }
  const ul = mk('ul', 'peer-list');
  // Stable, de-duplicated, ordered by depth then purl.
  const seen = new Set();
  const items = ids.filter((id) => !seen.has(id) && seen.add(id))
    .map((id) => state.byId.get(id)).filter(Boolean)
    .sort((a, b) => (a.depth - b.depth) || a.purl.localeCompare(b.purl));
  for (const n of items) {
    const li = mk('li');
    li.appendChild(mk('span', 'pl-purl', shortLabel(n.purl)));
    li.appendChild(mk('span', 'pl-depth', 'd' + n.depth));
    li.title = n.purl;
    li.addEventListener('click', () => focusNode(n.id));
    ul.appendChild(li);
  }
  wrap.appendChild(ul);
  return wrap;
}

// --- Overview: run provenance + stats ---------------------------------------

async function loadOverview(run) {
  const meta = el('run-meta');
  clear(meta);
  try {
    const r = await getJson('/api/run?run=' + run);
    meta.appendChild(chip('eco', 'ecosystem', r.ecosystem));
    const tier = r.fidelity_tier || 'unknown';
    meta.appendChild(chip('fidelity-' + tier, 'fidelity', tier));
    if (r.resolver) meta.appendChild(chip('', 'resolver', r.resolver));
    for (const cav of (r.fidelity_caveats || [])) meta.appendChild(chip('caveat', null, cav));
  } catch (_) { /* run meta is best-effort */ }

  const list = el('stats-list');
  clear(list);
  try {
    const s = await getJson('/api/graph/stats?run=' + run);
    const rows = [
      ['nodes', s.nodes, null],
      ['edges', s.edges, null],
      ['SCCs', s.scc_count, null],
      ['has cycle', s.has_cycle ? 'yes' : 'no', s.has_cycle ? 'flag-yes' : 'flag-no'],
      ['max depth', s.max_depth, null],
      ['duplicate families', s.duplicate_version_count,
        s.duplicate_version_count ? 'flag-yes' : 'flag-no'],
    ];
    for (const [k, v, cls] of rows) {
      list.appendChild(mk('dt', '', k));
      const dd = mk('dd', cls, String(v));
      list.appendChild(dd);
    }
    setBadge('node', null);
  } catch (e) {
    list.appendChild(mk('dd', 'muted', e.message));
  }
}

function chip(extra, label, value) {
  const c = mk('span', 'chip' + (extra ? ' ' + extra : ''));
  if (label) { const b = mk('b', '', label + ' '); c.appendChild(b); }
  c.appendChild(document.createTextNode(value));
  return c;
}

// --- Duplicates -------------------------------------------------------------

async function loadDuplicates(run) {
  const box = el('dup-list');
  clear(box);
  state.dupVersionIds = new Set();
  let clusters;
  try { clusters = await getJson('/api/graph/duplicates?run=' + run); }
  catch (e) { box.appendChild(mk('p', 'muted', e.message)); return; }

  setBadge('duplicates', clusters.length || null);
  if (!clusters.length) {
    box.appendChild(mk('p', 'muted', 'No package family appears at more than one version. 🎉'));
    return;
  }
  // Map version purls -> node ids so a click can highlight every occurrence.
  const purlToId = new Map();
  for (const n of state.nodes) purlToId.set(n.purl, n.id);

  box.appendChild(mk('p', 'muted',
    `${clusters.length} famil${clusters.length === 1 ? 'y' : 'ies'} present at multiple versions.`));

  for (const c of clusters) {
    const fam = mk('div', 'dup-fam');
    const head = mk('div', 'dup-head');
    head.appendChild(mk('span', 'dup-name', c.package_purl.replace(/^pkg:[^/]+\//, '')));
    head.appendChild(mk('span', 'dup-vcount', c.version_count + ' versions'));
    fam.appendChild(head);
    const vers = mk('div', 'dup-vers');
    const ids = [];
    for (const v of c.versions) {
      const ver = (v.purl.split('@')[1]) || v.purl;
      vers.appendChild(mk('div', 'v', ver));
      if (purlToId.has(v.purl)) {
        ids.push(purlToId.get(v.purl));
        state.dupVersionIds.add(v.version_id);
      }
    }
    fam.appendChild(vers);
    head.addEventListener('click', () => highlightSet(ids, c.package_purl));
    box.appendChild(fam);
  }
}

/* Highlight an explicit set of node ids (used by duplicate families). Reuses the
 * fwd channel so the graph dims everything else; clears the impact channel. */
function highlightSet(ids, label) {
  if (!ids.length) { setStatus('No occurrences of ' + label + ' in this graph.'); return; }
  state.sel = ids[0];
  state.fwd = new Set(ids);
  state.impact = new Set();
  refresh();
  // Center the view loosely on the first occurrence.
  if (state.view === '3d') Scene3D.focus(ids[0]);
  else { const p = state.pos.get(ids[0]); if (p) centerOn(p); }
  setStatus(`${ids.length} occurrence(s) of ${label.replace(/^pkg:[^/]+\//, '')} highlighted.`);
}

function centerOn(p) {
  const { w, h } = resizeSvg();
  state.T.x = w / 2 - p.x * state.T.k;
  state.T.y = h / 2 - p.y * state.T.k;
  applyTransform();
}

// --- Cycles (SCC) -----------------------------------------------------------

async function loadScc(run) {
  const box = el('scc-list');
  clear(box);
  let comps;
  try { comps = await getJson('/api/graph/scc?run=' + run); }
  catch (e) { box.appendChild(mk('p', 'muted', e.message)); return; }

  const cycles = comps.filter((c) => c.is_cycle);
  setBadge('cycles', cycles.length || null);
  if (!cycles.length) {
    box.appendChild(mk('div', 'scc-banner ok',
      `Acyclic ✓ — ${comps.length} strongly-connected components, each a single node.`));
    return;
  }
  box.appendChild(mk('div', 'scc-banner bad',
    `${cycles.length} dependency cycle${cycles.length === 1 ? '' : 's'} found.`));
  for (const c of cycles) {
    const comp = mk('div', 'scc-comp');
    comp.appendChild(mk('div', 'muted', c.members.length + ' members:'));
    for (const m of c.members) {
      const row = mk('div', 'm', shortLabel(m.purl));
      row.title = m.purl;
      row.addEventListener('click', () => focusNode(m.id));
      comp.appendChild(row);
    }
    box.appendChild(comp);
  }
}

// --- TDA barcode ------------------------------------------------------------

async function loadBarcode(run) {
  const box = el('barcode');
  clear(box);
  let data;
  try { data = await getJson('/api/tda?run=' + run); }
  catch (_) {
    setBadge('tda', null);
    box.appendChild(mk('p', 'muted', 'No persistent homology for this run yet.'));
    const hint = mk('p', 'muted');
    hint.appendChild(document.createTextNode('Compute it with '));
    hint.appendChild(mk('code', '', `prism tda run --run ${run}`));
    hint.appendChild(document.createTextNode('.'));
    box.appendChild(hint);
    return;
  }
  const summary = mk('dl');
  summary.appendChild(mk('dt', '', 'H₀ components'));
  summary.appendChild(mk('dd', '', String(data.component_count)));
  summary.appendChild(mk('dt', '', 'H₁ cycles'));
  summary.appendChild(mk('dd', '', String(data.cycle_count)));
  box.appendChild(summary);

  const dims = data.dims || [];
  setBadge('tda', dims.length || null);
  let max = 1;
  for (const d of dims) max = Math.max(max, d.total);
  for (const d of dims) {
    const grp = mk('div', 'bar-group');
    grp.appendChild(mk('div', 'gh', 'Dimension H' + d.dim));
    grp.appendChild(barRow('finite', d.finite, max, false));
    grp.appendChild(barRow('essential', d.essential, max, true));
    box.appendChild(grp);
  }
  if (!dims.length) box.appendChild(mk('p', 'muted', 'Empty barcode.'));
}

function barRow(label, count, max, essential) {
  const row = mk('div', 'bar-row');
  row.appendChild(mk('div', 'bar-label', label));
  const track = mk('div', 'bar-track');
  const fill = mk('div', 'bar-fill' + (essential ? ' essential' : ''));
  fill.style.width = Math.round((count / max) * 100) + '%';
  track.appendChild(fill);
  row.appendChild(track);
  row.appendChild(mk('div', 'bar-count', String(count)));
  return row;
}

// --- TDA features (persistence pairs -> 3D generator highlight) -------------

/* Load the run's persistence pairs and render a selectable feature list. The
 * server resolves a chosen pair's generators back to concrete graph objects via
 * /api/tda/support, which the 3D TDA mode lights up. Best-effort: a run with no
 * persisted .ptda just leaves the list empty (the barcode panel explains how to
 * compute one). */
async function loadFeatures(run) {
  const box = el('tda-features');
  clear(box);
  state.features = [];
  let data;
  try { data = await getJson('/api/tda/pairs?run=' + run); }
  catch (_) { return; }
  const pairs = data.pairs || [];
  state.features = pairs;
  if (!pairs.length) return;
  box.appendChild(mk('p', 'feat-note',
    'Click a feature to light up the packages that carry it (opens 3D · TDA).'));
  renderFeatureGroup(box, 'H₀ — connected components', pairs.filter((p) => p.dim === 0));
  renderFeatureGroup(box, 'H₁ — loops', pairs.filter((p) => p.dim === 1));
}

function renderFeatureGroup(box, title, list) {
  if (!list.length) return;
  box.appendChild(mk('div', 'feat-group-h', title + ' (' + list.length + ')'));
  // Essential (never-dying) first, then longest-lived — the most significant
  // features at the top. Cap the displayed rows; note any remainder.
  const sorted = list.slice().sort((a, b) =>
    (Number(b.essential) - Number(a.essential)) ||
    ((b.persistence || 0) - (a.persistence || 0)) || (a.pair - b.pair));
  const CAP = 40;
  for (const p of sorted.slice(0, CAP)) box.appendChild(featRow(p));
  if (sorted.length > CAP) {
    box.appendChild(mk('p', 'feat-note',
      `… and ${sorted.length - CAP} more (showing the ${CAP} longest-lived).`));
  }
}

function fmtNum(x) { return (x == null) ? '?' : String(Math.round(x * 10) / 10); }

function featRow(p) {
  const row = mk('div', 'feat');
  row.dataset.pair = String(p.pair);
  row.appendChild(mk('span', 'feat-dim', 'H' + p.dim));
  const life = p.essential
    ? 'born d' + fmtNum(p.birth) + ' · never dies'
    : 'd' + fmtNum(p.birth) + ' → d' + fmtNum(p.death) + ' · life ' + fmtNum(p.persistence);
  row.appendChild(mk('span', 'feat-life', life));
  row.appendChild(mk('span', 'feat-tag' + (p.essential ? ' essential' : ''),
    p.essential ? 'essential' : 'finite'));
  row.addEventListener('click', () => selectFeature(p, row));
  return row;
}

async function selectFeature(p, row) {
  for (const r of document.querySelectorAll('.feat')) r.classList.toggle('active', r === row);
  setStatus('Resolving feature generators…');
  try {
    const sup = await getJson(`/api/tda/support?run=${state.run}&pair=${p.pair}`);
    const objs = sup.objects || [];
    if (state.view !== '3d') setView('3d');
    setMode3d('tda');
    Scene3D.setFeatureSupport(p.dim, objs);
    const nN = objs.filter((o) => o.type === 'node').length;
    const nE = objs.filter((o) => o.type === 'edge').length;
    setStatus(`H${p.dim} generator lit: ${nN} node(s), ${nE} edge(s).`);
  } catch (e) { setStatus(e.message, true); }
}

// --- 3D view control --------------------------------------------------------

/* Switch between the 2D SVG view and the 3D canvas. Both render the same data
 * and selection; only the active surface and its overlay controls change. */
function setView(v) {
  state.view = v;
  const is3d = v === '3d';
  el('graph-svg').classList.toggle('hidden', is3d);
  el('graph-3d').classList.toggle('hidden', !is3d);
  el('view3d-controls').classList.toggle('hidden', !is3d);
  el('zoom-controls').classList.toggle('hidden', is3d);
  el('view-2d').classList.toggle('active', !is3d);
  el('view-3d').classList.toggle('active', is3d);
  if (is3d) {
    Scene3D.show();
  } else {
    Scene3D.hide();
    el('v3d-spin').classList.remove('active');
    render(); // refresh the SVG with any selection changed while in 3D
  }
}

function setMode3d(m) {
  for (const b of document.querySelectorAll('.v3d-mode')) {
    b.classList.toggle('active', b.dataset.mode === m);
  }
  el('v3d-signal-row').classList.toggle('hidden', m !== 'terrain');
  el('v3d-hint').classList.toggle('hidden', false);
  Scene3D.setMode(m);
  if (m === 'tda') showTab('tda'); // surface the feature list to pick from
}

// --- search -----------------------------------------------------------------

function runSearch(term) {
  term = (term || '').trim().toLowerCase();
  state.matches = new Set();
  const countEl = el('search-count');
  if (!term) { countEl.textContent = ''; refresh(); return; }
  let first = null;
  for (const n of state.nodes) {
    if (n.purl.toLowerCase().includes(term)) {
      state.matches.add(n.id);
      if (first == null) first = n.id;
    }
  }
  countEl.textContent = state.matches.size ? (state.matches.size + ' hit' + (state.matches.size === 1 ? '' : 's'))
                                           : 'no hits';
  refresh();
  return first;
}

// --- tabs -------------------------------------------------------------------

function showTab(name) {
  for (const t of document.querySelectorAll('.tab')) {
    t.classList.toggle('active', t.dataset.tab === name);
  }
  for (const b of document.querySelectorAll('.tab-body')) {
    b.classList.toggle('active', b.dataset.panel === name);
  }
}

function setBadge(tab, count) {
  const t = document.querySelector('.tab[data-tab="' + tab + '"]');
  if (!t) return;
  let b = t.querySelector('.badge');
  if (count == null) { if (b) b.remove(); return; }
  if (!b) { b = mk('span', 'badge'); t.appendChild(b); }
  b.textContent = String(count);
}

// --- wiring -----------------------------------------------------------------

function init() {
  installPanZoom();

  el('run-select').addEventListener('change', (ev) => {
    if (ev.target.value !== '') selectRun(Number(ev.target.value));
  });
  el('reload-btn').addEventListener('click', loadRuns);

  for (const t of document.querySelectorAll('.tab')) {
    t.addEventListener('click', () => showTab(t.dataset.tab));
  }

  const search = el('search');
  search.addEventListener('input', () => runSearch(search.value));
  search.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter') {
      const first = runSearch(search.value);
      if (first != null) focusNode(first); // focusNode centers per active view
    } else if (ev.key === 'Escape') {
      search.value = ''; runSearch('');
    }
  });

  install3dControls();
  loadRuns().catch((e) => setStatus(e.message, true));
}

/* Wire the 3D canvas, the 2D/3D toggle, and the 3D overlay controls. */
function install3dControls() {
  Scene3D.init(el('graph-3d'));
  Scene3D.onPick((node) => onNodeClick(node));

  el('view-2d').addEventListener('click', () => setView('2d'));
  el('view-3d').addEventListener('click', () => setView('3d'));

  for (const b of document.querySelectorAll('.v3d-mode')) {
    b.addEventListener('click', () => setMode3d(b.dataset.mode));
  }
  el('v3d-signal').addEventListener('change', (ev) => Scene3D.setSignal(ev.target.value));
  el('v3d-spin').addEventListener('click', () => {
    const on = Scene3D.toggleSpin();
    el('v3d-spin').classList.toggle('active', on);
  });
  el('v3d-reset').addEventListener('click', () => Scene3D.resetView());
}

document.addEventListener('DOMContentLoaded', init);

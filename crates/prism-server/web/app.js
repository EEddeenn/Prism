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
    // Side panels (independent; a failure in one must not blank the others).
    await Promise.all([loadOverview(run), loadDuplicates(run), loadScc(run), loadBarcode(run)]);
    showTab('overview');
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

/* Deterministic layered layout. Column = BFS depth. Within a column, order by
 * the mean row of already-placed parents (barycenter heuristic) to cut edge
 * crossings; ties and parentless nodes fall back to stable id order. Pure
 * function of the export, so reloads are pixel-stable. */
function layout() {
  const colGap = 230, rowGap = 60, padX = 80, padY = 60;
  const byDepth = new Map();
  for (const n of state.nodes) {
    if (!byDepth.has(n.depth)) byDepth.set(n.depth, []);
    byDepth.get(n.depth).push(n);
  }
  const depths = Array.from(byDepth.keys()).sort((a, b) => a - b);
  const rowOf = new Map(); // id -> row index in its column
  state.pos = new Map();
  let maxRow = 0;

  for (const d of depths) {
    const col = byDepth.get(d);
    // Barycenter key: average parent row (parents live in shallower columns and
    // are already placed). Parentless nodes keep their export order via index.
    col.forEach((n, idx) => { n._idx = idx; });
    const bary = (n) => {
      const parents = state.in.get(n.id) || [];
      let sum = 0, cnt = 0;
      for (const p of parents) {
        if (rowOf.has(p)) { sum += rowOf.get(p); cnt++; }
      }
      return cnt ? sum / cnt : n._idx;
    };
    col.sort((a, b) => (bary(a) - bary(b)) || (a._idx - b._idx));
    col.forEach((n, row) => {
      rowOf.set(n.id, row);
      state.pos.set(n.id, { x: padX + d * colGap, y: padY + row * rowGap });
      if (row > maxRow) maxRow = row;
    });
  }
  state._world = {
    w: padX * 2 + Math.max(0, depths.length - 1) * colGap,
    h: padY * 2 + maxRow * rowGap,
  };
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
    g.addEventListener('click', (ev) => { ev.stopPropagation(); onNodeClick(n); });
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
  let dragging = false, lastX = 0, lastY = 0, moved = 0;

  svg.addEventListener('pointerdown', (ev) => {
    // Only pan from the background (node clicks stopPropagation in render()).
    dragging = true; moved = 0;
    lastX = ev.clientX; lastY = ev.clientY;
    svg.classList.add('panning');
    svg.setPointerCapture(ev.pointerId);
  });
  svg.addEventListener('pointermove', (ev) => {
    if (!dragging) return;
    const dx = ev.clientX - lastX, dy = ev.clientY - lastY;
    lastX = ev.clientX; lastY = ev.clientY;
    moved += Math.abs(dx) + Math.abs(dy);
    state.T.x += dx; state.T.y += dy;
    applyTransform();
  });
  const endPan = (ev) => {
    if (!dragging) return;
    dragging = false;
    svg.classList.remove('panning');
    try { svg.releasePointerCapture(ev.pointerId); } catch (_) {}
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
    render();
    setStatus('');
  } catch (e) {
    setStatus(e.message, true);
  }
}

function focusNode(id) {
  const n = state.byId.get(id);
  if (n) onNodeClick(n);
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
  render();
  // Center the view loosely on the first occurrence.
  const p = state.pos.get(ids[0]);
  if (p) centerOn(p);
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

// --- search -----------------------------------------------------------------

function runSearch(term) {
  term = (term || '').trim().toLowerCase();
  state.matches = new Set();
  const countEl = el('search-count');
  if (!term) { countEl.textContent = ''; render(); return; }
  let first = null;
  for (const n of state.nodes) {
    if (n.purl.toLowerCase().includes(term)) {
      state.matches.add(n.id);
      if (first == null) first = n.id;
    }
  }
  countEl.textContent = state.matches.size ? (state.matches.size + ' hit' + (state.matches.size === 1 ? '' : 's'))
                                           : 'no hits';
  render();
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
      if (first != null) { const p = state.pos.get(first); if (p) centerOn(p); focusNode(first); }
    } else if (ev.key === 'Escape') {
      search.value = ''; runSearch('');
    }
  });

  loadRuns().catch((e) => setStatus(e.message, true));
}

document.addEventListener('DOMContentLoaded', init);

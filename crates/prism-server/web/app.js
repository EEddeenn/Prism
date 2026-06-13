/* Prism graph explorer — vanilla JS, no framework, no external script.
 *
 * Flow: load /api/runs to populate the picker; on pick, fetch /api/export and
 * draw nodes+edges as inline SVG with a deterministic layered layout (BFS depth
 * -> column, stable index -> row). Clicking a node fetches its forward
 * neighborhood and reverse impact and highlights both. The side panel shows
 * graph stats and the TDA barcode (if one is persisted for the run).
 */
'use strict';

const SVG_NS = 'http://www.w3.org/2000/svg';

const state = {
  run: null,
  nodes: [],          // [{id, purl, depth, ...}]
  edges: [],          // [{source, target, kind, flags}]
  pos: new Map(),     // id -> {x, y}
};

// --- tiny DOM/SVG helpers ---------------------------------------------------

function el(id) { return document.getElementById(id); }

function svgEl(name, attrs) {
  const node = document.createElementNS(SVG_NS, name);
  if (attrs) {
    for (const k of Object.keys(attrs)) node.setAttribute(k, attrs[k]);
  }
  return node;
}

function clear(node) { while (node.firstChild) node.removeChild(node.firstChild); }

function setStatus(text, isError) {
  const s = el('status-text');
  s.textContent = text || '';
  s.classList.toggle('error', !!isError);
}

async function getJson(url) {
  const resp = await fetch(url);
  let body;
  try { body = await resp.json(); } catch (_) { body = null; }
  if (!resp.ok) {
    const msg = (body && body.error) ? body.error : ('HTTP ' + resp.status);
    throw new Error(msg);
  }
  return body;
}

// --- run picker -------------------------------------------------------------

async function loadRuns() {
  setStatus('Loading runs…');
  const runs = await getJson('/api/runs');
  const sel = el('run-select');
  clear(sel);
  if (!runs.length) {
    const opt = document.createElement('option');
    opt.textContent = '(no ingested runs)';
    opt.value = '';
    sel.appendChild(opt);
    setStatus('No runs. Ingest a lockfile with the CLI first.');
    return;
  }
  for (const r of runs) {
    const opt = document.createElement('option');
    const counts = (r.nodes != null) ? (' (' + r.nodes + 'n/' + r.edges + 'e)') : '';
    opt.value = String(r.run);
    opt.textContent = 'run ' + r.run + counts;
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
    layout();
    render(null, new Set(), new Set());
    await loadStats(run);
    await loadBarcode(run);
    setStatus('');
  } catch (e) {
    setStatus(e.message, true);
  }
}

/* Deterministic layered layout: column = BFS depth, row = stable order index
 * among nodes sharing that depth. Pure function of the export, so reloads are
 * pixel-stable. */
function layout() {
  const colGap = 220, rowGap = 64, padX = 70, padY = 50;
  const byDepth = new Map();
  // Nodes arrive in id order; keep that as the within-column row order.
  for (const n of state.nodes) {
    if (!byDepth.has(n.depth)) byDepth.set(n.depth, []);
    byDepth.get(n.depth).push(n);
  }
  state.pos = new Map();
  let maxRow = 0;
  const depths = Array.from(byDepth.keys()).sort((a, b) => a - b);
  for (const d of depths) {
    const col = byDepth.get(d);
    col.forEach((n, row) => {
      state.pos.set(n.id, { x: padX + d * colGap, y: padY + row * rowGap });
      if (row > maxRow) maxRow = row;
    });
  }
  const width = padX * 2 + Math.max(0, depths.length - 1) * colGap + 40;
  const height = padY * 2 + maxRow * rowGap + 40;
  const svg = el('graph-svg');
  svg.setAttribute('viewBox', '0 0 ' + width + ' ' + height);
}

/* Draw the graph. `center` is the selected node id (or null); `fwd`/`impact`
 * are Sets of node ids to highlight as forward-reachable / reverse-impact. */
function render(center, fwd, impact) {
  const svg = el('graph-svg');
  clear(svg);

  // Arrowhead marker (defined once).
  const defs = svgEl('defs');
  const marker = svgEl('marker', {
    id: 'arrow', viewBox: '0 0 10 10', refX: '9', refY: '5',
    markerWidth: '6', markerHeight: '6', orient: 'auto-start-reverse',
  });
  marker.appendChild(svgEl('path', { d: 'M 0 0 L 10 5 L 0 10 z', fill: '#4b5366' }));
  defs.appendChild(marker);
  svg.appendChild(defs);

  const hasSelection = center != null;
  const edgeKey = (s, t) => s + '>' + t;
  const fwdEdges = new Set();
  const impactEdges = new Set();
  if (hasSelection) {
    for (const e of state.edges) {
      if (fwd.has(e.source) && fwd.has(e.target)) fwdEdges.add(edgeKey(e.source, e.target));
      if (impact.has(e.source) && impact.has(e.target)) impactEdges.add(edgeKey(e.source, e.target));
    }
  }

  // Edges first (under nodes).
  for (const e of state.edges) {
    const a = state.pos.get(e.source), b = state.pos.get(e.target);
    if (!a || !b) continue;
    let cls = 'edge';
    if (fwdEdges.has(edgeKey(e.source, e.target))) cls += ' fwd';
    else if (impactEdges.has(edgeKey(e.source, e.target))) cls += ' impact';
    const line = svgEl('line', {
      x1: a.x, y1: a.y, x2: b.x, y2: b.y, class: cls,
      'marker-end': 'url(#arrow)',
    });
    svg.appendChild(line);
  }

  // Nodes.
  for (const n of state.nodes) {
    const p = state.pos.get(n.id);
    if (!p) continue;
    let cls = 'node';
    if (hasSelection) {
      if (n.id === center) cls += ' center';
      else if (fwd.has(n.id)) cls += ' fwd';
      else if (impact.has(n.id)) cls += ' impact';
      else cls += ' dim';
    }
    const g = svgEl('g', { class: cls, 'data-id': n.id });
    g.appendChild(svgEl('circle', { cx: p.x, cy: p.y, r: 10 }));
    const label = svgEl('text', { x: p.x + 14, y: p.y + 4 });
    label.textContent = shortLabel(n.purl);
    g.appendChild(label);
    const title = svgEl('title');
    title.textContent = n.purl + '  (node ' + n.id + ', depth ' + n.depth + ')';
    g.appendChild(title);
    g.addEventListener('click', () => onNodeClick(n));
    svg.appendChild(g);
  }
}

/* Trim a purl to a compact display label. */
function shortLabel(purl) {
  if (!purl) return '?';
  let s = purl.replace(/^pkg:[^/]+\//, '');
  if (s.length > 28) s = s.slice(0, 27) + '…';
  return s;
}

// --- node selection: neighborhood + impact ----------------------------------

async function onNodeClick(node) {
  setStatus('Loading neighborhood…');
  showNodeInfo(node);
  try {
    const run = state.run;
    const [nb, imp] = await Promise.all([
      getJson('/api/graph/neighborhood?run=' + run + '&node=' + node.id + '&depth=64'),
      getJson('/api/graph/impact?run=' + run + '&node=' + node.id),
    ]);
    const fwd = new Set([node.id]);
    for (const n of (nb.nodes || [])) fwd.add(n.id);
    const impact = new Set([node.id]);
    for (const n of (imp.nodes || [])) impact.add(n.id);
    // The clicked node belongs to both; render() colors it as the center.
    render(node.id, fwd, impact);
    setStatus('');
  } catch (e) {
    setStatus(e.message, true);
  }
}

function showNodeInfo(node) {
  const box = el('node-info');
  clear(box);
  const purl = document.createElement('div');
  purl.className = 'purl';
  purl.textContent = node.purl;
  box.appendChild(purl);
  const rows = [
    ['node id', node.id],
    ['version id', node.version_id],
    ['depth', node.depth],
  ];
  for (const [k, v] of rows) {
    const row = document.createElement('div');
    row.className = 'row';
    const a = document.createElement('span'); a.textContent = k;
    const b = document.createElement('span'); b.textContent = v;
    row.appendChild(a); row.appendChild(b);
    box.appendChild(row);
  }
}

// --- side panel: stats + barcode --------------------------------------------

async function loadStats(run) {
  const list = el('stats-list');
  clear(list);
  try {
    const s = await getJson('/api/graph/stats?run=' + run);
    const rows = [
      ['nodes', s.nodes],
      ['edges', s.edges],
      ['SCCs', s.scc_count],
      ['has cycle', s.has_cycle ? 'yes' : 'no'],
      ['max depth', s.max_depth],
      ['duplicates', s.duplicate_version_count],
    ];
    for (const [k, v] of rows) {
      const dt = document.createElement('dt'); dt.textContent = k;
      const dd = document.createElement('dd'); dd.textContent = v;
      list.appendChild(dt); list.appendChild(dd);
    }
  } catch (e) {
    const dd = document.createElement('dd');
    dd.className = 'muted';
    dd.textContent = e.message;
    list.appendChild(dd);
  }
}

async function loadBarcode(run) {
  const box = el('barcode');
  clear(box);
  let data;
  try {
    data = await getJson('/api/tda?run=' + run);
  } catch (e) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.textContent = 'No TDA for this run (compute one with `prism tda run`).';
    box.appendChild(p);
    return;
  }
  const dims = data.dims || [];
  let max = 1;
  for (const d of dims) max = Math.max(max, d.total);
  for (const d of dims) {
    box.appendChild(barRow('H' + d.dim + ' finite', d.finite, max, false));
    box.appendChild(barRow('H' + d.dim + ' essential', d.essential, max, true));
  }
  if (!dims.length) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.textContent = 'Empty barcode.';
    box.appendChild(p);
  }
}

function barRow(label, count, max, essential) {
  const row = document.createElement('div');
  row.className = 'bar-row';
  const lbl = document.createElement('div');
  lbl.className = 'bar-label';
  lbl.textContent = label;
  const track = document.createElement('div');
  track.className = 'bar-track';
  const fill = document.createElement('div');
  fill.className = 'bar-fill' + (essential ? ' essential' : '');
  fill.style.width = Math.round((count / max) * 100) + '%';
  track.appendChild(fill);
  const num = document.createElement('div');
  num.className = 'bar-count';
  num.textContent = count;
  row.appendChild(lbl); row.appendChild(track); row.appendChild(num);
  return row;
}

// --- wiring -----------------------------------------------------------------

function init() {
  el('run-select').addEventListener('change', (ev) => {
    const v = ev.target.value;
    if (v !== '') selectRun(Number(v));
  });
  el('reload-btn').addEventListener('click', () => loadRuns());
  loadRuns().catch((e) => setStatus(e.message, true));
}

document.addEventListener('DOMContentLoaded', init);

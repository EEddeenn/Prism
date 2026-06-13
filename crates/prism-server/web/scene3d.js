/* Prism 3D Topology View — a self-contained Canvas renderer (no WebGL library,
 * no external script). It complements the 2D SVG graph with a spatial,
 * *semantic* topology landscape rather than a force-directed hairball.
 *
 * Four view modes, each giving the third dimension a concrete meaning:
 *   layered — dependency DEPTH becomes stacked horizontal layers (roots on top).
 *   fiber   — each package FAMILY is a vertical version fiber; duplicates stack.
 *   terrain — the graph becomes a heightfield whose elevation is an analytical
 *             signal (centrality / duplicate pressure / fan-in / depth …).
 *   tda     — a selected persistence feature's generator support is lit up on
 *             the depth substrate, tying H0 components / H1 loops back to packages.
 *
 * It reads the shared `state` object and helpers defined by app.js (same global
 * lexical scope across classic scripts); it owns only its own 3D positions,
 * camera, and per-node metrics. Rendering is painter's-algorithm over a manual
 * perspective projection with depth fog — fast enough for ~1000 nodes / ~2000
 * edges on a 2D canvas, and picking is exact (project centers, hit-test screen).
 */
'use strict';

const Scene3D = (function () {
  // ---- palette (mirrors style.css; kept in sync by eye) --------------------
  const PAL = {
    bg: [14, 16, 22],
    node: [59, 130, 246],
    root: [167, 139, 250],
    center: [255, 180, 84],
    fwd: [52, 211, 153],
    impact: [248, 113, 113],
    dup: [244, 114, 182],
    edge: [58, 65, 80],
    accent: [90, 169, 255],
    ink: [231, 233, 240],
    muted: [139, 147, 167],
    gen: [255, 209, 102], // TDA generator highlight
  };
  // Terrain elevation ramp: low → high (cool deep → teal → warm → hot).
  const RAMP = [
    [0.0, [30, 48, 80]],
    [0.35, [37, 110, 120]],
    [0.7, [202, 166, 74]],
    [1.0, [224, 88, 79]],
  ];

  const FOV = 52 * Math.PI / 180;
  const NEAR = 0.05;
  const LAYER_GAP = 78, NODE_SPACE = 46;
  const FAM_GAP = 76, VER_GAP = 50;
  const TERRAIN_S = 320, TERRAIN_H = 150, GRID_N = 46;
  const MIN_DIST = 60, MAX_DIST = 16000;

  // ---- module state --------------------------------------------------------
  let canvas = null, ctx = null, dpr = 1;
  let active = false;
  let mode = 'layered';
  let signal = 'centrality';
  let pickCb = null;

  const cam = { yaw: 0.7, pitch: 0.55, dist: 1400, tx: 0, ty: 0, tz: 0 };
  let spin = false, rafPending = false, spinRaf = 0;

  const pos3 = new Map();      // id -> {x,y,z}
  const metrics = new Map();   // id -> {inDeg,outDeg,revReach,fwdReach,famSize,depth}
  let decor = null;            // mode-specific decorations (planes/fibers/grid)
  let feature = null;          // {dim, nodeIds:Set, edgeKeys:Set} for TDA mode
  const screen = new Map();    // id -> {sx,sy,sz,scale,vis}
  let hover = null;            // hovered node id

  // ---- small vector helpers ------------------------------------------------
  function sub(a, b) { return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]; }
  function cross(a, b) {
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  }
  function dot(a, b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
  function norm(a) {
    const l = Math.hypot(a[0], a[1], a[2]) || 1;
    return [a[0] / l, a[1] / l, a[2] / l];
  }
  function lerp(a, b, t) { return a + (b - a) * t; }
  function rgb(c, a) { return `rgba(${c[0]|0},${c[1]|0},${c[2]|0},${a == null ? 1 : a})`; }
  function mix(a, b, t) { return [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]; }
  function rampColor(t) {
    t = Math.max(0, Math.min(1, t));
    for (let i = 1; i < RAMP.length; i++) {
      if (t <= RAMP[i][0]) {
        const u = (t - RAMP[i - 1][0]) / (RAMP[i][0] - RAMP[i - 1][0] || 1);
        return mix(RAMP[i - 1][1], RAMP[i][1], u);
      }
    }
    return RAMP[RAMP.length - 1][1];
  }

  /* Compact display label for a purl (mirrors app.js shortLabel, kept local so
   * scene3d has no load-order dependency on it). */
  function lbl(purl) {
    if (!purl) return '?';
    let s = purl.replace(/^pkg:[^/]+\//, '');
    if (s.length > 28) s = s.slice(0, 27) + '…';
    return s;
  }

  /* Sort key for a package version parsed out of a purl ("name@1.2.3-rc.1"). */
  function verKey(purl) {
    const at = purl.lastIndexOf('@');
    const v = at >= 0 ? purl.slice(at + 1) : purl;
    const dash = v.indexOf('-');
    const core = (dash >= 0 ? v.slice(0, dash) : v).split('.').map((x) => parseInt(x, 10) || 0);
    const pre = dash >= 0 ? v.slice(dash + 1) : '';
    return { core, pre, raw: v };
  }
  function verCmp(a, b) {
    const ka = verKey(a), kb = verKey(b);
    for (let i = 0; i < Math.max(ka.core.length, kb.core.length); i++) {
      const d = (ka.core[i] || 0) - (kb.core[i] || 0);
      if (d) return d;
    }
    // No prerelease sorts AFTER a prerelease of the same core (semver rule).
    if (!ka.pre && kb.pre) return 1;
    if (ka.pre && !kb.pre) return -1;
    return ka.pre < kb.pre ? -1 : ka.pre > kb.pre ? 1 : 0;
  }

  // ---- per-node metrics (computed once on data load) -----------------------
  function familyKey(n) {
    if (n.package_id != null) return 'p' + n.package_id;
    const s = (n.purl || '').replace(/^pkg:[^/]+\//, '');
    const at = s.lastIndexOf('@');
    return 'n' + (at >= 0 ? s.slice(0, at) : s);
  }

  function computeMetrics() {
    metrics.clear();
    const nodes = state.nodes;
    // Family membership (for duplicate pressure + the fiber view).
    const famVersions = new Map(); // key -> Set(version_id)
    for (const n of nodes) {
      const k = familyKey(n);
      if (!famVersions.has(k)) famVersions.set(k, new Set());
      famVersions.get(k).add(n.version_id);
    }
    for (const n of nodes) {
      metrics.set(n.id, {
        inDeg: (state.in.get(n.id) || []).length,
        outDeg: (state.out.get(n.id) || []).length,
        revReach: (state.in.get(n.id) || []).length,
        fwdReach: (state.out.get(n.id) || []).length,
        famSize: famVersions.get(familyKey(n)).size,
        depth: n.depth || 0,
      });
    }
    // Transitive reach (true centrality) — bounded so huge graphs stay snappy;
    // beyond the cap we keep the in/out-degree proxies seeded above. One pass:
    // a forward DFS from every source gives fwdReach[src] = #transitive deps and,
    // by tallying each node it lands on, revReach[t] = #transitive dependents.
    if (nodes.length <= 1500) {
      const idx = new Map();
      nodes.forEach((n, i) => idx.set(n.id, i));
      for (const n of nodes) metrics.get(n.id).revReach = 0; // drop the inDeg seed
      const seen = new Int32Array(nodes.length).fill(-1);
      for (let s = 0; s < nodes.length; s++) {
        const src = nodes[s].id;
        let reached = 0;
        const stack = [src];
        while (stack.length) {
          const cur = stack.pop();
          for (const t of (state.out.get(cur) || [])) {
            const ti = idx.get(t);
            if (ti == null || seen[ti] === s) continue;
            seen[ti] = s;
            reached++;
            metrics.get(t).revReach++; // t gains one transitive dependent (src)
            stack.push(t);
          }
        }
        metrics.get(src).fwdReach = reached;
      }
    }
  }

  function signalValue(n, sig) {
    const m = metrics.get(n.id);
    if (!m) return 0;
    switch (sig) {
      case 'centrality': return m.revReach;            // transitive dependents
      case 'reach': return m.fwdReach;                 // transitive dependencies
      case 'fanin': return m.inDeg;
      case 'fanout': return m.outDeg;
      case 'duplicates': return m.famSize - 1;         // 0 unless duplicated
      case 'depth': return m.depth;                    // = the TDA filtration value
      case 'risk':                                     // composite, documented
        return m.revReach * (1 + (m.famSize - 1) * 0.6) * (1 + m.depth * 0.05);
      default: return m.revReach;
    }
  }

  // ---- layout builders -----------------------------------------------------
  function buildLayout() {
    pos3.clear();
    decor = null;
    if (!state.nodes.length) return;
    if (mode === 'fiber') return layoutFiber();
    if (mode === 'terrain') return layoutTerrain();
    return layoutLayered(); // 'layered' and 'tda' share the depth substrate
  }

  /* Layered: depth → stacked horizontal planes, roots on top. Within a plane,
   * nodes sit on a centred grid ordered by parent barycentre to limit crossings. */
  function layoutLayered() {
    const byDepth = new Map();
    let maxD = 0;
    for (const n of state.nodes) {
      const d = n.depth || 0;
      maxD = Math.max(maxD, d);
      if (!byDepth.has(d)) byDepth.set(d, []);
      byDepth.get(d).push(n);
    }
    const depths = Array.from(byDepth.keys()).sort((a, b) => a - b);
    const planes = [];
    for (const d of depths) {
      const layerNodes = byDepth.get(d);
      const y = (maxD / 2 - d) * LAYER_GAP;
      // Order by mean parent X (already placed at shallower depth).
      const keyed = layerNodes.map((n) => {
        const parents = (state.in.get(n.id) || []).map((p) => pos3.get(p)).filter(Boolean);
        const bx = parents.length ? parents.reduce((s, p) => s + p.x, 0) / parents.length : 0;
        const bz = parents.length ? parents.reduce((s, p) => s + p.z, 0) / parents.length : 0;
        return { n, k: bx * 1000 + bz };
      });
      keyed.sort((a, b) => (a.k - b.k) || (a.n.id - b.n.id));
      const count = keyed.length;
      const cols = Math.max(1, Math.ceil(Math.sqrt(count * 1.3)));
      const rows = Math.ceil(count / cols);
      const w = (cols - 1) * NODE_SPACE, h = (rows - 1) * NODE_SPACE;
      keyed.forEach((e, i) => {
        const c = i % cols, r = Math.floor(i / cols);
        pos3.set(e.n.id, { x: c * NODE_SPACE - w / 2, y, z: r * NODE_SPACE - h / 2 });
      });
      planes.push({ y, depth: d, count, radius: Math.max(w, h) / 2 + NODE_SPACE });
    }
    decor = { kind: 'layers', planes };
  }

  /* Fiber: each family is a vertical stack of its versions; tall = duplicated. */
  function layoutFiber() {
    const fams = new Map(); // key -> {nodes:[], label}
    for (const n of state.nodes) {
      const k = familyKey(n);
      if (!fams.has(k)) fams.set(k, { nodes: [], label: lbl(n.purl).split('@')[0] });
      fams.get(k).nodes.push(n);
    }
    const list = Array.from(fams.values());
    // Bigger / duplicated families toward the centre; deterministic order.
    list.sort((a, b) => (b.nodes.length - a.nodes.length) || a.label.localeCompare(b.label));
    const cols = Math.max(1, Math.ceil(Math.sqrt(list.length * 1.2)));
    const fibers = [];
    list.forEach((fam, i) => {
      const c = i % cols, r = Math.floor(i / cols);
      const gx = (c - (cols - 1) / 2) * FAM_GAP;
      const gz = (r - Math.floor(list.length / cols) / 2) * FAM_GAP;
      // Distinct versions, ascending; rank gives the height.
      const versions = Array.from(new Set(fam.nodes.map((n) => n.purl)))
        .sort(verCmp);
      const rankOf = new Map();
      versions.forEach((v, k) => rankOf.set(v, k));
      const seenRank = new Map();
      for (const n of fam.nodes) {
        const rank = rankOf.get(n.purl) || 0;
        // Spread same-version occurrences in a tiny ring around the fiber.
        const dupAt = seenRank.get(rank) || 0;
        seenRank.set(rank, dupAt + 1);
        const a = dupAt * 1.7, rr = dupAt ? 12 : 0;
        pos3.set(n.id, { x: gx + Math.cos(a) * rr, y: rank * VER_GAP, z: gz + Math.sin(a) * rr });
      }
      fibers.push({
        x: gx, z: gz, top: (versions.length - 1) * VER_GAP,
        dup: versions.length > 1, count: versions.length, label: fam.label,
      });
    });
    decor = { kind: 'fibers', fibers };
  }

  /* Terrain: ground (x,z) from the 2D layout; height = the chosen signal. A
   * smoothed heightfield grid renders as the landscape; nodes sit on it. */
  function layoutTerrain() {
    // Ground positions from the shared 2D layout, normalised to a centred square.
    let minx = Infinity, maxx = -Infinity, minz = Infinity, maxz = -Infinity;
    for (const n of state.nodes) {
      const p = state.pos.get(n.id);
      if (!p) continue;
      minx = Math.min(minx, p.x); maxx = Math.max(maxx, p.x);
      minz = Math.min(minz, p.y); maxz = Math.max(maxz, p.y);
    }
    const sx = TERRAIN_S * 2 / Math.max(1, maxx - minx);
    const sz = TERRAIN_S * 2 / Math.max(1, maxz - minz);
    const s = Math.min(sx, sz);
    let maxSig = 0;
    for (const n of state.nodes) maxSig = Math.max(maxSig, signalValue(n, signal));
    const hScale = TERRAIN_H / (maxSig || 1);
    const ground = new Map();
    for (const n of state.nodes) {
      const p = state.pos.get(n.id) || { x: 0, y: 0 };
      const gx = (p.x - (minx + maxx) / 2) * s;
      const gz = (p.y - (minz + maxz) / 2) * s;
      const y = signalValue(n, signal) * hScale;
      ground.set(n.id, { gx, gz });
      pos3.set(n.id, { x: gx, y, z: gz });
    }
    // Heightfield: bin nodes (max), then fill empties and blur for a surface.
    const G = GRID_N, H = new Float32Array(G * G).fill(NaN);
    const cell = TERRAIN_S * 2 / (G - 1);
    for (const n of state.nodes) {
      const g = ground.get(n.id);
      const i = Math.round((g.gx + TERRAIN_S) / cell);
      const j = Math.round((g.gz + TERRAIN_S) / cell);
      if (i < 0 || j < 0 || i >= G || j >= G) continue;
      const y = pos3.get(n.id).y, k = j * G + i;
      if (isNaN(H[k]) || y > H[k]) H[k] = y;
    }
    fillAndBlur(H, G);
    decor = { kind: 'terrain', H, G, cell, signal, maxSig };
  }

  function fillAndBlur(H, G) {
    // Iteratively fill NaN cells from defined neighbours.
    for (let pass = 0; pass < 6; pass++) {
      let changed = false;
      const next = H.slice();
      for (let j = 0; j < G; j++) for (let i = 0; i < G; i++) {
        const k = j * G + i;
        if (!isNaN(H[k])) continue;
        let sum = 0, cnt = 0;
        for (const [di, dj] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
          const ni = i + di, nj = j + dj;
          if (ni < 0 || nj < 0 || ni >= G || nj >= G) continue;
          const v = H[nj * G + ni];
          if (!isNaN(v)) { sum += v; cnt++; }
        }
        if (cnt) { next[k] = sum / cnt; changed = true; }
      }
      H.set(next);
      if (!changed) break;
    }
    for (let k = 0; k < H.length; k++) if (isNaN(H[k])) H[k] = 0;
    // One box-blur pass for smoothness.
    const out = H.slice();
    for (let j = 0; j < G; j++) for (let i = 0; i < G; i++) {
      let sum = 0, cnt = 0;
      for (let dj = -1; dj <= 1; dj++) for (let di = -1; di <= 1; di++) {
        const ni = i + di, nj = j + dj;
        if (ni < 0 || nj < 0 || ni >= G || nj >= G) continue;
        sum += H[nj * G + ni]; cnt++;
      }
      out[j * G + i] = sum / cnt;
    }
    H.set(out);
  }

  // ---- camera & projection -------------------------------------------------
  function basis() {
    const eye = [
      cam.tx + cam.dist * Math.cos(cam.pitch) * Math.sin(cam.yaw),
      cam.ty + cam.dist * Math.sin(cam.pitch),
      cam.tz + cam.dist * Math.cos(cam.pitch) * Math.cos(cam.yaw),
    ];
    const f = norm(sub([cam.tx, cam.ty, cam.tz], eye));
    const r = norm(cross(f, [0, 1, 0]));
    const u = cross(r, f);
    return { eye, f, r, u };
  }

  function project(P, b, W, Hh, focal) {
    const d = sub(P, b.eye);
    const cz = dot(d, b.f);
    if (cz < NEAR) return null;
    const cx = dot(d, b.r), cy = dot(d, b.u);
    return { sx: W / 2 + (cx / cz) * focal, sy: Hh / 2 - (cy / cz) * focal, sz: cz };
  }

  // ---- node visual role (shared selection semantics with the 2D view) ------
  function nodeStyle(n) {
    const sel = state.sel != null;
    const isSel = n.id === state.sel;
    let base = PAL.node, ring = null, rscale = 1, lab = false;
    if (state.rootIds.has(n.id)) base = PAL.root;
    if (state.dupVersionIds.has(n.version_id)) ring = PAL.dup;
    if (mode === 'tda' && feature) {
      if (feature.nodeIds.has(n.id)) { base = PAL.gen; ring = PAL.gen; rscale = 1.5; lab = true; }
      else return { base, ring, dim: true, rscale: 0.8, lab: false };
    }
    if (sel) {
      if (isSel) { base = PAL.center; rscale = 1.5; lab = true; }
      else if (state.fwd.has(n.id)) { base = PAL.fwd; lab = true; }
      else if (state.impact.has(n.id)) { base = PAL.impact; lab = true; }
      else return { base, ring, dim: true, rscale: 0.85, lab: false };
    }
    if (state.matches && state.matches.has(n.id)) { ring = PAL.accent; lab = true; }
    if (hover === n.id) lab = true;
    return { base, ring, dim: false, rscale, lab };
  }

  // ---- draw ----------------------------------------------------------------
  function draw() {
    rafPending = false;
    if (!ctx || !active) return;
    const W = canvas.clientWidth || 800, Hh = canvas.clientHeight || 600;
    const focal = (Hh / 2) / Math.tan(FOV / 2);
    const b = basis();

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, Hh);
    ctx.fillStyle = rgb(PAL.bg);
    ctx.fillRect(0, 0, W, Hh);

    // Project nodes once; cache for edges, picking, labels.
    screen.clear();
    let maxz = 1, minz = Infinity;
    for (const n of state.nodes) {
      const P = pos3.get(n.id);
      if (!P) continue;
      const s = project([P.x, P.y, P.z], b, W, Hh, focal);
      if (s) { maxz = Math.max(maxz, s.sz); minz = Math.min(minz, s.sz); }
      screen.set(n.id, s);
    }
    const fog = (z) => { // 0 near → 1 far, for atmospheric fade
      if (!isFinite(minz)) return 0;
      return Math.max(0, Math.min(1, (z - minz) / (maxz - minz || 1)));
    };

    const items = []; // {z, paint}
    if (decor) collectDecor(items, b, W, Hh, focal, fog);
    collectEdges(items, fog);
    collectNodes(items, fog);

    items.sort((a, c) => c.z - a.z); // far → near
    for (const it of items) it.paint();

    drawLabels(W, Hh);
    drawHud(W, Hh);
  }

  function collectDecor(items, b, W, Hh, focal, fog) {
    if (decor.kind === 'layers') {
      for (const pl of decor.planes) {
        const R = Math.max(pl.radius, NODE_SPACE * 2);
        const corners = [[-R, pl.y, -R], [R, pl.y, -R], [R, pl.y, R], [-R, pl.y, R]]
          .map((c) => project(c, b, W, Hh, focal));
        if (corners.some((c) => !c)) continue;
        const cz = corners.reduce((s, c) => s + c.sz, 0) / 4;
        items.push({
          z: cz + 1e6, paint: () => { // planes behind their nodes
            ctx.beginPath();
            ctx.moveTo(corners[0].sx, corners[0].sy);
            for (let i = 1; i < 4; i++) ctx.lineTo(corners[i].sx, corners[i].sy);
            ctx.closePath();
            ctx.fillStyle = rgb(PAL.accent, 0.05);
            ctx.fill();
            ctx.strokeStyle = rgb(PAL.accent, 0.16);
            ctx.lineWidth = 1;
            ctx.stroke();
          },
        });
      }
    } else if (decor.kind === 'fibers') {
      for (const fb of decor.fibers) {
        const a = project([fb.x, 0, fb.z], b, W, Hh, focal);
        const top = project([fb.x, fb.top + VER_GAP * 0.4, fb.z], b, W, Hh, focal);
        if (!a || !top) continue;
        const col = fb.dup ? PAL.dup : PAL.muted;
        items.push({
          z: (a.sz + top.sz) / 2, paint: () => {
            ctx.strokeStyle = rgb(col, fb.dup ? 0.55 : 0.22);
            ctx.lineWidth = fb.dup ? 2.4 : 1;
            ctx.beginPath(); ctx.moveTo(a.sx, a.sy); ctx.lineTo(top.sx, top.sy); ctx.stroke();
            ctx.fillStyle = rgb(col, 0.7);
            ctx.beginPath(); ctx.arc(a.sx, a.sy, 2.5, 0, 7); ctx.fill();
          },
        });
      }
    } else if (decor.kind === 'terrain') {
      collectTerrain(items, b, W, Hh, focal);
    }
  }

  function collectTerrain(items, b, W, Hh, focal) {
    const { H, G, cell, maxSig } = decor;
    const hMax = TERRAIN_H || 1;
    for (let j = 0; j < G - 1; j++) for (let i = 0; i < G - 1; i++) {
      const p = (ii, jj) => {
        const x = -TERRAIN_S + ii * cell, z = -TERRAIN_S + jj * cell, y = H[jj * G + ii];
        return { w: [x, y, z], s: project([x, y, z], b, W, Hh, focal), y };
      };
      const a = p(i, j), c = p(i + 1, j), d = p(i + 1, j + 1), e = p(i, j + 1);
      if (!a.s || !c.s || !d.s || !e.s) continue;
      const avgY = (a.y + c.y + d.y + e.y) / 4;
      const t = avgY / hMax;
      // Lambert-ish shade from a fixed light using the quad's screen normal.
      const ux = c.s.sx - a.s.sx, uy = c.s.sy - a.s.sy;
      const vx = e.s.sx - a.s.sx, vy = e.s.sy - a.s.sy;
      const fn = (ux * vy - uy * vx);
      const shade = 0.72 + 0.28 * Math.max(-1, Math.min(1, fn / 4000));
      const col = rampColor(t).map((q) => q * shade);
      const cz = (a.s.sz + c.s.sz + d.s.sz + e.s.sz) / 4;
      items.push({
        z: cz + 5e5, paint: () => {
          ctx.beginPath();
          ctx.moveTo(a.s.sx, a.s.sy); ctx.lineTo(c.s.sx, c.s.sy);
          ctx.lineTo(d.s.sx, d.s.sy); ctx.lineTo(e.s.sx, e.s.sy); ctx.closePath();
          ctx.fillStyle = rgb(col, 0.92);
          ctx.fill();
          ctx.strokeStyle = rgb(col.map((q) => q * 0.7), 0.5);
          ctx.lineWidth = 0.5; ctx.stroke();
        },
      });
    }
  }

  function collectEdges(items, fog) {
    const sel = state.sel != null;
    const feat = mode === 'tda' && feature;
    for (const e of state.edges) {
      const a = screen.get(e.source), c = screen.get(e.target);
      if (!a || !c) continue;
      const z = (a.sz + c.sz) / 2;
      let col = PAL.edge, w = 1, alpha = 0.5 - 0.35 * fog(z);
      const back = (e.flags & 16) !== 0; // BACK_EDGE — closes a cycle
      if (feat) {
        const key = e.source + '>' + e.target;
        if (feature.edgeKeys.has(key)) { col = PAL.gen; w = 2.6; alpha = 0.95; }
        else { alpha = 0.05; }
      } else if (sel) {
        if (state.fwd.has(e.source) && state.fwd.has(e.target)) { col = PAL.fwd; w = 2; alpha = 0.9; }
        else if (state.impact.has(e.source) && state.impact.has(e.target)) { col = PAL.impact; w = 2; alpha = 0.9; }
        else alpha = 0.06;
      } else if (back) { col = PAL.impact; alpha = 0.6; }
      items.push({
        z, paint: () => {
          ctx.strokeStyle = rgb(col, alpha);
          ctx.lineWidth = w;
          ctx.beginPath(); ctx.moveTo(a.sx, a.sy); ctx.lineTo(c.sx, c.sy); ctx.stroke();
        },
      });
    }
  }

  function collectNodes(items, fog) {
    const focalR = 8;
    for (const n of state.nodes) {
      const s = screen.get(n.id);
      if (!s) continue;
      const st = nodeStyle(n);
      const f = fog(s.sz);
      const r = Math.max(2, Math.min(16, focalR * st.rscale * (cam.dist / s.sz) * 0.9));
      const baseCol = st.dim ? mix(st.base, PAL.bg, 0.55 + 0.25 * f) : mix(st.base, PAL.bg, 0.15 * f);
      items.push({
        z: s.sz, paint: () => {
          // Fake-sphere radial gradient for a touch of depth.
          const g = ctx.createRadialGradient(s.sx - r * 0.3, s.sy - r * 0.3, r * 0.2, s.sx, s.sy, r);
          g.addColorStop(0, rgb(mix(baseCol, [255, 255, 255], 0.35)));
          g.addColorStop(1, rgb(baseCol));
          ctx.fillStyle = g;
          ctx.beginPath(); ctx.arc(s.sx, s.sy, r, 0, 7); ctx.fill();
          if (st.ring) {
            ctx.strokeStyle = rgb(st.ring, st.dim ? 0.4 : 0.95);
            ctx.lineWidth = 2; ctx.stroke();
          } else {
            ctx.strokeStyle = rgb([11, 13, 18], 0.9); ctx.lineWidth = 1; ctx.stroke();
          }
        },
      });
    }
  }

  function drawLabels(W, Hh) {
    const small = state.nodes.length <= 70;
    ctx.font = '11px ui-monospace, SFMono-Regular, Menlo, monospace';
    ctx.textBaseline = 'middle';
    for (const n of state.nodes) {
      const s = screen.get(n.id);
      if (!s) continue;
      const st = nodeStyle(n);
      if (!(st.lab || small) || st.dim) continue;
      if (s.sx < -40 || s.sx > W + 40 || s.sy < 0 || s.sy > Hh) continue;
      const text = lbl(n.purl);
      ctx.lineWidth = 3; ctx.strokeStyle = rgb([11, 13, 18], 0.85);
      ctx.strokeText(text, s.sx + 9, s.sy);
      ctx.fillStyle = rgb(PAL.ink, st.lab ? 1 : 0.8);
      ctx.fillText(text, s.sx + 9, s.sy);
    }
  }

  function drawHud(W, Hh) {
    const tags = {
      layered: 'LAYERED · depth → height', fiber: 'FIBER · versions stack',
      terrain: 'TERRAIN · ' + signal, tda: 'TDA · ' + (feature ? 'H' + feature.dim + ' generator' : 'pick a feature'),
    };
    ctx.font = '11px ui-monospace, monospace';
    ctx.textBaseline = 'top';
    ctx.fillStyle = rgb(PAL.muted, 0.9);
    ctx.fillText(tags[mode] || mode, 12, 10);
    if (decor && decor.kind === 'terrain') drawTerrainLegend(W, Hh);
  }

  function drawTerrainLegend(W, Hh) {
    const x = 12, y = Hh - 24, w = 140, h = 8;
    for (let i = 0; i < w; i++) {
      ctx.fillStyle = rgb(rampColor(i / w));
      ctx.fillRect(x + i, y, 1, h);
    }
    ctx.fillStyle = rgb(PAL.muted);
    ctx.textBaseline = 'bottom';
    ctx.fillText('low', x, y - 1);
    ctx.fillText('high', x + w - 22, y - 1);
  }

  // ---- picking -------------------------------------------------------------
  function pick(mx, my) {
    let best = null, bestD = 1e9;
    for (const n of state.nodes) {
      const s = screen.get(n.id);
      if (!s) continue;
      const r = Math.max(6, Math.min(18, 8 * (cam.dist / s.sz))) + 4;
      const d = (s.sx - mx) ** 2 + (s.sy - my) ** 2;
      if (d < r * r && s.sz < bestD) { best = n; bestD = s.sz; }
    }
    return best;
  }

  // ---- camera fit ----------------------------------------------------------
  function fit() {
    let cx = 0, cy = 0, cz = 0, k = 0;
    let minx = Infinity, maxx = -Infinity, miny = Infinity, maxy = -Infinity, minz = Infinity, maxz = -Infinity;
    for (const P of pos3.values()) {
      cx += P.x; cy += P.y; cz += P.z; k++;
      minx = Math.min(minx, P.x); maxx = Math.max(maxx, P.x);
      miny = Math.min(miny, P.y); maxy = Math.max(maxy, P.y);
      minz = Math.min(minz, P.z); maxz = Math.max(maxz, P.z);
    }
    if (!k) return;
    cam.tx = cx / k; cam.ty = cy / k; cam.tz = cz / k;
    const radius = Math.max(maxx - minx, maxy - miny, maxz - minz, 100) / 2;
    cam.dist = Math.max(MIN_DIST, Math.min(MAX_DIST, radius / Math.tan(FOV / 2) * 1.7));
  }

  // ---- interaction ---------------------------------------------------------
  function install() {
    let down = false, drag = false, lastX = 0, lastY = 0, moved = 0, panning = false, pid = null;
    canvas.addEventListener('pointerdown', (ev) => {
      down = true; drag = false; moved = 0; pid = ev.pointerId;
      lastX = ev.clientX; lastY = ev.clientY;
      panning = ev.shiftKey || ev.button === 1 || ev.button === 2;
      spin = false;
    });
    canvas.addEventListener('pointermove', (ev) => {
      const rect = canvas.getBoundingClientRect();
      if (!down) {
        const h = pick(ev.clientX - rect.left, ev.clientY - rect.top);
        const id = h ? h.id : null;
        if (id !== hover) { hover = id; canvas.style.cursor = id ? 'pointer' : 'grab'; invalidate(); }
        return;
      }
      const dx = ev.clientX - lastX, dy = ev.clientY - lastY;
      lastX = ev.clientX; lastY = ev.clientY; moved += Math.abs(dx) + Math.abs(dy);
      if (!drag && moved > 4) { drag = true; try { canvas.setPointerCapture(pid); } catch (_) {} }
      if (!drag) return;
      if (panning) {
        const b = basis(), k = cam.dist * 0.0016;
        cam.tx -= (b.r[0] * dx - b.u[0] * dy) * k;
        cam.ty -= (b.r[1] * dx - b.u[1] * dy) * k;
        cam.tz -= (b.r[2] * dx - b.u[2] * dy) * k;
      } else {
        cam.yaw -= dx * 0.008;
        cam.pitch = Math.max(-1.5, Math.min(1.5, cam.pitch + dy * 0.008));
      }
      invalidate();
    });
    const up = (ev) => {
      if (!down) return;
      down = false;
      if (drag) { try { canvas.releasePointerCapture(ev.pointerId); } catch (_) {} }
      else {
        const rect = canvas.getBoundingClientRect();
        const h = pick(ev.clientX - rect.left, ev.clientY - rect.top);
        if (h && pickCb) pickCb(h);
      }
      drag = false;
    };
    canvas.addEventListener('pointerup', up);
    canvas.addEventListener('pointercancel', up);
    canvas.addEventListener('contextmenu', (ev) => ev.preventDefault());
    canvas.addEventListener('wheel', (ev) => {
      ev.preventDefault();
      cam.dist = Math.max(MIN_DIST, Math.min(MAX_DIST, cam.dist * (ev.deltaY < 0 ? 0.9 : 1.111)));
      invalidate();
    }, { passive: false });
    window.addEventListener('resize', () => { if (active) resize(); });
  }

  function invalidate() {
    if (rafPending || !active) return;
    rafPending = true;
    requestAnimationFrame(draw);
  }

  function spinLoop() {
    if (!spin || !active) { spinRaf = 0; return; }
    cam.yaw += 0.0032;
    draw();
    spinRaf = requestAnimationFrame(spinLoop);
  }

  function resize() {
    if (!canvas) return;
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = canvas.clientWidth || 800, h = canvas.clientHeight || 600;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    invalidate();
  }

  // ---- public API ----------------------------------------------------------
  return {
    init(cv) {
      canvas = cv;
      ctx = cv.getContext('2d');
      install();
    },
    setData() {
      computeMetrics();
      buildLayout();
      fit();
      invalidate();
    },
    setMode(m) {
      mode = m;
      if (m !== 'tda') feature = null;
      buildLayout();
      fit();
      invalidate();
    },
    getMode() { return mode; },
    setSignal(s) {
      signal = s;
      if (mode === 'terrain') { buildLayout(); fit(); invalidate(); }
    },
    show() {
      active = true;
      resize();
      invalidate();
    },
    resetView() { fit(); invalidate(); },
    hide() { active = false; spin = false; },
    isActive() { return active; },
    redraw() { invalidate(); },
    resize,
    toggleSpin() { spin = !spin; if (spin && !spinRaf) spinLoop(); return spin; },
    focus(id) {
      const P = pos3.get(id);
      if (P) { cam.tx = P.x; cam.ty = P.y; cam.tz = P.z; invalidate(); }
    },
    setFeatureSupport(dim, objects) {
      const nodeIds = new Set(), edgeKeys = new Set();
      for (const o of objects) {
        if (o.type === 'node') nodeIds.add(o.id);
        else if (o.type === 'edge') {
          edgeKeys.add(o.source + '>' + o.target);
          nodeIds.add(o.source); nodeIds.add(o.target);
        }
      }
      feature = { dim, nodeIds, edgeKeys };
      invalidate();
    },
    clearFeature() { feature = null; invalidate(); },
    onPick(cb) { pickCb = cb; },
  };
})();

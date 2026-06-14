/* Colors for the canvas / R3F scene. The SINGLE source of truth is the
 * `@theme` block in src/app/globals.css; three.js and inline SVG can't consume
 * `var(--x)`, so this module RESOLVES those CSS variables to hex strings at
 * runtime (browser). Literal fallbacks — kept equal to the @theme values — cover
 * the no-DOM build prerender, where the canvas never actually renders.
 * Roles match the explorer's legend so 2D and 3D read identically. */

/** Read a CSS custom property off :root, falling back when there's no DOM. */
function cssVar(name: string, fallback: string): string {
  if (typeof document === 'undefined') return fallback;
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

/* Fallback literals MUST mirror globals.css `@theme`. */
const FALLBACK = {
  bg: '#030303', // --color-deep-field
  node: '#8aa6a3', // --color-node
  root: '#a99bc0', // --color-node-root
  center: '#c9a45c', // --color-node-center  (selected)
  fwd: '#6fa890', // --color-node-fwd
  impact: '#cf695f', // --color-node-impact
  dup: '#c77fa0', // --color-node-dup
  edge: '#3a3a38', // --color-edge
  accent: '#8fb8ce', // --color-node-match   (search match)
  ink: '#e8e4d8', // --color-bone
  muted: '#a8a49a', // --color-archive
  gen: '#d8c06a', // --color-node-gen
  void: '#9e84c4', // --color-node-void
} as const;

type Role = keyof typeof FALLBACK;

const VAR: Record<Role, string> = {
  bg: '--color-deep-field',
  node: '--color-node',
  root: '--color-node-root',
  center: '--color-node-center',
  fwd: '--color-node-fwd',
  impact: '--color-node-impact',
  dup: '--color-node-dup',
  edge: '--color-edge',
  accent: '--color-node-match',
  ink: '--color-bone',
  muted: '--color-archive',
  gen: '--color-node-gen',
  void: '--color-node-void',
};

function resolve(): Record<Role, string> {
  const out = { ...FALLBACK } as Record<Role, string>;
  for (const k of Object.keys(VAR) as Role[]) out[k] = cssVar(VAR[k], FALLBACK[k]);
  return out;
}

/** Resolved role palette (CSS vars in the browser, literals during prerender). */
export const PALETTE: Record<Role, string> = resolve();

/** Terrain / importance elevation ramp (low → high). A viridis map: perceptually
 *  uniform, monotonically increasing in luminance, and colour-vision-deficiency
 *  safe (no red/green confusion) — kept as-is from the original encoding. */
const RAMP: Array<[number, [number, number, number]]> = [
  [0.0, [68, 1, 84]], // dark violet
  [0.25, [59, 82, 139]], // blue
  [0.5, [33, 145, 140]], // teal
  [0.75, [94, 201, 98]], // green
  [1.0, [253, 231, 37]], // yellow
];

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/** A `[r,g,b]` (0–255) sample of the elevation ramp at `t ∈ [0,1]`. */
export function rampRgb(t: number): [number, number, number] {
  const x = Math.max(0, Math.min(1, t));
  for (let i = 1; i < RAMP.length; i++) {
    if (x <= RAMP[i][0]) {
      const u = (x - RAMP[i - 1][0]) / (RAMP[i][0] - RAMP[i - 1][0] || 1);
      const a = RAMP[i - 1][1];
      const b = RAMP[i][1];
      return [lerp(a[0], b[0], u), lerp(a[1], b[1], u), lerp(a[2], b[2], u)];
    }
  }
  return RAMP[RAMP.length - 1][1];
}

/** Ramp sample as a CSS/three hex string. */
export function rampHex(t: number): string {
  const [r, g, b] = rampRgb(t);
  return `#${[r, g, b].map((v) => Math.round(v).toString(16).padStart(2, '0')).join('')}`;
}

export function hexToRgb(hex: string): [number, number, number] {
  const h = hex.replace('#', '');
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

export function rgbToHex([r, g, b]: [number, number, number]): string {
  return `#${[r, g, b].map((v) => Math.round(Math.max(0, Math.min(255, v))).toString(16).padStart(2, '0')).join('')}`;
}

/** Mix two hex colours; `t=0` → a, `t=1` → b. */
export function mixHex(a: string, b: string, t: number): string {
  const ca = hexToRgb(a);
  const cb = hexToRgb(b);
  return rgbToHex([lerp(ca[0], cb[0], t), lerp(ca[1], cb[1], t), lerp(ca[2], cb[2], t)]);
}

/** Fade a colour toward the background (used for dimmed/far elements). */
export function dimHex(hex: string, t: number): string {
  return mixHex(hex, PALETTE.bg, t);
}

/** A compact display label for a purl ("pkg:cargo/serde@1.0.0" → "serde@1.0.0").
 * Scoped npm names are stored percent-encoded ("@scope%2Fname"); decode the
 * `%2F` back to a slash so they read naturally ("@scope/name"). */
export function shortLabel(purl: string): string {
  if (!purl) return '?';
  let s = purl.replace(/^pkg:[^/]+\//, '').replace(/%2[Ff]/g, '/');
  if (s.length > 30) s = s.slice(0, 29) + '…';
  return s;
}

/** Colors for inline SVG / canvas chrome (TDA diagram, candidate strip), kept
 * here as the single source so panels never re-type theme hexes in markup.
 * Surface/line chrome mirrors the @theme neutral ramp; dim0/1/2 are the three
 * homology-dimension hues (kept distinct + CVD-aware). */
export const CHART_COLORS = {
  frame: cssVar('--color-charcoal', '#111111'),
  grid: cssVar('--color-iron', '#2b2b2b'),
  rail: cssVar('--color-graphite', '#1c1c1c'),
  diagonal: cssVar('--color-dust', '#6f6f68'),
  highlight: PALETTE.accent,
  accent2: cssVar('--color-amber', '#bfa15a'),
  dim0: PALETTE.accent, // H0 — cool cyan (search-match hue)
  dim1: cssVar('--color-amber', '#bfa15a'), // H1 — amber
  dim2: PALETTE.void, // H2 — void violet
} as const;

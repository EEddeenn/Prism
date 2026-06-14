/* Colours for the canvas / R3F scene (the Tailwind theme mirrors these hexes).
 * Roles match the original explorer's legend so 2D and 3D read identically. */

export const PALETTE = {
  bg: '#0e1016',
  node: '#3b82f6',
  root: '#a78bfa',
  center: '#ffb454',
  fwd: '#34d399',
  impact: '#f87171',
  dup: '#f472b6',
  edge: '#3a4150',
  accent: '#5aa9ff',
  ink: '#e7e9f0',
  muted: '#8b93a7',
  gen: '#ffd166',
};

/** Terrain / importance elevation ramp (low → high). A viridis map: perceptually
 *  uniform, monotonically increasing in luminance, and colour-vision-deficiency
 *  safe (no red/green confusion) — unlike a temperature ramp whose amber→red top
 *  reads identically to a deuteranope and dips in luminance at the top. */
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

/** Colours for inline SVG / canvas chrome (TDA diagram, candidate strip), kept
 * here as the single source so panels never re-type theme hexes in markup. */
export const CHART_COLORS = {
  frame: '#1d212e', // panel-2
  grid: '#2a2f3c', // line
  rail: '#252a3a', // panel-3
  diagonal: '#353c4d', // line-2
  highlight: '#5aa9ff', // accent
  accent2: '#ffb454', // accent-2
  dim0: '#5aa9ff', // H0 — keep the original accent blue (was historically blue)
  dim1: '#ffb454', // H1 — accent-2 amber
} as const;

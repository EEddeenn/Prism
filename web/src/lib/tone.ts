/* The single semantic-tone vocabulary for the whole UI. Every badge, meter,
 * callout, bar, text tint, and dot maps through this — replacing the old
 * per-component Tone / CalloutTone / MeterTone enums and the ~7 duplicated
 * score->tone helpers (scoreTone, deltaTone, convergenceTone, DRIFT_TONE,
 * CYCLE_TONE, FLAG_TONE, KIND_TONE). The class maps reference design tokens
 * defined in globals.css `@theme` (info=cyan-gray, warn=amber, bad=raised-red,
 * good=green), so a tone is one name that resolves to a coherent set of styles. */

export type Tone = 'neutral' | 'info' | 'good' | 'warn' | 'bad' | 'accent';

/** Foreground/text color per tone. */
export const toneText: Record<Tone, string> = {
  neutral: 'text-archive',
  info: 'text-info',
  good: 'text-good',
  warn: 'text-warn',
  bad: 'text-bad',
  accent: 'text-bone',
};

/** Border color per tone (the soft -bd tints; never relies on color alone). */
export const toneBorder: Record<Tone, string> = {
  neutral: 'border-iron',
  info: 'border-info-bd',
  good: 'border-good-bd',
  warn: 'border-warn-bd',
  bad: 'border-bad-bd',
  accent: 'border-dust',
};

/** Soft tinted surface per tone (callouts, nested panels). */
export const toneSurface: Record<Tone, string> = {
  neutral: 'bg-charcoal',
  info: 'bg-info-bg',
  good: 'bg-good-bg',
  warn: 'bg-warn-bg',
  bad: 'bg-bad-bg',
  accent: 'bg-charcoal',
};

/** Solid fill per tone (meter fills, swatches, dots). */
export const toneFill: Record<Tone, string> = {
  neutral: 'bg-dust',
  info: 'bg-info',
  good: 'bg-good',
  warn: 'bg-warn',
  bad: 'bg-bad',
  accent: 'bg-bone',
};

/** Generic lookup with a fallback — for verdict/kind strings whose exact set
 *  varies. `pickTone(MAP, key)` keeps panels from re-deriving class strings. */
export function pickTone<K extends string>(map: Record<K, Tone>, key: string, fallback: Tone = 'neutral'): Tone {
  return (map as Record<string, Tone>)[key] ?? fallback;
}

/** Risk/severity score (0–100) → tone. CVD-distinct 4-band ramp:
 *  low=good(green) · moderate=info(cyan) · elevated=warn(amber) · high=bad(red). */
export function severity(score: number): Tone {
  if (score >= 75) return 'bad';
  if (score >= 50) return 'warn';
  if (score >= 25) return 'info';
  return 'good';
}

/** A signed change → tone. `moreIsWorse` flips the sentiment (e.g. risk up = bad,
 *  but coverage up = good). Zero/near-zero reads neutral. */
export function deltaTone(changePct: number, moreIsWorse = true): Tone {
  if (Math.abs(changePct) < 0.5) return 'neutral';
  const worse = changePct > 0 ? moreIsWorse : !moreIsWorse;
  return worse ? 'bad' : 'good';
}

/** Semver drift severity for duplicate clusters. */
export const DRIFT_TONE: Record<string, Tone> = { major: 'bad', minor: 'warn', patch: 'info', none: 'neutral' };
export function driftTone(drift: string): Tone {
  return pickTone(DRIFT_TONE, drift, 'neutral');
}

/** Edge-kind → tone for the edge-lens badges. */
export const EDGE_KIND_TONE: Record<string, Tone> = {
  runtime: 'info',
  dev: 'neutral',
  build: 'accent',
  peer: 'warn',
  optional: 'neutral',
  test: 'neutral',
  plugin: 'good',
  workspace: 'warn',
  unknown: 'neutral',
};
export function edgeKindTone(kind: string): Tone {
  return pickTone(EDGE_KIND_TONE, kind, 'neutral');
}

/** Supply-chain smell flags → tone (peer-unmet is the loudest). */
export const FLAG_TONE: Record<string, Tone> = {
  'peer-unmet': 'bad',
  overridden: 'warn',
  'optional-omitted': 'neutral',
  'back-edge': 'warn',
};
export function flagTone(flag: string): Tone {
  return pickTone(FLAG_TONE, flag, 'neutral');
}

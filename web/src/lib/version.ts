/* Lightweight semver-ish ordering for stacking versions in the fiber view. Not a
 * full SemVer engine (that lives in Rust) — just enough to order a family's
 * versions sensibly along the height axis. */

interface VerKey {
  core: number[];
  pre: string;
}

export function verKey(purl: string): VerKey {
  const at = purl.lastIndexOf('@');
  const v = at >= 0 ? purl.slice(at + 1) : purl;
  const dash = v.indexOf('-');
  const core = (dash >= 0 ? v.slice(0, dash) : v).split('.').map((x) => parseInt(x, 10) || 0);
  const pre = dash >= 0 ? v.slice(dash + 1) : '';
  return { core, pre };
}

/** Compare two purls by version. A release sorts after its prereleases. */
export function verCmp(a: string, b: string): number {
  const ka = verKey(a);
  const kb = verKey(b);
  const n = Math.max(ka.core.length, kb.core.length);
  for (let i = 0; i < n; i++) {
    const d = (ka.core[i] || 0) - (kb.core[i] || 0);
    if (d) return d;
  }
  if (!ka.pre && kb.pre) return 1;
  if (ka.pre && !kb.pre) return -1;
  return ka.pre < kb.pre ? -1 : ka.pre > kb.pre ? 1 : 0;
}

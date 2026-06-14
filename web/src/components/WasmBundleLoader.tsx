'use client';

/* Top-level workspace gate for the static-export Prism web app.
 *
 * On mount it inspects `?bundle=<sha>` in the page URL and resolves to one
 * of four states:
 *
 *   - idle    no `?bundle=` parameter — show the drop zone.
 *   - loading bundle bytes found in IndexedDB; decoding them through WASM.
 *   - loaded  WASM `load_bundle` succeeded; the singleton workspace is open
 *             and the children (the actual explorer) may mount.
 *   - error   either the bytes were not in this browser's IndexedDB, or
 *             `load_bundle` threw. The drop zone re-renders with a hint so
 *             the user can re-upload the same `.prismbundle`.
 *
 * The component is `'use client'` and all WASM touches happen inside a
 * `useEffect`, so Next's static-export pre-render never executes this code
 * path. That keeps the Webpack 5 SSR-wasm bug (next.js #25852) at arm's
 * length without needing a `next/dynamic({ ssr: false })` indirection at
 * every mount site.
 */

import { useEffect, useState, type ReactNode } from 'react';
import { getBundle } from '@/lib/bundleStore';
import { wasm } from '@/lib/wasm';
import { Callout, Eyebrow } from '@/design-system';
import BundleDropzone from './BundleDropzone';
import Logo from './Logo';

const REPO_URL = 'https://github.com/EEddeenn/Prism';

/** Octocat — the canonical GitHub mark. Inline so the footer link works
 *  in any deploy environment without an external request. */
function GitHubIcon({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="currentColor"
      width={14}
      height={14}
      aria-hidden
      className={className}
    >
      <path
        fillRule="evenodd"
        d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"
      />
    </svg>
  );
}

/** The landing-page footer: a single, subtle row with the GitHub link.
 *  Used only by the non-loaded states (idle / missing / error) — once a
 *  bundle is open the Explorer takes the full viewport and renders its
 *  own chrome. */
function LandingFooter() {
  return (
    <footer className="flex items-center justify-center gap-2 py-4 text-meta text-archive border-t border-iron">
      <a
        href={REPO_URL}
        target="_blank"
        rel="noopener noreferrer"
        aria-label="Prism source on GitHub"
        className="inline-flex items-center gap-1.5 font-mono hover:text-bone transition-colors duration-150"
      >
        <GitHubIcon />
        <span>EEddeenn/Prism</span>
      </a>
    </footer>
  );
}

type State =
  | { kind: 'idle' }
  | { kind: 'loading'; sha: string }
  | { kind: 'loaded' }
  | { kind: 'missing'; sha: string }
  | { kind: 'error'; sha: string; error: string };

function readBundleParam(): string | null {
  if (typeof window === 'undefined') return null;
  const sp = new URLSearchParams(window.location.search);
  // Lowercase the param so a shared link with uppercase hex (some chat
  // clients normalize URLs that way) still hits the same IndexedDB key
  // the dropzone wrote — `crypto.subtle.digest` produces lowercase hex.
  const sha = sp.get('bundle')?.toLowerCase();
  // Accept only the canonical 64-char lowercase hex digest; anything else is
  // treated as 'no bundle' rather than triggering an IndexedDB miss.
  if (!sha || !/^[0-9a-f]{64}$/.test(sha)) return null;
  return sha;
}

export default function WasmBundleLoader({ children }: { children: ReactNode }) {
  // Start in 'idle'. We resolve the real state in the effect below to ensure
  // every WASM / IndexedDB touch happens client-side only.
  const [state, setState] = useState<State>({ kind: 'idle' });
  // Bumped on every `popstate` event so the URL-reading effect re-runs after
  // browser back/forward navigation. Today the only entry path is a full
  // reload via `window.location.search = ...` in BundleDropzone, but this
  // keeps the gate honest if SPA-style routing is added later.
  const [tick, setTick] = useState(0);

  useEffect(() => {
    const onPop = () => setTick((t) => t + 1);
    window.addEventListener('popstate', onPop);
    return () => window.removeEventListener('popstate', onPop);
  }, []);

  useEffect(() => {
    let cancelled = false;
    const sha = readBundleParam();
    if (!sha) {
      setState({ kind: 'idle' });
      return;
    }
    setState({ kind: 'loading', sha });
    (async () => {
      try {
        const bytes = await getBundle(sha);
        if (cancelled) return;
        if (!bytes) {
          setState({ kind: 'missing', sha });
          return;
        }
        const m = await wasm();
        if (cancelled) return;
        // load_bundle throws on a corrupt envelope / TOC / SHA mismatch.
        // We don't actually need the summary here — useGraphStore.loadRuns()
        // will fetch the run list via the WASM dispatcher once it mounts.
        m.load_bundle(bytes);
        if (cancelled) return;
        setState({ kind: 'loaded' });
      } catch (e) {
        if (cancelled) return;
        const msg = e instanceof Error ? e.message : String(e);
        setState({ kind: 'error', sha, error: msg });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [tick]);

  if (state.kind === 'loaded') return <>{children}</>;

  if (state.kind === 'loading') {
    return (
      <div
        className="flex flex-col items-center justify-center h-full gap-3 text-center px-4 bg-deep-field"
        aria-live="polite"
        aria-busy="true"
      >
        <Logo size={44} className="text-cyan-gray opacity-80 animate-pulse" />
        <Eyebrow>Decoding</Eyebrow>
        <p className="text-bone text-data">Loading bundle…</p>
        <p className="text-archive text-data-s font-mono break-all max-w-md">{state.sha}</p>
      </div>
    );
  }

  // idle / missing / error: render the polished landing page (hero +
  // drop zone + footer). When `?bundle=` was present but the bytes were
  // missing or rejected, an extra hint appears between the dropzone and
  // the deeper explanation.
  const hint =
    state.kind === 'missing'
      ? 'This bundle is not in this browser — re-upload the same .prismbundle to reopen it.'
      : state.kind === 'error'
        ? `Failed to open bundle: ${state.error}`
        : null;

  return (
    <div className="flex flex-col min-h-full bg-deep-field">
      <main className="flex-1 flex flex-col items-center justify-center gap-7 px-6 py-12">
        {/* Hero: logo + brand + tagline. The vertex tints on the logo
            (H₀ / H₁ / H₂ graph-role tokens) echo the palette used in the
            TDA views, so the brand mark also reads as a small legend. */}
        <header className="flex flex-col items-center gap-3 text-center">
          <Logo size={64} className="text-cyan-gray" />
          <h1 className="text-figure-l font-semibold tracking-tight text-bone leading-none">
            Prism
          </h1>
          <Eyebrow>dependency topology</Eyebrow>
        </header>

        {/* Dropzone column, capped at a comfortable reading width so the
            dashed border doesn't stretch across an ultrawide monitor. */}
        <div className="w-full max-w-md flex flex-col gap-3">
          <BundleDropzone />
          {hint &&
            (state.kind === 'error' ? (
              <Callout tone="bad" role="alert" showWord className="break-words">
                {hint}
              </Callout>
            ) : (
              <p className="text-data-s text-center max-w-md break-words mx-auto text-archive">
                {hint}
              </p>
            ))}
          <p className="text-archive text-data-s text-center">
            Or produce one with{' '}
            <code className="font-mono bg-charcoal px-1.5 py-px rounded-control">prism analyze</code>
            {' / '}
            <code className="font-mono bg-charcoal px-1.5 py-px rounded-control">prism export bundle</code>.
          </p>
        </div>
      </main>

      <LandingFooter />
    </div>
  );
}

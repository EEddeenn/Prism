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
import BundleDropzone from './BundleDropzone';

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
      <div className="flex flex-col items-center justify-center h-full gap-2 text-center">
        <p className="text-ink text-sm">Loading bundle…</p>
        <p className="text-muted text-xs font-mono break-all max-w-md">{state.sha}</p>
      </div>
    );
  }

  // idle / missing / error: render the drop zone, with a hint when relevant.
  const hint =
    state.kind === 'missing'
      ? 'This bundle is not in this browser — re-upload the same .prismbundle to reopen it.'
      : state.kind === 'error'
        ? `Failed to open bundle: ${state.error}`
        : null;

  return (
    <div className="flex flex-col items-center justify-center h-full gap-3 text-center px-4">
      <BundleDropzone />
      {hint && (
        <p
          className={[
            'text-xs max-w-md break-words',
            state.kind === 'error' ? 'text-red-400' : 'text-muted',
          ].join(' ')}
        >
          {hint}
        </p>
      )}
      <p className="text-muted text-xs">
        Or produce one with <code className="bg-panel-2 px-1.5 rounded">prism export bundle</code>.
      </p>
    </div>
  );
}

'use client';

/* Shared `.prismbundle` loader used by both the landing-page BundleDropzone
 * (drag/drop + click-to-pick) and the Topbar's Load-bundle button (click-to-
 * pick only). One implementation, one set of edge cases — without this hook
 * the SHA + WASM-decode + IndexedDB-persist + URL-navigate dance would be
 * duplicated in two places and drift on every small change.
 *
 * Status is returned for the caller to render however it likes:
 *   - dropzone surfaces it inline under the drop target
 *   - topbar surfaces it as a destructive-tone tooltip + border
 * Decoding sets a busy ref so concurrent calls (rapid clicks, dropping two
 * files in the same tick) are dropped on the floor.
 */

import { useCallback, useRef, useState } from 'react';
import { wasm } from '@/lib/wasm';
import { putBundle } from '@/lib/bundleStore';

export type LoadStatus =
  | { kind: 'idle' }
  | { kind: 'decoding'; name: string }
  | { kind: 'error'; error: string };

async function sha256Hex(buf: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', buf));
  let hex = '';
  for (const b of digest) hex += b.toString(16).padStart(2, '0');
  return hex;
}

export function useLoadBundle() {
  const [status, setStatus] = useState<LoadStatus>({ kind: 'idle' });
  // Concurrent-drop / rapid-click guard. A ref (not state) so the check is
  // synchronous against the latest value — React batches setStatus, so two
  // gestures arriving in the same tick could otherwise race past a
  // state-based gate.
  const busyRef = useRef(false);

  const loadFile = useCallback(async (file: File) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setStatus({ kind: 'decoding', name: file.name });
    try {
      const buf = await file.arrayBuffer();
      const bytes = new Uint8Array(buf);
      const sha = await sha256Hex(buf);
      const m = await wasm();
      // Throws on corrupt input — we let the error bubble so we never
      // persist bytes the WASM rejected.
      JSON.parse(m.load_bundle(bytes));
      // Persist BEFORE the URL flip. If persistence fails (quota, storage
      // denied in some incognito modes), surface a clear message and do
      // NOT navigate: the URL-driven rehydrate would miss IndexedDB on
      // the next load and dump the user back at an empty drop zone with
      // no explanation.
      try {
        await putBundle(sha, bytes);
      } catch (e) {
        const raw = e instanceof Error ? e.message : String(e);
        const quotaLike =
          (e instanceof DOMException && e.name === 'QuotaExceededError') ||
          raw.toLowerCase().includes('quota');
        const friendly = quotaLike
          ? `Bundle decoded but the browser refused to store ${bytes.length.toLocaleString()} bytes — site storage quota exceeded. Free up space and try again.`
          : `Bundle decoded but could not be stored in this browser: ${raw}`;
        setStatus({ kind: 'error', error: friendly });
        return;
      }
      const params = new URLSearchParams(window.location.search);
      params.set('bundle', sha);
      // Full reload — WasmBundleLoader on the next page load reads the
      // sha from the URL and rehydrates the workspace from IndexedDB.
      window.location.search = params.toString();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setStatus({ kind: 'error', error: msg });
    } finally {
      busyRef.current = false;
    }
  }, []);

  return { status, loadFile };
}

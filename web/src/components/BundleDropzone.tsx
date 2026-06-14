'use client';

import { useCallback, useRef, useState } from 'react';
import { wasm } from '@/lib/wasm';
import { putBundle } from '@/lib/bundleStore';

type Status =
  | { kind: 'idle' }
  | { kind: 'decoding'; name: string }
  | { kind: 'error'; error: string };

/* Compute the SHA-256 of a byte array as a 64-char lowercase hex string,
 * using the WebCrypto SubtleCrypto API. Matches the `bundle_sha256` value
 * the WASM `load_bundle` returns, and is used as both the URL `?bundle=`
 * parameter and the IndexedDB key. */
async function sha256Hex(buf: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', buf));
  let hex = '';
  for (const b of digest) hex += b.toString(16).padStart(2, '0');
  return hex;
}

export default function BundleDropzone() {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [status, setStatus] = useState<Status>({ kind: 'idle' });
  const [dragOver, setDragOver] = useState(false);
  // Concurrent-drop guard. A ref (not state) so the check is synchronous
  // against the latest value — React batches setStatus, so two drops
  // arriving in the same tick could otherwise race past a state-based gate.
  const busyRef = useRef(false);

  const handleFile = useCallback(async (file: File) => {
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
      // Persist BEFORE the URL flip. If persistence fails (quota,
      // storage denied in some incognito modes), surface a clear
      // message and do NOT navigate: the URL-driven rehydrate would
      // miss IndexedDB on the next load and dump the user back at
      // an empty drop zone with no explanation.
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

  const onDrop = useCallback(
    (e: React.DragEvent<HTMLDivElement>) => {
      e.preventDefault();
      setDragOver(false);
      if (busyRef.current) return;
      const file = e.dataTransfer.files?.[0];
      if (file) void handleFile(file);
    },
    [handleFile],
  );

  const onDragOver = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setDragOver(true);
  }, []);

  const onDragLeave = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setDragOver(false);
  }, []);

  const onChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      if (busyRef.current) return;
      const file = e.target.files?.[0];
      if (file) void handleFile(file);
    },
    [handleFile],
  );

  const busy = status.kind === 'decoding';

  return (
    <div
      onDrop={onDrop}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      className={[
        'flex flex-col items-center justify-center gap-2 rounded-card border border-dashed px-8 py-7 transition-colors',
        'bg-panel border-line',
        dragOver ? 'border-accent bg-panel-2' : '',
      ].join(' ')}
    >
      {/* Document-with-down-arrow glyph: telegraphs "drop a file here". */}
      <svg
        className={`h-8 w-8 mb-1 transition-colors ${dragOver ? 'text-accent' : 'text-muted'}`}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
        <path d="M14 2v6h6" />
        <path d="M12 12v6" />
        <path d="M9 15l3 3 3-3" />
      </svg>
      <p className="text-ink text-sm">
        Drop a <code className="bg-panel-2 px-1.5 rounded-badge">.prismbundle</code> file here
      </p>
      <p className="text-muted text-xs">or</p>
      <label
        className={[
          'inline-flex items-center cursor-pointer rounded-control border border-line px-3 py-1.5 text-xs',
          'bg-panel-2 text-ink hover:border-ink transition-colors',
          busy ? 'opacity-60 cursor-wait' : '',
        ].join(' ')}
      >
        <input
          ref={inputRef}
          type="file"
          accept=".prismbundle,application/octet-stream"
          className="hidden"
          onChange={onChange}
          disabled={busy}
        />
        {busy ? `decoding ${status.name}…` : 'choose file'}
      </label>
      {status.kind === 'error' && (
        <p className="text-xs text-red-400 max-w-md text-center break-words">
          {status.error}
        </p>
      )}
    </div>
  );
}

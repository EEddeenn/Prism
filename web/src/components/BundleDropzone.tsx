'use client';

import { useCallback, useRef, useState } from 'react';
import { useLoadBundle } from '@/lib/useLoadBundle';

export default function BundleDropzone() {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [dragOver, setDragOver] = useState(false);
  const { status, loadFile } = useLoadBundle();

  const onDrop = useCallback(
    (e: React.DragEvent<HTMLDivElement>) => {
      e.preventDefault();
      setDragOver(false);
      const file = e.dataTransfer.files?.[0];
      if (file) void loadFile(file);
    },
    [loadFile],
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
      const file = e.target.files?.[0];
      if (file) void loadFile(file);
    },
    [loadFile],
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

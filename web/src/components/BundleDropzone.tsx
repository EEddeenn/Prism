'use client';

import { useCallback, useRef, useState } from 'react';
import { useLoadBundle } from '@/lib/useLoadBundle';
import { Callout, buttonClass } from '@/design-system';
import { cn } from '@/lib/cn';

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
      className={cn(
        'flex flex-col items-center justify-center gap-2 rounded-control border border-dashed px-8 py-7 transition-colors duration-150 ease-instrument',
        'bg-charcoal border-iron',
        dragOver && 'border-cyan-gray bg-model-black',
      )}
    >
      {/* Document-with-down-arrow glyph: telegraphs "drop a file here". */}
      <svg
        className={cn('h-8 w-8 mb-1 transition-colors duration-150', dragOver ? 'text-cyan-gray' : 'text-archive')}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.8}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
        <path d="M14 2v6h6" />
        <path d="M12 12v6" />
        <path d="M9 15l3 3 3-3" />
      </svg>
      <p className="text-bone text-data">
        Drop a <code className="font-mono bg-model-black px-1.5 py-px rounded-control">.prismbundle</code> file here
      </p>
      <p className="text-archive text-data-s">or</p>
      <label className={buttonClass('secondary', 'sm', cn('cursor-pointer', busy && 'opacity-60 cursor-wait'))}>
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
        <Callout tone="bad" role="alert" showWord className="mt-1 w-full max-w-md break-words">
          {status.error}
        </Callout>
      )}
    </div>
  );
}

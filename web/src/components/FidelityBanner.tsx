'use client';

import { Badge, Callout } from '@/components/ui';
import { useGraphStore } from '@/store/useGraphStore';

export default function FidelityBanner() {
  const runMeta = useGraphStore((s) => s.runMeta);

  if (!runMeta) return null;

  const caveats = runMeta.fidelity_caveats ?? [];
  const isExact = runMeta.fidelity_tier === 'lockfile-exact';

  if (isExact && caveats.length === 0) return null;

  return (
    <Callout tone="warn" className="mb-2.5">
      <div>
        Rankings are <b className="font-semibold">approximate</b> — resolution{' '}
        <span className="font-mono">{runMeta.fidelity_tier}</span>.
      </div>
      {caveats.length > 0 && (
        <div className="flex flex-wrap gap-1 mt-1">
          {caveats.map((c) => (
            <Badge key={c} tone="muted" mono>
              {c}
            </Badge>
          ))}
        </div>
      )}
    </Callout>
  );
}

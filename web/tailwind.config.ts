import type { Config } from 'tailwindcss';

/* The Prism palette — the same dark, technical theme as the original UI, lifted
 * into Tailwind tokens. Kept in sync with src/lib/palette.ts (which the 3D scene
 * and canvas use). */
const config: Config = {
  content: ['./src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        bg: '#0e1016',
        panel: { DEFAULT: '#151823', 2: '#1d212e', 3: '#252a3a' },
        ink: '#e7e9f0',
        muted: '#8b93a7',
        line: { DEFAULT: '#2a2f3c', 2: '#353c4d' },
        accent: { DEFAULT: '#5aa9ff', 2: '#ffb454', bd: '#2b4a6f' },
        // Text on an accent fill (e.g. the active tab badge), for contrast.
        onAccent: '#06203f',
        node: { DEFAULT: '#3b82f6', root: '#a78bfa', center: '#ffb454', fwd: '#34d399', impact: '#f87171', dup: '#f472b6' },
        edge: '#3a4150',
        // Semantic tones carry their soft surface (`bg`) and border (`bd`) so
        // callouts/badges stop hand-rolling hex pairs. `text-ok`/`bg-ok-bg`/etc.
        ok: { DEFAULT: '#34d399', bg: '#0f2419', bd: '#1d4a32' },
        warn: { DEFAULT: '#ffb454', bg: '#20180c', bd: '#4a3a1e' },
        bad: { DEFAULT: '#f87171', bg: '#2a1414', bd: '#5a2424' },
        gen: '#ffd166',
      },
      // One named type scale for the side rail; arbitrary text-[Npx] is retired.
      // (Prose keeps text-xs/text-sm.)
      fontSize: {
        micro: ['10px', '14px'],
        mini: ['11px', '15px'],
        label: ['11px', '14px'],
        body: ['12px', '17px'],
        head: ['13px', '18px'],
        display: ['20px', '24px'], // hero figures (e.g. the risk score)
      },
      fontFamily: {
        mono: ['ui-monospace', 'SFMono-Regular', 'Menlo', 'Consolas', 'monospace'],
      },
      boxShadow: {
        panel: '0 6px 22px rgba(0, 0, 0, 0.45)',
      },
      // Role-named radii — the class name describes what the surface DOES,
      // not how big the radius is. Prefer these over rounded-md / rounded-lg /
      // rounded-xl in all new code. The default Tailwind scale is left intact
      // so any straggler size-named class keeps working during migration.
      borderRadius: {
        tile: '3px',      // treemap swatches, sparkline columns, tiny data dots
        badge: '4px',     // inline code, kbd hints, meter bars, skeletons, layer labels
        control: '6px',   // buttons, inputs, segmented halves, list rows, icon buttons
        card: '12px',     // panels, modals, HUDs, callouts, dropzone, content cards
        pill: '9999px',   // true pills (HoverCard caps, dots, Badge, MetricPill)
      },
    },
  },
  plugins: [],
};

export default config;

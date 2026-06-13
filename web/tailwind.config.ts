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
        accent: { DEFAULT: '#5aa9ff', 2: '#ffb454' },
        node: { DEFAULT: '#3b82f6', root: '#a78bfa', center: '#ffb454', fwd: '#34d399', impact: '#f87171', dup: '#f472b6' },
        edge: '#3a4150',
        ok: '#34d399',
        warn: '#ffb454',
        bad: '#f87171',
        gen: '#ffd166',
      },
      fontFamily: {
        mono: ['ui-monospace', 'SFMono-Regular', 'Menlo', 'Consolas', 'monospace'],
      },
      boxShadow: {
        panel: '0 6px 22px rgba(0, 0, 0, 0.45)',
      },
    },
  },
  plugins: [],
};

export default config;

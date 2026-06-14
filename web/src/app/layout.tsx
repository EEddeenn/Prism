import type { Metadata } from 'next';
import { Inter, IBM_Plex_Mono } from 'next/font/google';
import './globals.css';

/* Inter = UI sans; IBM Plex Mono = all data, IDs, numbers, metadata labels.
 * Self-hosted at build time (works with `output: export`), exposed as CSS vars
 * that globals.css `@theme` maps to --font-sans / --font-mono. */
const sans = Inter({
  subsets: ['latin'],
  variable: '--font-inter',
  display: 'swap',
});
const mono = IBM_Plex_Mono({
  subsets: ['latin'],
  weight: ['400', '500', '600'],
  variable: '--font-plex-mono',
  display: 'swap',
});

export const metadata: Metadata = {
  title: 'Prism — Dependency Topology',
  description: 'Interactive 2D + 3D dependency-topology explorer over the Prism analysis store.',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${sans.variable} ${mono.variable}`}>
      <body>
        <a href="#main" className="skip-link">
          Skip to content
        </a>
        {children}
      </body>
    </html>
  );
}

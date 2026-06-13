import type { Metadata } from 'next';
import './globals.css';

export const metadata: Metadata = {
  title: 'Prism — Dependency Topology',
  description: 'Interactive 2D + 3D dependency-topology explorer over the Prism analysis store.',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}

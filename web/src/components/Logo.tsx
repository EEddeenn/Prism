'use client';

/**
 * The Prism mark.
 *
 * A triangle (the literal prism — the optics metaphor that gives the
 * project its name) with three vertex nodes and a center node connected
 * by thin spokes — the smallest non-trivial dependency graph: three
 * packages sharing a common dep. The vertex circles carry a faint three-
 * tone tint (sky / pink / gold) so the mark reads as both spectrum
 * (light through a prism) and topology (the H₀ / H₁ / H₂ dimensions the
 * TDA views surface). The outline, inner edges, and center node use
 * `currentColor`, so the mark adapts to the surrounding text color and
 * scales to any size.
 */
export default function Logo({
  size = 56,
  className,
}: {
  size?: number;
  className?: string;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 48 48"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      role="img"
      aria-label="Prism"
      className={className}
    >
      {/* prism outline */}
      <path
        d="M24 5 L43.5 38 L4.5 38 Z"
        stroke="currentColor"
        strokeWidth={2.25}
        strokeLinejoin="round"
      />
      {/* internal graph — centroid spokes to each vertex */}
      <line x1="24" y1="8" x2="24" y2="27" stroke="currentColor" strokeWidth={1.5} opacity={0.55} />
      <line x1="40.9" y1="36.5" x2="24" y2="27" stroke="currentColor" strokeWidth={1.5} opacity={0.55} />
      <line x1="7.1" y1="36.5" x2="24" y2="27" stroke="currentColor" strokeWidth={1.5} opacity={0.55} />
      {/* spectrum vertex nodes — sky / pink / gold */}
      <circle cx="24" cy="5" r="3" fill="#60a5fa" />
      <circle cx="43.5" cy="38" r="3" fill="#f472b6" />
      <circle cx="4.5" cy="38" r="3" fill="#facc15" />
      {/* center node */}
      <circle cx="24" cy="27" r="2.25" fill="currentColor" opacity={0.85} />
    </svg>
  );
}

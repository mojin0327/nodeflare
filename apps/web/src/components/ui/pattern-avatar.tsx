import { ReactNode } from 'react';

// FNV-1a hash → unsigned 32-bit int. Deterministic so each seed keeps a
// stable pattern across renders and sessions.
function hashSeed(seed: string): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/**
 * A small geometric (Bauhaus-style) avatar generated deterministically from a
 * seed. Replaces flat/gradient color fills — every seed gets its own layout of
 * shapes drawn from a single hue, so an overlaid label stays legible.
 *
 * Pass sizing/font via `className` (e.g. "h-8 w-8 text-sm font-semibold");
 * the label inherits font-size from it.
 */
export function PatternAvatar({
  seed,
  label,
  rounded = 'rounded-full',
  className = '',
  children,
}: {
  seed: string;
  label?: ReactNode;
  rounded?: string;
  className?: string;
  children?: ReactNode;
}) {
  const h = hashSeed(seed || 'x');
  const bit = (shift: number, mod: number) => (h >>> shift) % mod;

  const base = h % 360;
  const bg = `hsl(${base} 58% 52%)`;
  const shade = `hsl(${base} 60% 40%)`; // darker, same hue
  const tint = `hsl(${(base + 16) % 360} 62% 63%)`; // lighter neighbour
  const accent = `hsl(${(base + 210) % 360} 55% 58%)`; // complementary accent

  const rectRot = 15 + bit(3, 60);
  const cx = 18 + bit(6, 44);
  const cy = 16 + bit(9, 48);
  const barRot = bit(12, 4) * 90;

  return (
    <span
      className={`relative inline-flex shrink-0 items-center justify-center overflow-hidden ${rounded} ${className}`}
    >
      <svg
        viewBox="0 0 80 80"
        className="absolute inset-0 h-full w-full"
        preserveAspectRatio="xMidYMid slice"
        aria-hidden="true"
      >
        <rect width="80" height="80" fill={bg} />
        <rect x="-20" y="34" width="120" height="120" fill={shade} transform={`rotate(${rectRot} 40 40)`} />
        <circle cx={cx} cy={cy} r="24" fill={tint} opacity="0.9" />
        <rect x="46" y="-20" width="20" height="120" fill={accent} opacity="0.75" transform={`rotate(${barRot} 40 40)`} />
      </svg>
      {label != null && (
        <span className="relative z-10 leading-none text-white drop-shadow-[0_1px_2px_rgba(0,0,0,0.45)]">
          {label}
        </span>
      )}
      {children}
    </span>
  );
}

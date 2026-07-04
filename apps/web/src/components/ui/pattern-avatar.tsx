import { ReactNode } from 'react';

// FNV-1a hash → unsigned 32-bit seed.
function hashSeed(seed: string): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

// mulberry32 PRNG — a small procedural stream of [0,1) values from one seed.
function mulberry32(a: number): () => number {
  return function () {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const COLS = 5;
const ROWS = 5;
const HALF = 3; // left half + centre column; mirrored for symmetry

/**
 * A procedural, seamless pixel-grid identicon generated deterministically from
 * a seed. The 5×5 lattice is left/right mirrored for a tessellated look, and
 * every cell is drawn in one of two shades of a single seed-derived hue on a
 * light tint of the same hue.
 *
 * Pass sizing/shape via `className` / `rounded`. No label — the pattern itself
 * is the identifier.
 */
export function PatternAvatar({
  seed,
  rounded = 'rounded-full',
  className = '',
  children,
}: {
  seed: string;
  rounded?: string;
  className?: string;
  children?: ReactNode;
}) {
  const rand = mulberry32(hashSeed(seed || 'x'));
  const hue = Math.floor(rand() * 360);
  const primary = `hsl(${hue} 62% 55%)`;
  const secondary = `hsl(${(hue + 28) % 360} 68% 64%)`;
  const bg = `hsl(${hue} 42% 95%)`;

  const cells: number[][] = [];
  for (let y = 0; y < ROWS; y++) {
    const row: number[] = [];
    for (let x = 0; x < HALF; x++) {
      const r = rand();
      row[x] = r < 0.45 ? 0 : r < 0.82 ? 1 : 2; // empty / primary / secondary
    }
    for (let x = HALF; x < COLS; x++) row[x] = row[COLS - 1 - x];
    cells.push(row);
  }

  return (
    <span className={`relative inline-flex shrink-0 overflow-hidden ${rounded} ${className}`}>
      <svg viewBox="0 0 5 5" className="h-full w-full" aria-hidden="true">
        <rect width="5" height="5" fill={bg} />
        {cells.flatMap((row, y) =>
          row.map((v, x) =>
            v === 0 ? null : (
              <rect
                key={`${x}-${y}`}
                x={x + 0.06}
                y={y + 0.06}
                width={0.88}
                height={0.88}
                rx={0.18}
                fill={v === 1 ? primary : secondary}
              />
            )
          )
        )}
      </svg>
      {children}
    </span>
  );
}

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

type Variant = 'grid' | 'geo';

// 5×5 mirrored pixel-grid identicon.
function renderGrid(rand: () => number, primary: string, secondary: string) {
  const COLS = 5;
  const ROWS = 5;
  const HALF = 3; // left half + centre column; mirrored for symmetry
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
  return cells.flatMap((row, y) =>
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
  );
}

// Triangle tessellation — each cell is split by a diagonal into two triangles,
// giving a denser, more overtly geometric pattern.
function renderGeo(rand: () => number, palette: string[]) {
  const N = 4;
  const s = 5 / N;
  const tris: ReactNode[] = [];
  for (let y = 0; y < N; y++) {
    for (let x = 0; x < N; x++) {
      const x0 = x * s;
      const y0 = y * s;
      const x1 = x0 + s;
      const y1 = y0 + s;
      const flip = rand() < 0.5;
      const c1 = palette[Math.floor(rand() * palette.length)];
      const c2 = palette[Math.floor(rand() * palette.length)];
      const [p1, p2] = flip
        ? [`${x0},${y0} ${x1},${y0} ${x1},${y1}`, `${x0},${y0} ${x0},${y1} ${x1},${y1}`]
        : [`${x0},${y0} ${x1},${y0} ${x0},${y1}`, `${x1},${y0} ${x1},${y1} ${x0},${y1}`];
      tris.push(<polygon key={`${x}-${y}-a`} points={p1} fill={c1} />);
      tris.push(<polygon key={`${x}-${y}-b`} points={p2} fill={c2} />);
    }
  }
  return tris;
}

/**
 * A procedural avatar generated deterministically from a seed.
 *
 * - `grid` (default): a 5×5 mirrored pixel-grid identicon.
 * - `geo`: a triangle tessellation for a more overtly geometric look.
 *
 * Colors come from a single seed-derived hue. Pass sizing/shape via
 * `className` / `rounded`. No label — the pattern itself is the identifier.
 */
export function PatternAvatar({
  seed,
  variant = 'grid',
  rounded = 'rounded-full',
  className = '',
  children,
}: {
  seed: string;
  variant?: Variant;
  rounded?: string;
  className?: string;
  children?: ReactNode;
}) {
  const rand = mulberry32(hashSeed(seed || 'x'));
  const hue = Math.floor(rand() * 360);
  const primary = `hsl(${hue} 66% 44%)`;
  const secondary = `hsl(${(hue + 28) % 360} 70% 54%)`;
  const dark = `hsl(${hue} 62% 32%)`;
  const tint = `hsl(${hue} 46% 82%)`;
  const bg = `hsl(${hue} 44% 92%)`;

  return (
    <span className={`relative inline-flex shrink-0 overflow-hidden ${rounded} ${className}`}>
      <svg viewBox="0 0 5 5" className="h-full w-full" aria-hidden="true">
        <rect width="5" height="5" fill={bg} />
        {variant === 'geo'
          ? renderGeo(rand, [primary, secondary, dark, tint])
          : renderGrid(rand, primary, secondary)}
      </svg>
      {children}
    </span>
  );
}

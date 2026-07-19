import { ImageResponse } from 'next/og';
import { SiNodedotjs, SiPython, SiGo, SiRust, SiDocker } from 'react-icons/si';

export const alt = 'MCP Server Template';
export const size = { width: 1200, height: 630 };
export const contentType = 'image/png';

const SITE_URL = 'https://nodeflare.tech';

const RUNTIME_LABELS: Record<string, string> = {
  node: 'Node.js', python: 'Python', go: 'Go', rust: 'Rust', docker: 'Docker',
};
const RUNTIME_COLORS: Record<string, string> = {
  node: '#16a34a', python: '#2563eb', go: '#0891b2', rust: '#ea580c', docker: '#0284c7',
};

function fmt(n: number) {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}
function titleSize(name: string) {
  if (name.length <= 14) return 80;
  if (name.length <= 22) return 68;
  if (name.length <= 30) return 56;
  return 46;
}

// FNV-1a hash — same as jazz-avatar.tsx
function fnv1a(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

// Mersenne Twister — same algorithm react-jazzicon uses internally
function makeMT(seed: number) {
  const N = 624, M = 397;
  const mt = new Array<number>(N);
  mt[0] = seed >>> 0;
  for (let i = 1; i < N; i++) {
    mt[i] = (Math.imul(1812433253, mt[i - 1] ^ (mt[i - 1] >>> 30)) + i) >>> 0;
  }
  let idx = N;
  return (): number => {
    if (idx >= N) {
      for (let k = 0; k < N; k++) {
        const y = (mt[k] & 0x80000000) | (mt[(k + 1) % N] & 0x7fffffff);
        mt[k] = mt[(k + M) % N] ^ (y >>> 1) ^ (y & 1 ? 0x9908b0df : 0);
      }
      idx = 0;
    }
    let y = mt[idx++];
    y ^= y >>> 11;
    y ^= (y << 7) & 0x9d2c5680;
    y ^= (y << 15) & 0xefc60000;
    y ^= y >>> 18;
    return (y >>> 0) / 0x100000000;
  };
}

// Jazzicon color palette (same as react-jazzicon)
const JAZZ_COLORS = ['#01888C', '#FC7500', '#034F5D', '#F73F01', '#FC1960', '#C7144C', '#F3C100', '#1598F2', '#2465E1', '#F19E02'];

function jazziconShapes(seed: number, d: number) {
  const rand = makeMT(seed);
  // hueShift: rotate palette by random amount (jazzicon's colorRotate)
  const colors = JAZZ_COLORS.slice();
  const shift = Math.floor(rand() * colors.length);
  for (let i = 0; i < shift; i++) colors.push(colors.shift()!);
  const bg = colors.shift()!;
  const w = d * 1.92;
  const shapes = Array.from({ length: 3 }, (_, i) => ({
    color: colors[i % colors.length],
    tx: d / 2 + (rand() - 0.5) * d,
    ty: d / 2 + (rand() - 0.5) * d,
    rot: (rand() * 360).toFixed(1),
  }));
  return { bg, w, shapes };
}

function RuntimeIcon({ runtime }: { runtime: string }) {
  const color = RUNTIME_COLORS[runtime] ?? '#6b7280';
  const s = { width: 28, height: 28, color, flexShrink: 0 } as React.CSSProperties;
  switch (runtime) {
    case 'node':   return <SiNodedotjs style={s} />;
    case 'python': return <SiPython style={s} />;
    case 'go':     return <SiGo style={s} />;
    case 'rust':   return <SiRust style={s} />;
    case 'docker': return <SiDocker style={s} />;
    default:       return null;
  }
}

async function loadInterFont(weight: number): Promise<ArrayBuffer | null> {
  try {
    const res = await fetch(
      `https://cdn.jsdelivr.net/npm/@fontsource/inter@5.0.0/files/inter-latin-${weight}-normal.woff2`,
      { next: { revalidate: 86400 } }
    );
    if (!res.ok) return null;
    return res.arrayBuffer();
  } catch {
    return null;
  }
}

export default async function Image({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;

  const apiBase = process.env.NEXT_PUBLIC_API_URL
    ? `${process.env.NEXT_PUBLIC_API_URL}/api/v1`
    : `${SITE_URL}/api/v1`;

  const [tmpl, font500, font800] = await Promise.all([
    fetch(`${apiBase}/templates/${id}`, { next: { revalidate: 3600 } })
      .then(r => r.ok ? r.json() : null)
      .catch(() => null),
    loadInterFont(500),
    loadInterFont(800),
  ]);

  const name       = tmpl?.name        ?? 'MCP Server Template';
  const desc       = tmpl?.description ?? '';
  const runtime    = tmpl?.runtime     ?? 'node';
  const useCount   = tmpl?.use_count   ?? 0;
  const iconUrl    = tmpl?.icon_url    ?? null;
  const githubRepo = tmpl?.github_repo ?? null;

  const label     = RUNTIME_LABELS[runtime] ?? runtime;
  const shortDesc = desc.length > 90 ? desc.slice(0, 90) + '…' : desc;
  const fs        = titleSize(name);
  const jazz      = jazziconShapes(fnv1a(id), 140);

  const fonts = [
    ...(font500 ? [{ name: 'Inter', data: font500, weight: 500 as const, style: 'normal' as const }] : []),
    ...(font800 ? [{ name: 'Inter', data: font800, weight: 800 as const, style: 'normal' as const }] : []),
  ];

  return new ImageResponse(
    (
      <div style={{
        width: 1200, height: 630, position: 'relative', display: 'flex',
        background: 'linear-gradient(140deg, #f8f6ff 0%, #ede9fe 60%, #faf5ff 100%)',
        overflow: 'hidden', fontFamily: 'Inter, sans-serif',
      }}>
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={`${SITE_URL}/sign.png`} alt="" style={{ position: 'absolute', top: 0, left: 0, width: '100%', height: '100%', objectFit: 'cover', opacity: 0.12 }} />
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={`${SITE_URL}/c1.png`} alt="" style={{ position: 'absolute', left: -40, bottom: 0, height: 260, width: 'auto', opacity: 0.55 }} />
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={`${SITE_URL}/c2.png`} alt="" style={{ position: 'absolute', right: -40, bottom: 0, height: 260, width: 'auto', opacity: 0.55 }} />

        <div style={{
          position: 'absolute', top: 0, left: 0, right: 0, bottom: 0,
          display: 'flex', flexDirection: 'row',
          padding: '68px 88px 60px 88px',
        }}>
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'space-between', paddingRight: 56 }}>
            <div style={{ display: 'flex', flexDirection: 'column' }}>
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img src={`${SITE_URL}/logo2.png`} alt="Nodeflare" style={{ height: 44, marginBottom: 22, opacity: 0.8 }} />

              <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 22, marginBottom: 16 }}>
                <RuntimeIcon runtime={runtime} />
                <span style={{ color: '#374151', fontWeight: 600 }}>{label}</span>
                <span style={{ color: '#d1d5db' }}>·</span>
                <span style={{ color: '#9ca3af', fontWeight: 500 }}>nodeflare.tech</span>
              </div>

              <div style={{ fontSize: fs, fontWeight: 800, color: '#333333', lineHeight: 1.1, letterSpacing: '-0.03em', marginBottom: shortDesc ? 20 : 0 }}>
                {name}
              </div>

              {shortDesc && (
                <div style={{ fontSize: 28, fontWeight: 500, color: '#374151', lineHeight: 1.55 }}>{shortDesc}</div>
              )}
            </div>

            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 32, borderTop: '1px solid rgba(0,0,0,0.18)', paddingTop: 26 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, fontSize: 30, color: '#374151', fontWeight: 500 }}>
                <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="#6b7280" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ flexShrink: 0 }}>
                  <path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z" />
                  <path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z" />
                  <path d="M9 12H4s.55-3.03 2-4c1.62-1.08 5 0 5 0" />
                  <path d="M12 15v5s3.03-.55 4-2c1.08-1.62 0-5 0-5" />
                </svg>
                <span style={{ fontWeight: 800, color: '#333333' }}>{fmt(useCount)}</span>
                <span>deploys</span>
              </div>
              {githubRepo && (
                <>
                  <span style={{ color: '#e5e7eb', fontSize: 20 }}>·</span>
                  <div style={{ fontSize: 22, color: '#6b7280', fontWeight: 500 }}>{githubRepo}</div>
                </>
              )}
            </div>
          </div>

          {/* Right: server icon */}
          <div style={{ width: 160, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
            {iconUrl ? (
              // eslint-disable-next-line @next/next/no-img-element
              <img src={iconUrl} alt="" style={{ width: 140, height: 140, borderRadius: 32, objectFit: 'cover' }} />
            ) : (
              // Jazzicon rendered as inline SVG — same algorithm as react-jazzicon / JazzAvatar
              <div style={{ width: 140, height: 140, borderRadius: 32, overflow: 'hidden', display: 'flex' }}>
                <svg width="140" height="140" viewBox="0 0 140 140">
                  <rect width="140" height="140" fill={jazz.bg} />
                  {jazz.shapes.map((s, i) => (
                    <rect
                      key={i}
                      x={-jazz.w / 2}
                      y={-jazz.w / 2}
                      width={jazz.w}
                      height={jazz.w}
                      transform={`translate(${s.tx},${s.ty}) rotate(${s.rot})`}
                      fill={s.color}
                    />
                  ))}
                </svg>
              </div>
            )}
          </div>
        </div>
      </div>
    ),
    { ...size, ...(fonts.length > 0 ? { fonts } : {}) },
  );
}

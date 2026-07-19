import { ImageResponse } from 'next/og';
import { SiNodedotjs, SiPython, SiGo, SiRust, SiDocker } from 'react-icons/si';

// Node.js runtime (not edge) — needed for react-icons and proper font loading
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

// FNV-1a hash → deterministic gradient (same algorithm as JazzAvatar in dashboard)
function idToGradient(id: string) {
  let h = 2166136261;
  for (let i = 0; i < id.length; i++) {
    h = (Math.imul(h ^ id.charCodeAt(i), 16777619)) >>> 0;
  }
  const hue1 = h % 360;
  const hue2 = (hue1 + 50 + (h >> 8) % 70) % 360;
  return {
    bg1: `hsl(${hue1}, 72%, 58%)`,
    bg2: `hsl(${hue2}, 76%, 42%)`,
  };
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
    const css = await fetch(
      `https://fonts.googleapis.com/css2?family=Inter:wght@${weight}&display=swap`,
      { headers: { 'User-Agent': 'Mozilla/5.0 (compatible; Googlebot/2.1)' }, next: { revalidate: 86400 } }
    ).then(r => r.text());
    const url = css.match(/src: url\((.+?)\) format\('woff2'\)/)?.[1];
    if (!url) return null;
    return fetch(url, { next: { revalidate: 86400 } }).then(r => r.arrayBuffer());
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
  const gradient  = idToGradient(id);

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
          {/* Left: text */}
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
              <div style={{
                width: 140, height: 140, borderRadius: 32,
                background: `linear-gradient(135deg, ${gradient.bg1}, ${gradient.bg2})`,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <span style={{ color: 'rgba(255,255,255,0.92)', fontSize: 64, fontWeight: 800, letterSpacing: '-0.05em' }}>
                  {name[0].toUpperCase()}
                </span>
              </div>
            )}
          </div>
        </div>
      </div>
    ),
    { ...size, ...(fonts.length > 0 ? { fonts } : {}) },
  );
}

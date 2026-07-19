import { ImageResponse } from 'next/og';

export const runtime = 'edge';
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

export default async function Image({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;

  const apiBase = process.env.NEXT_PUBLIC_API_URL
    ? `${process.env.NEXT_PUBLIC_API_URL}/api/v1`
    : `${SITE_URL}/api/v1`;

  let tmpl: any = null;
  try {
    const r = await fetch(`${apiBase}/templates/${id}`, { next: { revalidate: 3600 } });
    if (r.ok) tmpl = await r.json();
  } catch {}

  const name       = tmpl?.name        ?? 'MCP Server Template';
  const desc       = tmpl?.description ?? '';
  const runtime    = tmpl?.runtime     ?? 'node';
  const useCount   = tmpl?.use_count   ?? 0;
  const iconUrl    = tmpl?.icon_url    ?? null;
  const githubRepo = tmpl?.github_repo ?? null;

  const label    = RUNTIME_LABELS[runtime] ?? runtime;
  const color    = RUNTIME_COLORS[runtime] ?? '#7c3aed';
  const shortDesc = desc.length > 90 ? desc.slice(0, 90) + '…' : desc;
  const fs        = titleSize(name);

  return new ImageResponse(
    (
      <div
        style={{
          width: 1200, height: 630, position: 'relative', display: 'flex',
          background: 'linear-gradient(140deg, #f8f6ff 0%, #ede9fe 60%, #faf5ff 100%)',
          overflow: 'hidden', fontFamily: 'sans-serif',
        }}
      >
        {/* sign.png background */}
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={`${SITE_URL}/sign.png`} alt="" style={{ position: 'absolute', top: 0, left: 0, width: '100%', height: '100%', objectFit: 'cover', opacity: 0.12 }} />

        {/* c1.png — left character */}
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={`${SITE_URL}/c1.png`} alt="" style={{ position: 'absolute', left: -40, bottom: 0, height: 260, width: 'auto', opacity: 0.55 }} />

        {/* c2.png — right character */}
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={`${SITE_URL}/c2.png`} alt="" style={{ position: 'absolute', right: -40, bottom: 0, height: 260, width: 'auto', opacity: 0.55 }} />

        {/* Content */}
        <div
          style={{
            position: 'absolute', top: 0, left: 0, right: 0, bottom: 0,
            display: 'flex', flexDirection: 'row',
            padding: '68px 88px 60px 88px',
          }}
        >
          {/* Left: text */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', justifyContent: 'space-between', paddingRight: 56 }}>
            <div style={{ display: 'flex', flexDirection: 'column' }}>
              {/* Logo */}
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img src={`${SITE_URL}/logo2.png`} alt="Nodeflare" style={{ height: 44, marginBottom: 22, opacity: 0.8 }} />

              {/* Runtime icon box + label + site */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 22, marginBottom: 16 }}>
                <div style={{ width: 28, height: 28, borderRadius: 7, background: color, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
                  <span style={{ color: '#ffffff', fontSize: 15, fontWeight: 800, lineHeight: 1 }}>{label[0]}</span>
                </div>
                <span style={{ color: '#374151', fontWeight: 600 }}>{label}</span>
                <span style={{ color: '#d1d5db' }}>·</span>
                <span style={{ color: '#9ca3af', fontWeight: 500 }}>nodeflare.tech</span>
              </div>

              {/* Title */}
              <div style={{ fontSize: fs, fontWeight: 800, color: '#333333', lineHeight: 1.1, letterSpacing: '-0.03em', marginBottom: shortDesc ? 20 : 0 }}>
                {name}
              </div>

              {/* Description */}
              {shortDesc && (
                <div style={{ fontSize: 28, fontWeight: 500, color: '#374151', lineHeight: 1.55 }}>{shortDesc}</div>
              )}
            </div>

            {/* Bottom stats */}
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 32, borderTop: '1px solid rgba(0,0,0,0.18)', paddingTop: 26 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, fontSize: 30, color: '#374151', fontWeight: 500 }}>
                {/* Rocket icon (lucide path) */}
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
              <div style={{ width: 140, height: 140, borderRadius: 32, background: `${color}22`, border: `3px solid ${color}44`, display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 64, fontWeight: 800, color, letterSpacing: '-0.05em' }}>
                {label[0]}
              </div>
            )}
          </div>
        </div>
      </div>
    ),
    { ...size },
  );
}

import type { Metadata } from 'next';
import type { ReactNode } from 'react';

const SITE_URL = 'https://nodeflare.tech';

const RUNTIME_LABELS: Record<string, string> = {
  node: 'Node.js',
  python: 'Python',
  go: 'Go',
  rust: 'Rust',
  docker: 'Docker',
};

async function fetchTemplate(id: string) {
  const apiBase = process.env.NEXT_PUBLIC_API_URL
    ? `${process.env.NEXT_PUBLIC_API_URL}/api/v1`
    : `${SITE_URL}/api/v1`;

  try {
    const res = await fetch(`${apiBase}/templates/${id}`, {
      next: { revalidate: 3600 },
    });
    if (!res.ok) return null;
    return res.json();
  } catch {
    return null;
  }
}

export async function generateMetadata({
  params,
}: {
  params: Promise<{ id: string }>;
}): Promise<Metadata> {
  const { id } = await params;
  const tmpl = await fetchTemplate(id);

  if (!tmpl) {
    return {
      title: 'MCP Server Template | Nodeflare',
    };
  }

  const runtimeLabel = RUNTIME_LABELS[tmpl.runtime] ?? tmpl.runtime;
  const title = `${tmpl.name} | Nodeflare`;
  const description =
    tmpl.description ??
    `Deploy ${tmpl.name} (${runtimeLabel}) MCP server in seconds with Nodeflare.`;

  const imageUrl = `${SITE_URL}/explore/${id}/opengraph-image`;

  return {
    title,
    description,
    openGraph: {
      title,
      description,
      url: `${SITE_URL}/explore/${id}`,
      siteName: 'Nodeflare',
      type: 'website',
      images: [{ url: imageUrl, width: 1200, height: 630, alt: title }],
    },
    twitter: {
      card: 'summary_large_image',
      title,
      description,
      images: [imageUrl],
    },
  };
}

export default function TemplateDetailLayout({ children }: { children: ReactNode }) {
  return <>{children}</>;
}

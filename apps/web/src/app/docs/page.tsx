'use client';

import { useState, useEffect, useRef } from 'react';
import { useTranslations } from 'next-intl';
import { Header, Footer } from '@/components/layout';
import Link from 'next/link';

// Technical documentation for NodeFlare's internals (builder / adapter / proxy /
// hosting / runner). Prose is localized via the `docs` i18n namespace; code blocks
// and the architecture diagram are language-neutral and live inline here.
// Design is intentionally monochrome — no accent colors.

type SectionKey =
  | 'overview'
  | 'architecture'
  | 'builder'
  | 'adapter'
  | 'proxy'
  | 'tokens'
  | 'code'
  | 'hosting'
  | 'security';

const NAV: { id: string; key: SectionKey }[] = [
  { id: 'overview', key: 'overview' },
  { id: 'architecture', key: 'architecture' },
  { id: 'builder', key: 'builder' },
  { id: 'adapter', key: 'adapter' },
  { id: 'proxy', key: 'proxy' },
  { id: 'tokens', key: 'tokens' },
  { id: 'code', key: 'code' },
  { id: 'hosting', key: 'hosting' },
  { id: 'security', key: 'security' },
];

function Code({ children }: { children: string }) {
  return (
    <pre className="my-5 overflow-x-auto rounded-lg border border-gray-300 bg-gray-900 p-4 text-[13px] leading-relaxed text-gray-100">
      <code className="font-mono whitespace-pre">{children}</code>
    </pre>
  );
}

function Diagram({ children }: { children: string }) {
  return (
    <pre className="my-6 overflow-x-auto rounded-lg border border-gray-300 bg-gray-50 p-4 text-[12px] leading-snug text-gray-800">
      <code className="font-mono whitespace-pre">{children}</code>
    </pre>
  );
}

/** Benefit callout — answers "so what?" for a section. */
function Why({ text }: { text: string }) {
  const t = useTranslations('docs');
  return (
    <div className="my-6 border-l-2 border-gray-900 bg-gray-50 pl-4 py-3">
      <p className="text-xs font-semibold uppercase tracking-wider text-gray-500 mb-1">
        {t('whyLabel')}
      </p>
      <p className="text-sm text-gray-800 leading-relaxed">{text}</p>
    </div>
  );
}

function Section({
  id,
  children,
  title,
}: {
  id: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section id={id} className="scroll-mt-24 mb-16 pt-8 border-t first:border-t-0 first:pt-0">
      <h2 className="text-2xl font-bold text-gray-900 mb-4">{title}</h2>
      {children}
    </section>
  );
}

/** Paragraphs pulled from an i18n string array. */
function Paras({ items }: { items: string[] }) {
  return (
    <>
      {items.map((p, i) => (
        <p key={i} className="text-gray-600 mb-4 leading-relaxed">
          {p}
        </p>
      ))}
    </>
  );
}

/** term/desc cards used for capability grids. */
function FeatureGrid({ items }: { items: { t: string; d: string }[] }) {
  return (
    <div className="grid gap-4 sm:grid-cols-2 my-6">
      {items.map((f, i) => (
        <div key={i} className="rounded-lg border border-gray-200 p-4">
          <p className="font-semibold text-gray-900 mb-1">{f.t}</p>
          <p className="text-sm text-gray-600 leading-relaxed">{f.d}</p>
        </div>
      ))}
    </div>
  );
}

/** Numbered steps used for the build pipeline & code-mode flow. */
function Steps({ items }: { items: { t: string; d: string }[] }) {
  return (
    <ol className="my-6 space-y-4">
      {items.map((s, i) => (
        <li key={i} className="flex gap-4">
          <span className="flex-shrink-0 w-7 h-7 rounded-full bg-gray-900 text-white text-sm font-bold flex items-center justify-center">
            {i + 1}
          </span>
          <div className="min-w-0">
            <p className="font-semibold text-gray-900">{s.t}</p>
            <p className="text-sm text-gray-600 leading-relaxed">{s.d}</p>
          </div>
        </li>
      ))}
    </ol>
  );
}

export default function DocsPage() {
  const t = useTranslations('docs');
  const [activeSection, setActiveSection] = useState<string>('overview');
  const isScrollingRef = useRef(false);

  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        if (isScrollingRef.current) return;
        const visible = entries.filter((e) => e.isIntersecting).map((e) => e.target.id);
        if (visible.length > 0) {
          const order = NAV.map((s) => s.id);
          const top = visible.sort((a, b) => order.indexOf(a) - order.indexOf(b))[0];
          setActiveSection(top);
        }
      },
      { rootMargin: '-80px 0px -60% 0px', threshold: 0 }
    );
    NAV.forEach(({ id }) => {
      const el = document.getElementById(id);
      if (el) observer.observe(el);
    });
    return () => observer.disconnect();
  }, []);

  const scrollToSection = (id: string) => {
    setActiveSection(id);
    isScrollingRef.current = true;
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    setTimeout(() => {
      isScrollingRef.current = false;
    }, 1000);
  };

  const raw = <T,>(key: string, fallback: T): T => {
    const v = t.raw(key) as T | undefined;
    return v ?? fallback;
  };

  return (
    <div className="min-h-screen bg-white">
      <Header />

      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-12">
        <div className="flex gap-12">
          {/* Sidebar */}
          <aside className="hidden lg:block w-64 flex-shrink-0">
            <nav className="sticky top-24 space-y-1">
              <p className="text-xs font-semibold text-gray-400 uppercase tracking-wider mb-4">
                {t('sidebar')}
              </p>
              {NAV.map(({ id, key }) => (
                <button
                  key={id}
                  onClick={() => scrollToSection(id)}
                  className={`block w-full text-left px-3 py-2 text-sm border-l-2 transition-colors ${
                    activeSection === id
                      ? 'border-gray-900 text-gray-900 font-medium'
                      : 'border-transparent text-gray-400 hover:text-gray-700'
                  }`}
                >
                  {t(`nav.${key}`)}
                </button>
              ))}
            </nav>
          </aside>

          {/* Main Content */}
          <main className="flex-1 min-w-0">
            <div className="mb-12">
              <h1 className="text-4xl font-black text-gray-900 mb-4">{t('title')}</h1>
              <p className="text-base text-gray-600">{t('subtitle')}</p>
            </div>

            {/* Overview */}
            <Section id="overview" title={t('sec.overview.title')}>
              <Paras items={raw<string[]>('sec.overview.p', [])} />
              <div className="flex flex-wrap gap-2 mt-6">
                {raw<string[]>('sec.overview.badges', []).map((b, i) => (
                  <span
                    key={i}
                    className="inline-flex items-center rounded-full border border-gray-300 bg-gray-100 px-3 py-1 text-sm font-medium text-gray-700"
                  >
                    {b}
                  </span>
                ))}
              </div>
            </Section>

            {/* Architecture */}
            <Section id="architecture" title={t('sec.architecture.title')}>
              <Paras items={raw<string[]>('sec.architecture.p', [])} />
              <Diagram>{`   MCP client  (Claude Desktop / Cursor / your agent)
        │   Streamable HTTP · legacy SSE   (JSON-RPC 2.0)
        ▼
 ┌──────────────────────────────────────────────┐
 │  PROXY  (Rust)                                │
 │  API key / OAuth · per-request scope check    │
 │  caller-scoped cache · session affinity ·     │
 │  rate limit + monthly quota · tool transforms │
 └───────────────────────┬──────────────────────┘
        │   POST {endpoint}/mcp   (pinned to one Fly machine)
        ▼
 ┌──────────────────────────────────────────────┐
 │  stdio-adapter.cjs  (Node, injected at build) │
 │  stdin/stdout  <->  Streamable HTTP           │
 └───────────────────────┬──────────────────────┘
        │   JSON-RPC over stdio
        ▼
 ┌──────────────────────────────────────────────┐
 │  YOUR MCP SERVER                              │
 │  node · python · go · rust · docker           │
 └──────────────────────────────────────────────┘

 BUILDER (Rust)  GitHub repo -> detect -> Dockerfile -> Fly image -> deploy -> verify initialize
 RUNNER  (Deno)  run_code -> Firecracker sandbox -> tools.* -> proxy (scope re-checked per call)`}</Diagram>
              <p className="text-sm text-gray-500">{t('sec.architecture.caption')}</p>
            </Section>

            {/* Builder */}
            <Section id="builder" title={t('sec.builder.title')}>
              <Paras items={raw<string[]>('sec.builder.p', [])} />
              <Steps items={raw<{ t: string; d: string }[]>('sec.builder.steps', [])} />
              <Code>{`# What the Builder emits for a stdio MCP server (any language):
CMD ["node", "stdio-adapter.cjs", "npm", "start"]         # Node
CMD ["node", "stdio-adapter.cjs", "python", "server.py"]  # Python
CMD ["node", "stdio-adapter.cjs", "./server"]             # Go / Rust binary
CMD ["node", "stdio-adapter.cjs", "npx", "-y", "pkg"]     # npx package`}</Code>
              <div className="rounded-lg border border-gray-300 bg-gray-50 p-4 text-sm text-gray-700">
                {t('sec.builder.note')}
              </div>
              <Why text={t('sec.builder.why')} />
            </Section>

            {/* stdio -> HTTP adapter */}
            <Section id="adapter" title={t('sec.adapter.title')}>
              <Paras items={raw<string[]>('sec.adapter.p', [])} />
              <Code>{`// Each client request gets a private JSON-RPC id, so concurrent
// callers never collide on the one shared child process.
const internalId = \`nf-\${n++}\`;
child.stdin.write(JSON.stringify({ ...msg, id: internalId }) + "\\n");
// On reply, the caller's original id is restored before responding.

// Health: /health returns 503 once the restart budget is spent,
// so Fly marks the machine unhealthy instead of silently 500-ing.`}</Code>
              <FeatureGrid items={raw<{ t: string; d: string }[]>('sec.adapter.features', [])} />
              <Why text={t('sec.adapter.why')} />
            </Section>

            {/* Proxy */}
            <Section id="proxy" title={t('sec.proxy.title')}>
              <Paras items={raw<string[]>('sec.proxy.p', [])} />
              <FeatureGrid items={raw<{ t: string; d: string }[]>('sec.proxy.features', [])} />
              <p className="text-sm font-semibold text-gray-900 mb-1">{t('sec.proxy.scopeTitle')}</p>
              <Code>{`*                        full access
tools:list               list tools
tools:call               call any tool
tools:call:get_weather   call only the get_weather tool
resources:read:<uri>     read one specific resource
prompts:get:<name>       get one specific prompt`}</Code>
              <Why text={t('sec.proxy.why')} />
            </Section>

            {/* Token-optimization features */}
            <Section id="tokens" title={t('sec.tokens.title')}>
              <Paras items={raw<string[]>('sec.tokens.p', [])} />
              <div className="my-6 space-y-4">
                {raw<{ flag: string; t: string; d: string }[]>('sec.tokens.items', []).map((it, i) => (
                  <div key={i} className="rounded-lg border border-gray-200 p-4">
                    <code className="inline-block rounded bg-gray-900 px-2 py-0.5 text-[13px] font-mono text-gray-100 mb-2">
                      {it.flag}
                    </code>
                    <p className="font-semibold text-gray-900">{it.t}</p>
                    <p className="text-sm text-gray-600 leading-relaxed">{it.d}</p>
                  </div>
                ))}
              </div>
              <Why text={t('sec.tokens.why')} />
            </Section>

            {/* Code mode + runner */}
            <Section id="code" title={t('sec.code.title')}>
              <Paras items={raw<string[]>('sec.code.p', [])} />
              <Code>{`// tool_code_mode: the model writes JavaScript instead of many
// round-trips. tools.* is injected; every call is scope-checked
// server-side and counts against a per-run tool-call budget.
const top = await tools.searchVideos({ query: "rust async", maxResults: 5 });
const details = await Promise.all(
  top.map(v => tools.getVideoDetails({ videoIds: [v.id] }))
);
return details.filter(d => d.likeToViewRatio > 0.04);`}</Code>
              <Steps items={raw<{ t: string; d: string }[]>('sec.code.steps', [])} />
              <div className="rounded-lg border border-gray-200 bg-gray-50 p-4 text-sm text-gray-700">
                {t('sec.code.limits')}
              </div>
              <Why text={t('sec.code.why')} />
            </Section>

            {/* Hosting model */}
            <Section id="hosting" title={t('sec.hosting.title')}>
              <Paras items={raw<string[]>('sec.hosting.p', [])} />
              <FeatureGrid items={raw<{ t: string; d: string }[]>('sec.hosting.features', [])} />
              <Why text={t('sec.hosting.why')} />
            </Section>

            {/* Security */}
            <Section id="security" title={t('sec.security.title')}>
              <Paras items={raw<string[]>('sec.security.p', [])} />
              <ul className="space-y-2">
                {raw<string[]>('sec.security.points', []).map((p, i) => (
                  <li key={i} className="flex items-start gap-2 text-gray-600">
                    <span className="text-gray-900 mt-1 font-bold">✓</span>
                    <span>{p}</span>
                  </li>
                ))}
              </ul>
            </Section>

            {/* Contact */}
            <div className="mt-12 pt-8 border-t">
              <p className="text-gray-600 mb-4">{t('contact.prompt')}</p>
              <Link href="/contact" className="text-gray-900 font-medium underline underline-offset-4 hover:text-gray-600">
                {t('contact.link')}
              </Link>
            </div>
          </main>
        </div>
      </div>

      <Footer />
    </div>
  );
}

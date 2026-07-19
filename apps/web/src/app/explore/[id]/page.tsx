'use client';

import { useQuery } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import Link from 'next/link';
import { useParams } from 'next/navigation';
import { ServerTemplatePublic } from '@/types';
import { Header } from '@/components/layout';
import { SiNodedotjs, SiPython, SiGo, SiRust, SiDocker } from 'react-icons/si';
import { Github, Key, Rocket, ArrowLeft, X } from 'lucide-react';
import { useState } from 'react';
import { createPortal } from 'react-dom';
import { Button } from '@/components/ui/button';
import { useAuth } from '@/hooks/use-auth';

const API_BASE = process.env.NEXT_PUBLIC_API_URL
  ? `${process.env.NEXT_PUBLIC_API_URL}/api/v1`
  : '/api/v1';

async function fetchTemplate(id: string): Promise<ServerTemplatePublic> {
  const res = await fetch(`${API_BASE}/templates/${id}`, {
    credentials: 'include',
  });
  if (res.status === 404) throw new NotFoundError();
  if (!res.ok) throw new Error('Failed to fetch template');
  return res.json();
}

class NotFoundError extends Error {}

const RUNTIME_LABELS: Record<string, string> = {
  node: 'Node.js',
  python: 'Python',
  go: 'Go',
  rust: 'Rust',
  docker: 'Docker',
};

const TRANSPORT_LABELS: Record<string, string> = {
  sse: 'Streamable HTTP',
  stdio: 'STDIO',
};

function RuntimeIcon({ runtime, size = 'md' }: { runtime: string; size?: 'sm' | 'md' }) {
  const cls = size === 'md' ? 'w-5 h-5' : 'w-4 h-4';
  switch (runtime) {
    case 'node':   return <SiNodedotjs className={`${cls} text-green-600`} />;
    case 'python': return <SiPython className={`${cls} text-blue-500`} />;
    case 'go':     return <SiGo className={`${cls} text-cyan-500`} />;
    case 'rust':   return <SiRust className={`${cls} text-orange-600`} />;
    case 'docker': return <SiDocker className={`${cls} text-sky-500`} />;
    default:       return null;
  }
}

function DetailRow({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex flex-col sm:flex-row sm:items-start gap-1 sm:gap-4 py-3 border-b border-gray-100 last:border-0">
      <span className="text-xs font-medium text-gray-400 uppercase tracking-wide sm:w-36 shrink-0">{label}</span>
      <span className="text-sm text-gray-700 font-mono">{value}</span>
    </div>
  );
}

export default function TemplateDetailPage() {
  const t = useTranslations('explore');
  const { user } = useAuth();
  const params = useParams();
  const id = params.id as string;

  const [showAuthModal, setShowAuthModal] = useState(false);

  const { data: tmpl, isLoading, isError, error } = useQuery<ServerTemplatePublic>({
    queryKey: ['template', id],
    queryFn: () => fetchTemplate(id),
    retry: (count, err) => !(err instanceof NotFoundError) && count < 2,
  });

  const isNotFound = isError && error instanceof NotFoundError;

  return (
    <div className="min-h-screen bg-white">
      <Header />

      <div className="max-w-3xl mx-auto px-4 sm:px-6 py-8">
        <Link
          href="/explore"
          className="inline-flex items-center gap-1.5 text-sm text-gray-500 hover:text-gray-800 transition-colors mb-6"
        >
          <ArrowLeft className="w-4 h-4" />
          {t('detail.back')}
        </Link>

        {isLoading && (
          <div className="space-y-4 animate-pulse">
            <div className="h-8 bg-gray-100 rounded-xl w-2/3" />
            <div className="h-4 bg-gray-100 rounded-lg w-1/3" />
            <div className="h-24 bg-gray-100 rounded-2xl mt-6" />
            <div className="h-48 bg-gray-100 rounded-2xl" />
          </div>
        )}

        {isNotFound && (
          <div className="text-center py-20">
            <p className="text-base font-medium text-gray-700">{t('detail.notFound')}</p>
            <p className="text-sm text-gray-400 mt-1">{t('detail.notFoundDesc')}</p>
            <Link href="/explore" className="mt-6 inline-block text-sm text-violet-600 hover:underline">
              {t('detail.back')}
            </Link>
          </div>
        )}

        {isError && !isNotFound && (
          <div className="text-center py-20 text-sm text-gray-400">
            Failed to load template. Please try again later.
          </div>
        )}

        {tmpl && (
          <div className="space-y-6">
            {/* Header */}
            <div className="flex items-start justify-between gap-4">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-3 flex-wrap">
                  <h1 className="text-2xl font-bold text-gray-900">{tmpl.name}</h1>
                  <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full bg-gray-100 text-gray-600 text-xs font-medium">
                    <RuntimeIcon runtime={tmpl.runtime} size="sm" />
                    {RUNTIME_LABELS[tmpl.runtime] ?? tmpl.runtime}
                  </span>
                </div>
                <div className="flex items-center gap-4 mt-2 text-xs text-gray-400">
                  <span>{t('detail.deployCount', { count: tmpl.use_count.toLocaleString() })}</span>
                  <span>·</span>
                  <span>{t('detail.publishedOn')} {new Date(tmpl.created_at).toLocaleDateString()}</span>
                </div>
              </div>

              {user ? (
                <Link href={`/dashboard/servers/new?template=${tmpl.id}`} className="shrink-0">
                  <Button className="bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white">
                    <Rocket className="w-4 h-4 mr-2" />
                    {t('deploy')}
                  </Button>
                </Link>
              ) : (
                <Button
                  onClick={() => setShowAuthModal(true)}
                  className="shrink-0 bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white"
                >
                  <Rocket className="w-4 h-4 mr-2" />
                  {t('deploy')}
                </Button>
              )}
            </div>

            {/* Description */}
            <div className="rounded-2xl border border-gray-200 p-6">
              <h2 className="text-sm font-semibold text-gray-700 mb-3">Description</h2>
              {tmpl.description ? (
                <p className="text-sm text-gray-600 whitespace-pre-wrap leading-relaxed">{tmpl.description}</p>
              ) : (
                <p className="text-sm text-gray-400 italic">{t('detail.noDescription')}</p>
              )}
            </div>

            {/* GitHub */}
            <div className="rounded-2xl border border-gray-200 p-6">
              <h2 className="text-sm font-semibold text-gray-700 mb-3">Repository</h2>
              <a
                href={`https://github.com/${tmpl.github_repo}`}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-2 text-sm text-gray-700 hover:text-violet-700 transition-colors"
              >
                <Github className="w-4 h-4 shrink-0" />
                <span className="font-mono">{tmpl.github_repo}</span>
                <span className="text-gray-400">@ {tmpl.github_branch}</span>
              </a>
            </div>

            {/* Required env vars */}
            {tmpl.required_env_var_keys.length > 0 && (
              <div className="rounded-2xl border border-amber-100 bg-amber-50/50 p-6">
                <h2 className="text-sm font-semibold text-gray-700 mb-3">{t('requiredEnvVars')}</h2>
                <div className="flex flex-wrap gap-2">
                  {tmpl.required_env_var_keys.map((k) => (
                    <span
                      key={k}
                      className="inline-flex items-center gap-1.5 px-3 py-1 rounded-lg bg-white border border-amber-200 text-xs font-mono text-amber-700"
                    >
                      <Key className="w-3 h-3" />
                      {k}
                    </span>
                  ))}
                </div>
                <p className="text-xs text-amber-600 mt-3">
                  You will be prompted to set these environment variables when deploying.
                </p>
              </div>
            )}

            {/* Technical details */}
            <div className="rounded-2xl border border-gray-200 p-6">
              <h2 className="text-sm font-semibold text-gray-700 mb-1">{t('detail.technicalDetails')}</h2>
              <div className="mt-2">
                <DetailRow label={t('detail.transport')} value={TRANSPORT_LABELS[tmpl.transport] ?? tmpl.transport} />
                <DetailRow label={t('detail.mcpPath')} value={tmpl.mcp_path} />
                {tmpl.root_directory && tmpl.root_directory !== '.' && (
                  <DetailRow label={t('detail.rootDirectory')} value={tmpl.root_directory} />
                )}
                {tmpl.entry_command && (
                  <DetailRow label={t('detail.entryCommand')} value={tmpl.entry_command} />
                )}
                {tmpl.build_command && (
                  <DetailRow label={t('detail.buildCommand')} value={tmpl.build_command} />
                )}
                <DetailRow
                  label={t('detail.memory')}
                  value={tmpl.memory_mb ? `${tmpl.memory_mb} MB` : t('detail.memoryDefault')}
                />
                {tmpl.port && (
                  <DetailRow label={t('detail.port')} value={String(tmpl.port)} />
                )}
                <DetailRow
                  label={t('detail.auth')}
                  value={
                    tmpl.auth_enabled
                      ? t('detail.authEnabled')
                      : <span className="text-amber-600">{t('detail.authDisabled')}</span>
                  }
                />
                {tmpl.upstream_oauth_provider && (
                  <DetailRow label={t('detail.oauthProvider')} value={tmpl.upstream_oauth_provider} />
                )}
                {tmpl.upstream_oauth_scopes.length > 0 && (
                  <DetailRow
                    label={t('detail.oauthScopes')}
                    value={tmpl.upstream_oauth_scopes.join(' ')}
                  />
                )}
              </div>
            </div>

            {/* Deploy CTA (bottom) */}
            <div className="rounded-2xl border border-violet-100 bg-violet-50/50 p-6 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
              <div>
                <p className="text-sm font-semibold text-gray-800">Deploy this MCP server</p>
                <p className="text-xs text-gray-500 mt-0.5">
                  {tmpl.use_count.toLocaleString()} {t('results')} have already deployed this
                </p>
              </div>
              {user ? (
                <Link href={`/dashboard/servers/new?template=${tmpl.id}`} className="shrink-0">
                  <Button className="bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white">
                    <Rocket className="w-4 h-4 mr-2" />
                    {t('deploy')}
                  </Button>
                </Link>
              ) : (
                <Button
                  onClick={() => setShowAuthModal(true)}
                  className="shrink-0 bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white"
                >
                  <Rocket className="w-4 h-4 mr-2" />
                  {t('deploy')}
                </Button>
              )}
            </div>
          </div>
        )}
      </div>

      {showAuthModal && tmpl && createPortal(
        <div
          className="fixed inset-0 z-[100] flex items-center justify-center p-4 bg-black/40"
          onClick={() => setShowAuthModal(false)}
        >
          <div
            className="relative bg-white rounded-2xl shadow-xl w-full max-w-sm p-7 flex flex-col gap-4"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center gap-2">
              <Rocket className="w-4 h-4 text-gray-900 shrink-0" />
              <h2 className="text-base font-semibold text-gray-900">{t('authModal.title')}</h2>
            </div>
            <p className="text-sm text-gray-500">{t('authModal.desc', { name: tmpl.name })}</p>
            <div className="flex gap-2 w-full">
              <Link
                href={`/signup?return_to=${encodeURIComponent(`/dashboard/servers/new?template=${tmpl.id}`)}`}
                className="flex-1 text-center py-2.5 text-sm font-semibold rounded-xl bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white transition-colors"
              >
                {t('authModal.signup')}
              </Link>
              <Link
                href={`/login?return_to=${encodeURIComponent(`/dashboard/servers/new?template=${tmpl.id}`)}`}
                className="flex-1 text-center py-2.5 text-sm font-medium rounded-xl border border-gray-200 text-gray-700 hover:bg-gray-50 transition-colors"
              >
                {t('authModal.login')}
              </Link>
            </div>
            <button
              onClick={() => setShowAuthModal(false)}
              className="absolute top-4 right-4 p-1.5 rounded-lg text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>,
        document.body
      )}
    </div>
  );
}

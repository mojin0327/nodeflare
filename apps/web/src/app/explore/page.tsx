'use client';

import { useQuery } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import Link from 'next/link';
import { ServerTemplatePublic } from '@/types';
import { Header } from '@/components/layout';
import { SiNodedotjs, SiPython, SiGo, SiRust, SiDocker } from 'react-icons/si';
import { Github, Key, Rocket, Search, X } from 'lucide-react';
import { useState } from 'react';
import { createPortal } from 'react-dom';
import { Button } from '@/components/ui/button';
import { useAuth } from '@/hooks/use-auth';

const API_BASE = process.env.NEXT_PUBLIC_API_URL
  ? `${process.env.NEXT_PUBLIC_API_URL}/api/v1`
  : '/api/v1';

async function fetchTemplates(): Promise<ServerTemplatePublic[]> {
  const res = await fetch(`${API_BASE}/templates?limit=100`, {
    credentials: 'include',
  });
  if (!res.ok) throw new Error('Failed to fetch templates');
  return res.json();
}

function RuntimeIcon({ runtime }: { runtime: string }) {
  switch (runtime) {
    case 'node':   return <SiNodedotjs className="w-4 h-4 text-green-600" />;
    case 'python': return <SiPython className="w-4 h-4 text-blue-500" />;
    case 'go':     return <SiGo className="w-4 h-4 text-cyan-500" />;
    case 'rust':   return <SiRust className="w-4 h-4 text-orange-600" />;
    case 'docker': return <SiDocker className="w-4 h-4 text-sky-500" />;
    default:       return null;
  }
}

const RUNTIME_LABELS: Record<string, string> = {
  node: 'Node.js',
  python: 'Python',
  go: 'Go',
  rust: 'Rust',
  docker: 'Docker',
};

export default function ExplorePage() {
  const t = useTranslations('explore');
  const { user } = useAuth();
  const [input, setInput] = useState('');
  const [query, setQuery] = useState('');
  const [showAuthModal, setShowAuthModal] = useState(false);
  const [pendingTemplate, setPendingTemplate] = useState<ServerTemplatePublic | null>(null);

  const { data: templates = [], isLoading, isError } = useQuery<ServerTemplatePublic[]>({
    queryKey: ['templates'],
    queryFn: fetchTemplates,
    staleTime: 60_000,
  });

  const handleSearch = () => setQuery(input.trim());

  const filtered = templates.filter((tmpl) => {
    if (!query) return true;
    const q = query.toLowerCase();
    return (
      tmpl.name.toLowerCase().includes(q) ||
      tmpl.description?.toLowerCase().includes(q) ||
      tmpl.github_repo.toLowerCase().includes(q) ||
      tmpl.runtime.toLowerCase().includes(q)
    );
  });

  return (
    <div className="min-h-screen bg-white">
      <Header />

      {/* Page header */}
      <div className="max-w-6xl mx-auto px-4 sm:px-6 py-8 flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold text-gray-900">{t('title')}</h1>
          <p className="text-sm text-gray-500 mt-0.5">{t('subtitle')}</p>
        </div>
        <form
          className="flex h-10 w-full sm:w-auto"
          onSubmit={(e) => { e.preventDefault(); handleSearch(); }}
        >
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder={t('searchPlaceholder')}
            className="flex-1 sm:w-72 pl-4 pr-3 text-sm border border-gray-400 rounded-l-full bg-white focus:outline-none focus:border-violet-400"
          />
          <button
            type="submit"
            className="flex items-center px-4 text-gray-600 bg-gray-100 hover:bg-gray-200 border border-gray-400 border-l border-l-gray-400 rounded-r-full transition-colors shrink-0"
          >
            <Search className="w-4 h-4" />
          </button>
        </form>
      </div>

      {/* Grid */}
      <div className="max-w-6xl mx-auto px-4 sm:px-6 pb-12">
        {!isLoading && !isError && (
          <p className="text-sm text-gray-400 mb-5">
            {filtered.length} {t('results')}
          </p>
        )}

        {isLoading && (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-5">
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={i} className="rounded-2xl border border-gray-200 p-6 animate-pulse h-52 bg-gray-50" />
            ))}
          </div>
        )}

        {isError && (
          <div className="text-center py-20 text-sm text-gray-400">
            Failed to load templates. Please try again later.
          </div>
        )}

        {!isLoading && !isError && filtered.length === 0 && (
          <div className="text-center py-20">
            <p className="text-base font-medium text-gray-500">{t('emptyTitle')}</p>
            <p className="text-sm text-gray-400 mt-1">{t('emptyDesc')}</p>
          </div>
        )}

        {!isLoading && !isError && filtered.length > 0 && (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-5">
            {filtered.map((tmpl) => (
              <TemplateCard
                key={tmpl.id}
                template={tmpl}
                deployLabel={t('deploy')}
                onDeploy={() => {
                  if (!user) {
                    setPendingTemplate(tmpl);
                    setShowAuthModal(true);
                  }
                }}
                isAuthed={!!user}
              />
            ))}
          </div>
        )}
      </div>

      {showAuthModal && pendingTemplate && createPortal(
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
            <p className="text-sm text-gray-500">{t('authModal.desc', { name: pendingTemplate.name })}</p>
            <div className="flex gap-2 w-full">
              <Link
                href={`/signup?return_to=${encodeURIComponent(`/dashboard/servers/new?template=${pendingTemplate.id}`)}`}
                className="flex-1 text-center py-2.5 text-sm font-semibold rounded-xl bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white transition-colors"
              >
                {t('authModal.signup')}
              </Link>
              <Link
                href={`/login?return_to=${encodeURIComponent(`/dashboard/servers/new?template=${pendingTemplate.id}`)}`}
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

function TemplateCard({
  template: tmpl,
  deployLabel,
  onDeploy,
  isAuthed,
}: {
  template: ServerTemplatePublic;
  deployLabel: string;
  onDeploy: () => void;
  isAuthed: boolean;
}) {
  return (
    <div className="flex flex-col rounded-2xl border border-gray-200 hover:border-gray-300 hover:shadow-lg transition-all duration-200 bg-white p-6 gap-3">
      <div className="flex items-start justify-between gap-3">
        <h3 className="font-semibold text-gray-900 text-base leading-snug">{tmpl.name}</h3>
        <span className="flex items-center gap-1.5 text-gray-500 text-xs font-medium shrink-0">
          <RuntimeIcon runtime={tmpl.runtime} />
          {RUNTIME_LABELS[tmpl.runtime] ?? tmpl.runtime}
        </span>
      </div>

      {tmpl.description && (
        <p className="text-sm text-gray-500 line-clamp-2">{tmpl.description}</p>
      )}

      <a
        href={`https://github.com/${tmpl.github_repo}`}
        target="_blank"
        rel="noopener noreferrer"
        className="inline-flex items-center gap-1.5 text-xs text-gray-400 hover:text-gray-700 transition-colors w-fit"
      >
        <Github className="w-3.5 h-3.5 shrink-0" />
        <span className="truncate max-w-[220px]">{tmpl.github_repo}</span>
      </a>

      {tmpl.required_env_var_keys.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {tmpl.required_env_var_keys.map((k) => (
            <span
              key={k}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-amber-50 border border-amber-200 text-xs font-mono text-amber-700"
            >
              <Key className="w-2.5 h-2.5" />
              {k}
            </span>
          ))}
        </div>
      )}

      <div className="flex items-center justify-between mt-auto pt-2">
        <span className="text-xs text-gray-400">{tmpl.use_count.toLocaleString()} deploys</span>
        {isAuthed ? (
          <Link href={`/dashboard/servers/new?template=${tmpl.id}`}>
            <Button size="sm" className="h-7 text-xs px-2.5 bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white">
              <Rocket className="w-3.5 h-3.5 mr-1" />
              {deployLabel}
            </Button>
          </Link>
        ) : (
          <Button size="sm" onClick={onDeploy} className="h-7 text-xs px-2.5 bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white">
            <Rocket className="w-3.5 h-3.5 mr-1" />
            {deployLabel}
          </Button>
        )}
      </div>
    </div>
  );
}

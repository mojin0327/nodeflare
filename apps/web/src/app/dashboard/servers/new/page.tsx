'use client';

import { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useRouter } from 'next/navigation';
import { useTranslations } from 'next-intl';
import { Lock, Users, Globe, Server, Check, Link, Search, Folder, AlertCircle, Info, GitBranch, Terminal, AlertTriangle, XCircle, Plus, ArrowRight, MonitorPlay, Trash2, KeyRound, Loader2, ChevronDown } from 'lucide-react';
import { api } from '@/lib/api';
import { getLinkedAccounts, getRepos, LinkedGitHubAccount, inspectRepo, RepoDetection } from '@/lib/github-api';
import { CreateServerRequest, McpServer, Runtime, Visibility, GitHubRepo } from '@/types';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { GitHubAccountSelector } from '@/components/github/GitHubAccountSelector';
import { MemorySelect } from '@/components/servers/memory-select';
import { DEFAULT_MEMORY_MB } from '@/lib/plans';
import { useWorkspace } from '@/hooks/use-workspace';
import { SiNodedotjs, SiPython, SiGo, SiRust, SiDocker, SiGithub } from 'react-icons/si';
import { useSetPageHeader } from '../../page-header';

type SourceType = 'my-repos' | 'public-url';

/** Stable React Query key for a repo-inspect target. */
const inspectKey = (repo: string, branch?: string, subdir?: string) =>
  ['inspect', repo, branch ?? '', subdir ?? ''] as const;

/** Fetcher for the inspect cache. Omits an empty branch so the backend resolves default. */
const inspectFn = (
  repo: string,
  branch?: string,
  subdir?: string,
  accountId?: string,
): Promise<RepoDetection> =>
  inspectRepo({
    github_repo: repo,
    github_branch: branch || undefined,
    account_id: accountId || undefined,
    root_directory: subdir,
  });

/** Subtle shimmer placeholder shown in place of a value while detection is in flight. */
function Skeleton({ className = '' }: { className?: string }) {
  return <div className={`animate-pulse rounded bg-gray-100 ${className}`} />;
}

export default function NewServerPage() {
  const t = useTranslations('servers');
  const tCommon = useTranslations('common');
  const tApiErrors = useTranslations('apiErrors');
  const router = useRouter();
  const queryClient = useQueryClient();

  useSetPageHeader(t('create.title'), <Server className="w-4 h-4" />);

  const { activeWorkspace } = useWorkspace();

  const workspaceId = activeWorkspace?.id;

  // Plan limits drive which memory sizes are selectable (Free is capped at 256MB).
  const { data: plans } = useQuery<{ plan: string; limits: { max_memory_mb: number } }[]>({
    queryKey: ['billing-plans'],
    queryFn: () => api.get('/billing/plans'),
  });
  const currentPlan = activeWorkspace?.plan || 'free';
  const maxMemoryMb = plans?.find((p) => p.plan === currentPlan)?.limits.max_memory_mb ?? 256;

  // Linked GitHub accounts
  const { data: linkedAccounts, isLoading: accountsLoading } = useQuery<LinkedGitHubAccount[]>({
    queryKey: ['linked-github-accounts'],
    queryFn: getLinkedAccounts,
  });

  const [selectedAccountId, setSelectedAccountId] = useState<string | null>(null);

  // Auto-select primary account when accounts load
  useEffect(() => {
    if (linkedAccounts && linkedAccounts.length > 0 && !selectedAccountId) {
      const primary = linkedAccounts.find((a) => a.is_primary);
      setSelectedAccountId(primary?.id || linkedAccounts[0].id);
    }
  }, [linkedAccounts, selectedAccountId]);

  // Fetch repos from selected account
  const { data: repos, isLoading: reposLoading } = useQuery<GitHubRepo[]>({
    queryKey: ['github-repos', selectedAccountId],
    queryFn: () => getRepos(selectedAccountId || undefined),
    enabled: !!selectedAccountId,
  });

  const [sourceType, setSourceType] = useState<SourceType>('my-repos');
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedRepo, setSelectedRepo] = useState<GitHubRepo | null>(null);
  const [publicRepoUrl, setPublicRepoUrl] = useState('');
  const [publicRepoError, setPublicRepoError] = useState<string | null>(null);

  const filteredRepos = repos?.filter(
    (repo) =>
      repo.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      repo.full_name.toLowerCase().includes(searchQuery.toLowerCase())
  );

  const [formData, setFormData] = useState<CreateServerRequest>({
    name: '',
    slug: '',
    description: '',
    github_repo: '',
    github_branch: 'main',
    runtime: 'node',
    visibility: 'private',
    transport: 'sse',
    root_directory: '',
    mcp_path: '/mcp',
    auth_enabled: true,
    memory_mb: DEFAULT_MEMORY_MB,
  });
  // Environment variables to provision at creation time (sent before the initial
  // deploy so the first build already has them). Stored as a simple key/value list.
  const [envVars, setEnvVars] = useState<{ key: string; value: string }[]>([]);

  const addEnvVar = useCallback(() => {
    setEnvVars((prev) => [...prev, { key: '', value: '' }]);
  }, []);

  const updateEnvVar = useCallback((index: number, field: 'key' | 'value', value: string) => {
    setEnvVars((prev) =>
      prev.map((env, i) =>
        i === index
          ? { ...env, [field]: field === 'key' ? value.toUpperCase() : value }
          : env
      )
    );
  }, []);

  const removeEnvVar = useCallback((index: number) => {
    setEnvVars((prev) => prev.filter((_, i) => i !== index));
  }, []);

  // Auto-detect deploy config from the repo (Vercel-style): silently pre-fill the fields
  // below with the same values the builder will use. Every field stays editable.
  const detectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Detected deploy settings are folded away (Vercel-style); env vars stay visible.
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const applyDetection = useCallback((d: RepoDetection) => {
    setFormData((prev) => ({
      ...prev,
      github_branch: d.branch || prev.github_branch,
      runtime: d.runtime ?? prev.runtime,
      transport: d.transport ?? prev.transport,
      root_directory: d.root_directory ?? prev.root_directory,
      entry_command: d.entry_command ?? prev.entry_command,
      build_command: d.build_command ?? prev.build_command,
      mcp_path: d.mcp_path ?? prev.mcp_path,
      port: d.port ?? prev.port,
    }));
    // Prefill env var keys (values left blank for the user, like Vercel).
    if (d.env_vars.length > 0) {
      setEnvVars((prev) => {
        const existing = new Set(prev.map((e) => e.key));
        const additions = d.env_vars
          .filter((e) => !existing.has(e.key))
          .map((e) => ({ key: e.key, value: '' }));
        const base = prev.filter((e) => e.key.trim() !== '');
        return [...base, ...additions];
      });
    }
  }, []);

  // Auto-detect runs through a React Query CACHE keyed by the target, so hover-prefetch,
  // request dedup, and staleTime all come for free. `detectTarget` is the currently-selected
  // repo/branch/subdir; setting it drives the active `detectQuery` below.
  const [detectTarget, setDetectTarget] = useState<{
    repo: string;
    branch?: string;
    subdir?: string;
  } | null>(null);

  const detectQuery = useQuery<RepoDetection>({
    queryKey: detectTarget
      ? inspectKey(detectTarget.repo, detectTarget.branch, detectTarget.subdir)
      : ['inspect', 'idle'],
    queryFn: () =>
      inspectFn(
        detectTarget!.repo,
        detectTarget!.branch,
        detectTarget!.subdir,
        selectedAccountId || undefined,
      ),
    enabled: !!detectTarget,
    staleTime: 5 * 60_000,
  });

  // Apply each new detection result exactly once. React Query hands back a stable `data`
  // reference per result, so guarding on object identity re-applies when a *new* result
  // arrives (e.g. after switching repos) but never double-applies on re-render.
  const appliedRef = useRef<RepoDetection | null>(null);
  useEffect(() => {
    const data = detectQuery.data;
    if (data && appliedRef.current !== data) {
      appliedRef.current = data;
      applyDetection(data);
    }
  }, [detectQuery.data, applyDetection]);

  // Warm the inspect cache ahead of selection (repo hover/focus, URL paste). Dedup is
  // automatic — React Query won't refire an in-flight or still-fresh key.
  const prefetchDetect = useCallback(
    (repo: string, branch?: string, subdir?: string) => {
      if (!repo) return;
      queryClient.prefetchQuery({
        queryKey: inspectKey(repo, branch, subdir),
        queryFn: () => inspectFn(repo, branch, subdir, selectedAccountId || undefined),
        staleTime: 5 * 60_000,
      });
    },
    [queryClient, selectedAccountId],
  );

  // Selecting a target just points `detectTarget` at it; the useQuery (warmed by prefetch)
  // does the rest, so a click after hover is an instant cache hit.
  const runDetect = useCallback(
    (repo: string, branch: string | undefined, subdir?: string) => {
      if (!repo) return;
      setDetectTarget({ repo, branch: branch || undefined, subdir });
    },
    [],
  );

  // True while the active target's detection is actually in flight (false on a warm cache
  // hit); drives the skeletons so detected values slot in instead of popping.
  const detecting = !!detectTarget && detectQuery.isFetching;

  const generateSlug = useCallback((name: string) => {
    return name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .substring(0, 63);
  }, []);

  // Parse GitHub URL to extract owner/repo and, when present, the branch and
  // subdirectory from a `/tree/<branch>/<path>` or `/blob/<branch>/<path>` URL.
  // Note: branch is taken as the first segment after tree/blob, so slashed branch
  // names (e.g. `feature/x`) get split into the path — rare, and user-overridable.
  const parseGitHubUrl = useCallback((input: string): {
    owner: string;
    repo: string;
    branch?: string;
    subdir?: string;
  } | null => {
    // Full URLs, optionally pointing at a branch + path inside the repo.
    const urlMatch = input.match(
      /github\.com\/([^\/\s#?]+)\/([^\/\s#?]+)(?:\/(?:tree|blob)\/([^\/\s#?]+)(?:\/([^\s#?]+))?)?/
    );
    if (urlMatch) {
      return {
        owner: urlMatch[1],
        repo: urlMatch[2].replace(/\.git$/, ''),
        branch: urlMatch[3] || undefined,
        subdir: urlMatch[4]?.replace(/\/+$/, '') || undefined,
      };
    }
    // Handle owner/repo format
    const shortMatch = input.match(/^([^\/\s]+)\/([^\/\s]+)$/);
    if (shortMatch) {
      return { owner: shortMatch[1], repo: shortMatch[2].replace(/\.git$/, '') };
    }
    return null;
  }, []);

  // Handle public repo URL input
  const handlePublicRepoChange = useCallback((value: string) => {
    setPublicRepoUrl(value);
    setPublicRepoError(null);

    if (!value.trim()) {
      setFormData(prev => ({ ...prev, github_repo: '', name: '', slug: '' }));
      return;
    }

    const parsed = parseGitHubUrl(value);
    if (parsed) {
      // Name/slug from the subdir when the URL targets one (e.g. a monorepo
      // member like `src/filesystem`), otherwise from the repo.
      const leaf = parsed.subdir ? parsed.subdir.split('/').pop()! : parsed.repo;
      const slug = generateSlug(leaf);
      setFormData(prev => ({
        ...prev,
        github_repo: `${parsed.owner}/${parsed.repo}`,
        name: leaf,
        slug: slug,
        // Only pin the branch when the URL specified one; otherwise detection fills the
        // repo's actual default branch.
        github_branch: parsed.branch || '',
        root_directory: parsed.subdir || '',
      }));
      // Kick off the fetch ASAP — warm the cache the moment we have a valid repo so the
      // network round-trip overlaps with the user finishing their paste, independent of
      // the debounce below (which still gates when we actually set the active target).
      prefetchDetect(`${parsed.owner}/${parsed.repo}`, parsed.branch, parsed.subdir);
      // Debounce auto-detection while the user is still typing/pasting the URL.
      if (detectTimer.current) clearTimeout(detectTimer.current);
      detectTimer.current = setTimeout(() => {
        runDetect(`${parsed.owner}/${parsed.repo}`, parsed.branch, parsed.subdir);
      }, 350);
    } else {
      setPublicRepoError('Invalid format. Use owner/repo or full GitHub URL');
      setFormData(prev => ({ ...prev, github_repo: '', name: '', slug: '' }));
    }
  }, [parseGitHubUrl, generateSlug, runDetect, prefetchDetect]);

  const handleSelectRepo = (repo: GitHubRepo) => {
    setSelectedRepo(repo);
    const slug = generateSlug(repo.name);
    setFormData(prev => ({
      ...prev,
      name: repo.name,
      slug: slug,
      github_repo: repo.full_name,
      github_branch: repo.default_branch,
    }));
    // Auto-detect deploy config for the selected repo.
    runDetect(repo.full_name, repo.default_branch, undefined);
  };

  const createMutation = useMutation({
    mutationFn: (data: CreateServerRequest) => {
      if (!workspaceId) throw new Error('No workspace found');
      return api.post<McpServer>(`/workspaces/${workspaceId}/servers`, data);
    },
    onSuccess: (server) => {
      // Navigate away IMMEDIATELY so the user isn't left sitting on the form for a few
      // seconds. The cache invalidations below trigger refetches of the (still-mounted)
      // list queries; running them first delays the transition, so push first and let the
      // list caches refresh in the background — they re-fetch before the list is shown.
      router.replace(`/dashboard/servers/${server.id}`);
      // List views are keyed ['servers-list'|'servers-minimal'|'servers-basic', wsId]; a
      // bare ['servers'] key doesn't match them. Invalidate each list prefix (react-query
      // prefix-matches, so the workspace-scoped variants refresh too).
      queryClient.invalidateQueries({ queryKey: ['servers-list'] });
      queryClient.invalidateQueries({ queryKey: ['servers-minimal'] });
      queryClient.invalidateQueries({ queryKey: ['servers-basic'] });
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    // Only send env vars that have a key; trim keys so stray whitespace doesn't
    // trip the backend's key validation.
    const env_vars = envVars
      .map((env) => ({ key: env.key.trim(), value: env.value }))
      .filter((env) => env.key.length > 0);
    createMutation.mutate({
      ...formData,
      env_vars: env_vars.length > 0 ? env_vars : undefined,
    });
  };

  const runtimes = useMemo(() => [
    { value: 'node', label: t('create.runtimeNode'), color: 'bg-green-600', icon: <SiNodedotjs className="w-5 h-5" /> },
    { value: 'python', label: t('create.runtimePython'), color: 'bg-blue-500', icon: <SiPython className="w-5 h-5" /> },
    { value: 'go', label: t('create.runtimeGo'), color: 'bg-cyan-500', icon: <SiGo className="w-6 h-6" /> },
    { value: 'rust', label: t('create.runtimeRust'), color: 'bg-orange-600', icon: <SiRust className="w-5 h-5" /> },
    { value: 'docker', label: t('create.runtimeDocker'), color: 'bg-sky-500', icon: <SiDocker className="w-5 h-5" /> },
  ], [t]);

  const visibilities = useMemo(() => [
    { value: 'private', label: t('create.visibilityPrivate'), desc: t('create.visibilityPrivateDesc'), icon: <Lock className="w-5 h-5" /> },
    { value: 'team', label: t('create.visibilityTeam'), desc: t('create.visibilityTeamDesc'), icon: <Users className="w-5 h-5" /> },
    { value: 'public', label: t('create.visibilityPublic'), desc: t('create.visibilityPublicDesc'), icon: <Globe className="w-5 h-5" /> },
  ], [t]);

  const errorMessage = useMemo(() => {
    if (!createMutation.isError) return null;
    const error = createMutation.error as any;
    const errorCode = error?.code;
    if (errorCode) {
      try {
        const translated = tApiErrors(errorCode);
        if (translated && translated !== errorCode) {
          return translated;
        }
      } catch {
        // Translation not found
      }
    }
    return error?.message || t('create.failed');
  }, [createMutation.isError, createMutation.error, tApiErrors, t]);

  const errorSuggestion = useMemo(() => {
    if (!createMutation.isError) return null;
    const error = createMutation.error as any;
    return error?.details?.suggestion || null;
  }, [createMutation.isError, createMutation.error]);

  return (
    <div className="max-w-2xl">
      <form onSubmit={handleSubmit} className="space-y-8">
        {/* GitHub Repository Selection */}
        <section>
          <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-4">{t('create.githubRepo')}</h2>

          {/* Source Type Cards */}
          <div className="grid grid-cols-2 gap-3 mb-4">
            <button
              type="button"
              onClick={() => {
                setSourceType('my-repos');
                setPublicRepoUrl('');
                setPublicRepoError(null);
                if (!selectedRepo) {
                  setFormData(prev => ({ ...prev, github_repo: '', name: '', slug: '' }));
                }
              }}
              className={`relative flex items-center gap-3 p-4 rounded-xl transition-all text-left ${
                sourceType === 'my-repos'
                  ? 'bg-violet-50 border border-violet-200 shadow-sm'
                  : 'bg-white border border-gray-200 hover:border-gray-300 hover:bg-gray-50'
              }`}
            >
              <div className="flex items-center justify-center flex-shrink-0">
                <SiGithub className={`w-6 h-6 ${sourceType === 'my-repos' ? 'text-violet-600' : 'text-gray-400'}`} />
              </div>
              <div className="flex-1 min-w-0">
                <span className="block font-semibold text-sm text-[#323232]">
                  {t('create.myRepos')}
                </span>
                <span className={`block text-xs mt-0.5 ${sourceType === 'my-repos' ? 'text-violet-600' : 'text-gray-500'}`}>
                  {t('create.myReposDesc')}
                </span>
              </div>
              {sourceType === 'my-repos' && (
                <div className="absolute top-3 right-3 w-5 h-5 rounded-full bg-violet-500 flex items-center justify-center">
                  <Check className="w-3 h-3 text-white" strokeWidth={3} />
                </div>
              )}
            </button>
            <button
              type="button"
              onClick={() => {
                setSourceType('public-url');
                setSelectedRepo(null);
                setFormData(prev => ({ ...prev, github_repo: '', name: '', slug: '' }));
              }}
              className={`relative flex items-center gap-3 p-4 rounded-xl transition-all text-left ${
                sourceType === 'public-url'
                  ? 'bg-violet-50 border border-violet-200 shadow-sm'
                  : 'bg-white border border-gray-200 hover:border-gray-300 hover:bg-gray-50'
              }`}
            >
              <div className="flex items-center justify-center flex-shrink-0">
                <Link className={`w-6 h-6 ${sourceType === 'public-url' ? 'text-violet-600' : 'text-gray-400'}`} />
              </div>
              <div className="flex-1 min-w-0">
                <span className="block font-semibold text-sm text-[#323232]">
                  {t('create.publicUrl')}
                </span>
                <span className={`block text-xs mt-0.5 ${sourceType === 'public-url' ? 'text-violet-600' : 'text-gray-500'}`}>
                  {t('create.publicUrlDesc')}
                </span>
              </div>
              {sourceType === 'public-url' && (
                <div className="absolute top-3 right-3 w-5 h-5 rounded-full bg-violet-500 flex items-center justify-center">
                  <Check className="w-3 h-3 text-white" strokeWidth={3} />
                </div>
              )}
            </button>
          </div>

          {sourceType === 'my-repos' ? (
            // My Repos Selection
            <div className="space-y-3">
              {/* GitHub Account Selector */}
              <div>
                <GitHubAccountSelector
                  accounts={linkedAccounts || []}
                  selectedAccountId={selectedAccountId}
                  onSelect={(id) => {
                    setSelectedAccountId(id);
                    setSelectedRepo(null);
                    setFormData(prev => ({ ...prev, github_repo: '', name: '', slug: '' }));
                  }}
                  returnTo="/dashboard/servers/new"
                  isLoading={accountsLoading}
                />
              </div>

              {selectedRepo ? (
                <div className="flex items-center justify-between p-4 rounded-xl bg-gray-50 border border-gray-200">
                  <div className="flex items-center gap-4">
                    <div className="w-12 h-12 rounded-xl bg-gray-900 flex items-center justify-center">
                      <SiGithub className="w-6 h-6 text-white" />
                    </div>
                    <div>
                      <p className="font-semibold text-gray-900">{selectedRepo.full_name}</p>
                      <div className="flex items-center gap-2 mt-1 text-sm text-gray-500">
                        {selectedRepo.private ? (
                          <span className="inline-flex items-center gap-1">
                            <Lock className="w-3.5 h-3.5" />
                            Private
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-1">
                            <Globe className="w-3.5 h-3.5" />
                            Public
                          </span>
                        )}
                        {selectedRepo.language && (
                          <>
                            <span>·</span>
                            <span>{selectedRepo.language}</span>
                          </>
                        )}
                      </div>
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick={() => {
                      setSelectedRepo(null);
                      setFormData(prev => ({ ...prev, github_repo: '', name: '', slug: '' }));
                    }}
                    className="text-sm text-violet-600 hover:text-violet-700 font-medium"
                  >
                    {tCommon('change')}
                  </button>
                </div>
              ) : linkedAccounts && linkedAccounts.length > 0 ? (
                <div className="rounded-xl border border-gray-200 bg-white overflow-hidden">
                  <div className="p-3 border-b border-gray-100">
                    <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-gray-50">
                      <Search className="w-4 h-4 text-gray-400" />
                      <input
                        type="text"
                        placeholder={t('create.searchRepos')}
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                        className="flex-1 bg-transparent text-sm focus:outline-none"
                      />
                    </div>
                  </div>
                  <div className="max-h-72 overflow-y-auto">
                    {reposLoading || accountsLoading ? (
                    <div className="p-8 flex items-center justify-center">
                      <div className="w-8 h-8 border-4 rounded-full border-gray-200 border-t-violet-600 animate-spin" />
                    </div>
                  ) : filteredRepos?.length === 0 ? (
                    <div className="p-8 text-center text-gray-500">
                      <Folder className="w-12 h-12 mx-auto mb-3 text-gray-300" />
                      {t('create.noRepos')}
                    </div>
                  ) : (
                    filteredRepos?.map((repo) => (
                      <button
                        key={repo.id}
                        type="button"
                        onClick={() => handleSelectRepo(repo)}
                        // Warm the inspect cache on hover/focus so the click is instant.
                        onMouseEnter={() => prefetchDetect(repo.full_name, repo.default_branch, undefined)}
                        onFocus={() => prefetchDetect(repo.full_name, repo.default_branch, undefined)}
                        className="w-full flex items-center gap-3 p-3 hover:bg-violet-50 transition-colors text-left border-b border-gray-50 last:border-b-0"
                      >
                        <div className="w-10 h-10 rounded-lg bg-gray-100 flex items-center justify-center flex-shrink-0">
                          <SiGithub className="w-5 h-5 text-gray-600" />
                        </div>
                        <div className="flex-1 min-w-0">
                          <p className="font-medium text-gray-900 truncate">{repo.name}</p>
                          <p className="text-sm text-gray-500 truncate">
                            {repo.description || t('create.noDescription')}
                          </p>
                        </div>
                        <div className="flex items-center gap-2 flex-shrink-0">
                          {repo.private && (
                            <span className="px-2 py-0.5 text-xs rounded-full bg-gray-100 text-gray-600">Private</span>
                          )}
                          {repo.language && (
                            <span className="text-xs text-gray-400">{repo.language}</span>
                          )}
                        </div>
                      </button>
                    ))
                  )}
                  </div>
                </div>
              ) : null}
            </div>
          ) : (
            // Public URL Input
            <div className="space-y-3">
              <div className="flex items-center gap-3 px-4 py-3 rounded-xl border border-input bg-white focus-within:border-violet-400 focus-within:ring-2 focus-within:ring-violet-100 transition-all">
                <SiGithub className="w-5 h-5 text-gray-400 flex-shrink-0" />
                <input
                  type="text"
                  placeholder="owner/repo or https://github.com/owner/repo"
                  value={publicRepoUrl}
                  onChange={(e) => handlePublicRepoChange(e.target.value)}
                  className="flex-1 bg-transparent text-sm text-gray-900 placeholder:text-gray-400 focus:outline-none"
                />
              </div>
              {publicRepoError && (
                <p className="text-sm text-red-500 flex items-center gap-1.5">
                  <AlertCircle className="w-4 h-4 flex-shrink-0" />
                  {publicRepoError}
                </p>
              )}
              <p className="text-xs text-gray-500 flex items-start gap-2">
                <Info className="w-4 h-4 flex-shrink-0 mt-0.5" />
                {t('create.publicUrlHelp')}
              </p>
            </div>
          )}
        </section>

        {/* Server Details */}
        <section>
          <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-4">{t('create.configuration')}</h2>

          {detecting && (
            <div className="flex items-center gap-2 mb-4 px-3 py-2 rounded-lg bg-violet-50 border border-violet-100 text-violet-700">
              <Loader2 className="w-4 h-4 flex-shrink-0 animate-spin" />
              <span className="text-sm font-medium">{t('create.detecting')}</span>
            </div>
          )}

          <div className="space-y-4">
            <div>
              <Label htmlFor="name" className="text-gray-700">{t('create.name')}</Label>
              <Input
                id="name"
                placeholder={t('create.namePlaceholder')}
                value={formData.name}
                onChange={(e) => {
                  const name = e.target.value;
                  setFormData(prev => ({ ...prev, name, slug: generateSlug(name) }));
                }}
                required
                className="mt-2"
              />
            </div>

            <div>
              <Label htmlFor="description" className="text-gray-700">{t('create.description')}</Label>
              <Input
                id="description"
                placeholder={t('create.descriptionBrief')}
                value={formData.description}
                onChange={(e) => setFormData(prev => ({ ...prev, description: e.target.value }))}
                className="mt-2"
              />
            </div>

            {/* Detected deploy settings — folded away like Vercel; click to edit. */}
            <button
              type="button"
              onClick={() => setAdvancedOpen((o) => !o)}
              className="flex items-center gap-1.5 pt-2 text-sm font-medium text-gray-600 hover:text-gray-900 transition-colors"
            >
              <ChevronDown className={`w-4 h-4 transition-transform ${advancedOpen ? 'rotate-180' : ''}`} />
              {t('create.advancedSettings')}
            </button>


            <div className="space-y-4">
                {advancedOpen && (
                <div className="space-y-4">
                <div>
                  <Label htmlFor="github_branch" className="text-gray-700">{t('create.branch')}</Label>
                  <div className="mt-2 flex items-center gap-2 px-3 py-2 rounded-lg border border-gray-200 bg-white">
                    <GitBranch className="w-4 h-4 text-gray-400" />
                    <input
                      id="github_branch"
                      type="text"
                      placeholder={t('create.branchPlaceholder')}
                      value={formData.github_branch}
                      onChange={(e) => setFormData(prev => ({ ...prev, github_branch: e.target.value }))}
                      className="flex-1 bg-transparent text-sm focus:outline-none"
                    />
                  </div>
                </div>

                <div>
                  <Label className="text-gray-700">{t('create.runtime')}</Label>
                  {detecting ? (
                    <div className="grid grid-cols-5 gap-3 mt-2">
                      {runtimes.map((r) => (
                        <Skeleton key={r.value} className="h-[86px]" />
                      ))}
                    </div>
                  ) : (
                  <div className="grid grid-cols-5 gap-3 mt-2">
                    {runtimes.map((runtime) => {
                      const isSelected = formData.runtime === runtime.value;
                      return (
                        <button
                          key={runtime.value}
                          type="button"
                          onClick={() => setFormData(prev => ({ ...prev, runtime: runtime.value as Runtime }))}
                          className={`p-3 rounded-lg text-center transition-all duration-200 ${
                            isSelected
                              ? 'bg-white border border-gray-100 shadow-lg scale-110 z-10'
                              : 'bg-white border border-gray-50 opacity-40 hover:opacity-70'
                          }`}
                        >
                          <div className={`${isSelected ? 'w-12 h-12' : 'w-10 h-10'} mx-auto mb-2 rounded-lg ${runtime.color} flex items-center justify-center text-white transition-all duration-200`}>
                            {runtime.icon}
                          </div>
                          <span className={`text-xs font-medium ${isSelected ? 'text-gray-900' : 'text-gray-600'}`}>{runtime.label}</span>
                        </button>
                      );
                    })}
                  </div>
                  )}
                </div>

                <div>
                  <Label className="text-gray-700">{t('create.transport')}</Label>
                  {detecting ? (
                    <div className="grid grid-cols-2 gap-3 mt-2">
                      <Skeleton className="h-[76px]" />
                      <Skeleton className="h-[76px]" />
                    </div>
                  ) : (
                  <div className="grid grid-cols-2 gap-3 mt-2">
                    <button
                      type="button"
                      onClick={() => setFormData(prev => ({ ...prev, transport: 'sse' }))}
                      className={`relative flex items-center gap-3 p-4 rounded-xl transition-all text-left ${
                        formData.transport === 'sse'
                          ? 'bg-violet-50 border border-violet-200 shadow-sm'
                          : 'bg-white border border-gray-100 hover:border-gray-200 hover:bg-gray-50'
                      }`}
                    >
                      <div className="flex items-center justify-center flex-shrink-0">
                        <ArrowRight className={`w-6 h-6 ${formData.transport === 'sse' ? 'text-violet-600' : 'text-gray-400'}`} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <span className="block font-semibold text-sm text-[#323232]">Streamable HTTP</span>
                        <span className={`block text-xs mt-0.5 ${formData.transport === 'sse' ? 'text-violet-600' : 'text-gray-500'}`}>
                          {t('create.transportSseDesc')}
                        </span>
                      </div>
                      {formData.transport === 'sse' && (
                        <div className="absolute top-3 right-3 w-5 h-5 rounded-full bg-violet-500 flex items-center justify-center">
                          <Check className="w-3 h-3 text-white" strokeWidth={3} />
                        </div>
                      )}
                    </button>
                    <button
                      type="button"
                      onClick={() => setFormData(prev => ({ ...prev, transport: 'stdio', port: undefined }))}
                      className={`relative flex items-center gap-3 p-4 rounded-xl transition-all text-left ${
                        formData.transport === 'stdio'
                          ? 'bg-violet-50 border border-violet-200 shadow-sm'
                          : 'bg-white border border-gray-100 hover:border-gray-200 hover:bg-gray-50'
                      }`}
                    >
                      <div className="flex items-center justify-center flex-shrink-0">
                        <MonitorPlay className={`w-6 h-6 ${formData.transport === 'stdio' ? 'text-violet-600' : 'text-gray-400'}`} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <span className="block font-semibold text-sm text-[#323232]">STDIO</span>
                        <span className={`block text-xs mt-0.5 ${formData.transport === 'stdio' ? 'text-violet-600' : 'text-gray-500'}`}>
                          {t('create.transportStdioDesc')}
                        </span>
                      </div>
                      {formData.transport === 'stdio' && (
                        <div className="absolute top-3 right-3 w-5 h-5 rounded-full bg-violet-500 flex items-center justify-center">
                          <Check className="w-3 h-3 text-white" strokeWidth={3} />
                        </div>
                      )}
                    </button>
                  </div>
                  )}
                </div>

                <div>
                  <Label htmlFor="root_directory" className="text-gray-700">{t('create.rootDirectory')}</Label>
                  <p className="text-xs text-gray-500 mt-1 mb-2">{t('create.rootDirectoryHelp')}</p>
                  <div className="flex items-center gap-2 px-3 py-2 rounded-lg border border-gray-200 bg-white">
                    <Folder className="w-4 h-4 text-gray-400" />
                    <input
                      id="root_directory"
                      type="text"
                      placeholder="packages/mcp-server"
                      value={formData.root_directory || ''}
                      onChange={(e) => setFormData(prev => ({ ...prev, root_directory: e.target.value }))}
                      className="flex-1 bg-transparent text-sm focus:outline-none"
                    />
                  </div>
                </div>

                <div>
                  <Label htmlFor="mcp_path" className="text-gray-700">{t('create.mcpPath')}</Label>
                  <p className="text-xs text-gray-500 mt-1 mb-2">
                    {formData.transport === 'stdio' ? t('create.mcpPathAutoStdio') : t('create.mcpPathHelp')}
                  </p>
                  <div className={`flex items-center gap-2 px-3 py-2 rounded-lg border border-gray-200 ${
                    formData.transport === 'stdio' ? 'bg-gray-100 cursor-not-allowed' : 'bg-white'
                  }`}>
                    <Link className="w-4 h-4 text-gray-400" />
                    <input
                      id="mcp_path"
                      type="text"
                      placeholder="/mcp"
                      value={formData.transport === 'stdio' ? '/mcp' : (formData.mcp_path || '/mcp')}
                      onChange={(e) => setFormData(prev => ({ ...prev, mcp_path: e.target.value }))}
                      disabled={formData.transport === 'stdio'}
                      className={`flex-1 bg-transparent text-sm focus:outline-none ${
                        formData.transport === 'stdio' ? 'text-gray-500 cursor-not-allowed' : ''
                      }`}
                    />
                  </div>
                </div>

                {formData.transport === 'sse' && (
                  <div>
                    <Label htmlFor="port" className="text-gray-700">{t('create.port')}</Label>
                    <p className="text-xs text-gray-500 mt-1 mb-2">{t('create.portHelp')}</p>
                    {detecting ? (
                    <Skeleton className="h-[38px]" />
                    ) : (
                    <div className="flex items-center gap-2 px-3 py-2 rounded-lg border border-gray-200 bg-white">
                      <Server className="w-4 h-4 text-gray-400" />
                      <input
                        id="port"
                        type="number"
                        min={1}
                        max={65535}
                        placeholder={String(formData.runtime === 'python' ? 8000 : (formData.runtime === 'go' || formData.runtime === 'rust') ? 8080 : 3000)}
                        value={formData.port ?? ''}
                        onChange={(e) => setFormData(prev => ({
                          ...prev,
                          port: e.target.value === '' ? undefined : Number(e.target.value),
                        }))}
                        className="flex-1 bg-transparent text-sm focus:outline-none"
                      />
                    </div>
                    )}
                  </div>
                )}

                <div>
                  <Label htmlFor="entry_command" className="text-gray-700">{t('create.entryCommand')}</Label>
                  <p className="text-xs text-gray-500 mt-1 mb-2">{t('create.entryCommandHelp')}</p>
                  {detecting ? (
                  <Skeleton className="h-[38px]" />
                  ) : (
                  <div className="flex items-center gap-2 px-3 py-2 rounded-lg border border-gray-200 bg-white">
                    <Terminal className="w-4 h-4 text-gray-400" />
                    <input
                      id="entry_command"
                      type="text"
                      placeholder="python server.py"
                      value={formData.entry_command || ''}
                      onChange={(e) => setFormData(prev => ({ ...prev, entry_command: e.target.value || undefined }))}
                      className="flex-1 bg-transparent text-sm focus:outline-none font-mono"
                    />
                  </div>
                  )}
                </div>

                <div>
                  <Label htmlFor="build_command" className="text-gray-700">{t('create.buildCommand')}</Label>
                  <p className="text-xs text-gray-500 mt-1 mb-2">{t('create.buildCommandHelp')}</p>
                  {detecting ? (
                  <Skeleton className="h-[38px]" />
                  ) : (
                  <div className="flex items-center gap-2 px-3 py-2 rounded-lg border border-gray-200 bg-white">
                    <Terminal className="w-4 h-4 text-gray-400" />
                    <input
                      id="build_command"
                      type="text"
                      placeholder="npm run build"
                      value={formData.build_command || ''}
                      onChange={(e) => setFormData(prev => ({ ...prev, build_command: e.target.value || undefined }))}
                      className="flex-1 bg-transparent text-sm focus:outline-none font-mono"
                    />
                  </div>
                  )}
                </div>
                </div>
                )}

                {/* Machine memory */}
                <MemorySelect
                  value={formData.memory_mb ?? DEFAULT_MEMORY_MB}
                  onChange={(mb) => setFormData((prev) => ({ ...prev, memory_mb: mb }))}
                  maxMemoryMb={maxMemoryMb}
                />

                {/* Auth Enabled Toggle */}
                <div className="pt-4 border-t border-gray-100">
                  <div className="flex items-start gap-3">
                    <button
                      type="button"
                      onClick={() => setFormData(prev => ({ ...prev, auth_enabled: !prev.auth_enabled }))}
                      className={`mt-0.5 relative w-10 h-5 rounded-full transition-colors duration-200 flex-shrink-0 ${
                        formData.auth_enabled ? 'bg-violet-500' : 'bg-[#d1d5db]'
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 w-4 h-4 bg-white rounded-full shadow-sm transition-transform duration-200 ${
                          formData.auth_enabled ? 'left-[22px]' : 'left-0.5'
                        }`}
                      />
                    </button>
                    <div className="flex-1">
                      <Label className="text-gray-700 cursor-pointer" onClick={() => setFormData(prev => ({ ...prev, auth_enabled: !prev.auth_enabled }))}>
                        {t('create.authEnabled')}
                      </Label>
                      <p className="text-xs text-gray-500 mt-1">{t('create.authEnabledHelp')}</p>
                      {!formData.auth_enabled && (
                        <div className="mt-2 px-3 py-2 rounded-lg bg-amber-50 border border-amber-200">
                          <p className="text-xs text-amber-700 flex items-start gap-2">
                            <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-0.5" />
                            {t('create.authDisabledWarning')}
                          </p>
                        </div>
                      )}
                    </div>
                  </div>
                </div>

                {/* Environment Variables */}
                <div className="pt-4 border-t border-gray-100">
                  <Label className="text-gray-700">{t('create.envVars')}</Label>
                  <p className="text-xs text-gray-500 mt-1 mb-3">{t('create.envVarsHelp')}</p>

                  {detecting && envVars.length === 0 && (
                    <div className="space-y-2 mb-3">
                      <div className="flex items-center gap-2">
                        <Skeleton className="h-[38px] w-1/3" />
                        <Skeleton className="h-[38px] flex-1" />
                      </div>
                      <div className="flex items-center gap-2">
                        <Skeleton className="h-[38px] w-1/3" />
                        <Skeleton className="h-[38px] flex-1" />
                      </div>
                    </div>
                  )}

                  {envVars.length > 0 && (
                    <div className="space-y-2 mb-3">
                      {envVars.map((env, index) => (
                        <div key={index} className="flex items-center gap-2">
                          <div className="flex items-center gap-2 px-3 py-2 rounded-lg border border-gray-200 bg-white w-1/3">
                            <KeyRound className="w-4 h-4 text-gray-400 flex-shrink-0" />
                            <input
                              type="text"
                              value={env.key}
                              onChange={(e) => updateEnvVar(index, 'key', e.target.value)}
                              placeholder="API_KEY"
                              className="flex-1 min-w-0 bg-transparent text-sm font-mono focus:outline-none"
                            />
                          </div>
                          <input
                            type="password"
                            value={env.value}
                            onChange={(e) => updateEnvVar(index, 'value', e.target.value)}
                            placeholder={t('create.envVarsValuePlaceholder')}
                            className="flex-1 min-w-0 px-3 py-2 rounded-lg border border-gray-200 bg-white text-sm font-mono focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
                          />
                          <button
                            type="button"
                            onClick={() => removeEnvVar(index)}
                            className="p-2 text-gray-400 hover:text-red-600 transition-colors flex-shrink-0"
                            title={tCommon('delete')}
                          >
                            <Trash2 className="w-4 h-4" />
                          </button>
                        </div>
                      ))}
                    </div>
                  )}

                  <button
                    type="button"
                    onClick={addEnvVar}
                    className="flex items-center gap-2 text-sm text-violet-600 hover:text-violet-700 transition-colors"
                  >
                    <Plus className="w-4 h-4" />
                    {t('create.envVarsAdd')}
                  </button>
                </div>
              </div>
          </div>
        </section>


        {/* Visibility Selection */}
        <section>
          <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-4">{t('create.visibility')}</h2>

          <div className="space-y-2">
            {visibilities.map((vis) => {
              const isSelected = formData.visibility === vis.value;
              return (
                <button
                  key={vis.value}
                  type="button"
                  onClick={() => setFormData(prev => ({ ...prev, visibility: vis.value as Visibility }))}
                  className={`w-full flex items-center gap-3 px-4 py-3 rounded-lg transition-all text-left ${
                    isSelected
                      ? ''
                      : 'hover:bg-gray-50'
                  }`}
                >
                  <div className={`w-5 h-5 rounded-full border-2 flex items-center justify-center flex-shrink-0 transition-all ${
                    isSelected
                      ? 'border-violet-500 bg-violet-500'
                      : 'border-gray-500'
                  }`}>
                    {isSelected && (
                      <div className="w-2 h-2 rounded-full bg-white" />
                    )}
                  </div>
                  <span className={`${isSelected ? 'text-violet-600' : 'text-gray-400'}`}>
                    {vis.icon}
                  </span>
                  <div className="flex-1">
                    <span className={`text-sm font-medium ${isSelected ? 'text-gray-900' : 'text-gray-700'}`}>
                      {vis.label}
                    </span>
                    <span className={`text-sm ml-2 ${isSelected ? 'text-gray-600' : 'text-gray-400'}`}>
                      - {vis.desc}
                    </span>
                  </div>
                </button>
              );
            })}
          </div>
        </section>

        {/* Error Message */}
        {createMutation.isError && (
          <div className="p-4 rounded-xl bg-red-50 border border-red-200">
            <div className="flex items-start gap-3">
              <div className="w-8 h-8 rounded-full bg-red-100 flex items-center justify-center flex-shrink-0">
                <XCircle className="w-4 h-4 text-red-600" />
              </div>
              <div>
                <p className="font-medium text-red-800">{errorMessage}</p>
                {errorSuggestion && (
                  <p className="text-sm text-red-600 mt-1">
                    {t('create.trySuggestion')} <code className="px-1.5 py-0.5 bg-red-100 rounded text-xs">{errorSuggestion}</code>
                  </p>
                )}
              </div>
            </div>
          </div>
        )}

        {/* Actions */}
        <div className="flex justify-end gap-2.5 pt-4 border-t border-gray-100">
          <Button
            type="button"
            variant="outline"
            onClick={() => router.back()}
            className="h-10 px-4 rounded-lg border-[#d1d5db] text-[#374151] text-sm font-medium hover:bg-[#f3f4f6] transition-colors duration-200"
          >
            {tCommon('cancel')}
          </Button>
          <Button
            type="submit"
            disabled={createMutation.isPending || !workspaceId || !formData.github_repo}
            className="h-10 px-4 rounded-lg bg-violet-500 hover:bg-violet-600 border border-violet-600 text-white text-sm font-medium gap-2 transition-colors duration-200"
          >
            {createMutation.isPending ? (
              <div className="w-4 h-4 border-2 rounded-full border-white/30 border-t-white animate-spin" />
            ) : (
              <>
                <Plus className="w-4 h-4" />
                {t('create.submit')}
              </>
            )}
          </Button>
        </div>
      </form>
    </div>
  );
}

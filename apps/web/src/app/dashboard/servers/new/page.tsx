'use client';

import { useState, useCallback, useMemo, useEffect, useRef } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useRouter, useSearchParams } from 'next/navigation';
import { useTranslations } from 'next-intl';
import { Lock, Users, Globe, Server, Link, Search, Folder, AlertCircle, GitBranch, Terminal, AlertTriangle, XCircle, Plus, Trash2, KeyRound, Loader2, ChevronDown, ChevronRight, Key, Rocket } from 'lucide-react';
import { api } from '@/lib/api';
import { getLinkedAccounts, getRepos, getBranches, LinkedGitHubAccount, inspectRepo, RepoDetection } from '@/lib/github-api';
import { CreateServerRequest, McpServer, Runtime, Visibility, GitHubRepo, UpstreamOAuthProvider, ServerTemplatePublic } from '@/types';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { GitHubAccountSelector } from '@/components/github/GitHubAccountSelector';
import { MemorySelect } from '@/components/servers/memory-select';
import { Select } from '@/components/ui/select';
import { ProviderSelect } from '@/components/servers/provider-select';
import { DEFAULT_MEMORY_MB, findPlanLimits } from '@/lib/plans';
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
  const searchParams = useSearchParams();
  const rawTemplateId = searchParams.get('template');
  const templateId = rawTemplateId && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(rawTemplateId)
    ? rawTemplateId
    : null;
  const queryClient = useQueryClient();

  useSetPageHeader(t('create.title'), <Server className="w-4 h-4" />);

  const { activeWorkspace } = useWorkspace();

  const workspaceId = activeWorkspace?.id;

  // Plan limits drive which memory sizes are selectable (Free is capped at 256MB).
  const { data: plans } = useQuery<{ plan: string; limits: { max_memory_mb: number } }[]>({
    queryKey: ['billing-plans'],
    queryFn: () => api.get('/billing/plans'),
  });

  const { data: upstreamProviders = [] } = useQuery<UpstreamOAuthProvider[]>({
    queryKey: ['upstream-oauth-providers'],
    queryFn: () => api.get('/oauth/upstream-providers'),
  });

  const { data: templateData } = useQuery<ServerTemplatePublic>({
    queryKey: ['template', templateId],
    queryFn: () => api.get<ServerTemplatePublic>(`/templates/${templateId}`),
    enabled: !!templateId,
    staleTime: Infinity,
  });
  const maxMemoryMb = findPlanLimits(plans, activeWorkspace?.plan)?.max_memory_mb ?? 256;

  // Linked GitHub accounts
  const { data: linkedAccounts, isLoading: accountsLoading } = useQuery<LinkedGitHubAccount[]>({
    queryKey: ['linked-github-accounts'],
    queryFn: getLinkedAccounts,
  });

  const [selectedAccountId, setSelectedAccountId] = useState<string | null>(null);
  const effectiveAccountId = selectedAccountId ?? linkedAccounts?.find(a => a.is_primary)?.id ?? linkedAccounts?.[0]?.id ?? null;

  // Fetch repos from selected account
  const { data: repos, isLoading: reposLoading } = useQuery<GitHubRepo[]>({
    queryKey: ['github-repos', effectiveAccountId],
    queryFn: () => getRepos(effectiveAccountId || undefined),
    enabled: !!effectiveAccountId,
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
  const [upstreamOauthEnabled, setUpstreamOauthEnabled] = useState(false);
  const [upstreamOauthProvider, setUpstreamOauthProvider] = useState('');
  const [upstreamOauthScopes, setUpstreamOauthScopes] = useState('');
  const [upstreamOauthAuthUrl, setUpstreamOauthAuthUrl] = useState('');
  const [upstreamOauthTokenUrl, setUpstreamOauthTokenUrl] = useState('');
  const [upstreamOauthClientId, setUpstreamOauthClientId] = useState('');
  const [upstreamOauthClientSecret, setUpstreamOauthClientSecret] = useState('');
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
  // Detected deploy settings and env vars are each folded away (Vercel-style) into their
  // own collapsible card below the Visibility section.
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [envOpen, setEnvOpen] = useState(false);
  const templateAppliedRef = useRef(false);

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
        effectiveAccountId || undefined,
      ),
    enabled: !!detectTarget,
    staleTime: 5 * 60_000,
  });

  useEffect(() => {
    if (detectQuery.data) applyDetection(detectQuery.data);
  }, [detectQuery.data, applyDetection]);

  // Warm the inspect cache ahead of selection (repo hover/focus, URL paste). Dedup is
  // automatic — React Query won't refire an in-flight or still-fresh key.
  const prefetchDetect = useCallback(
    (repo: string, branch?: string, subdir?: string) => {
      if (!repo) return;
      queryClient.prefetchQuery({
        queryKey: inspectKey(repo, branch, subdir),
        queryFn: () => inspectFn(repo, branch, subdir, effectiveAccountId || undefined),
        staleTime: 5 * 60_000,
      });
    },
    [queryClient, effectiveAccountId],
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

  // Pre-fill form from a shared template when ?template=<id> is in the URL.
  // Runs once when templateData loads; ref guards against re-application on re-render.
  useEffect(() => {
    if (!templateData || templateAppliedRef.current) return;
    templateAppliedRef.current = true;
    const slug = generateSlug(templateData.name);
    setFormData((prev) => ({
      ...prev,
      name: templateData.name,
      slug,
      description: templateData.description ?? '',
      github_repo: templateData.github_repo,
      github_branch: templateData.github_branch,
      runtime: templateData.runtime,
      transport: templateData.transport,
      mcp_path: templateData.mcp_path,
      entry_command: templateData.entry_command ?? undefined,
      build_command: templateData.build_command ?? undefined,
      root_directory: templateData.root_directory,
      auth_enabled: templateData.auth_enabled,
      memory_mb: templateData.memory_mb ?? DEFAULT_MEMORY_MB,
      port: templateData.port ?? undefined,
    }));
    if (templateData.upstream_oauth_provider) {
      setUpstreamOauthEnabled(true);
      setUpstreamOauthProvider(templateData.upstream_oauth_provider);
      setUpstreamOauthScopes(templateData.upstream_oauth_scopes.join(' '));
    }
    if (templateData.required_env_var_keys.length > 0) {
      setEnvVars(templateData.required_env_var_keys.map((k) => ({ key: k, value: '' })));
      setEnvOpen(true);
    }
    setPublicRepoUrl(`https://github.com/${templateData.github_repo}`);
    setSourceType('public-url');
    runDetect(templateData.github_repo, templateData.github_branch);
  }, [templateData, generateSlug, runDetect]);

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
    if (formData.name.length > 255 || (formData.description?.length ?? 0) > 1000) return;
    // Only send env vars that have a key; trim keys so stray whitespace doesn't
    // trip the backend's key validation.
    const env_vars = envVars
      .map((env) => ({ key: env.key.trim(), value: env.value }))
      .filter((env) => env.key.length > 0);
    const isCustomProvider = upstreamOauthProvider === 'custom';
    createMutation.mutate({
      ...formData,
      env_vars: env_vars.length > 0 ? env_vars : undefined,
      upstream_oauth_provider: upstreamOauthEnabled && upstreamOauthProvider ? upstreamOauthProvider : undefined,
      upstream_oauth_scopes: upstreamOauthEnabled && upstreamOauthProvider
        ? upstreamOauthScopes.split(/\s+/).filter(Boolean)
        : undefined,
      upstream_oauth_authorization_url: upstreamOauthEnabled && isCustomProvider ? upstreamOauthAuthUrl || undefined : undefined,
      upstream_oauth_token_url: upstreamOauthEnabled && isCustomProvider ? upstreamOauthTokenUrl || undefined : undefined,
      upstream_oauth_client_id: upstreamOauthEnabled && isCustomProvider ? upstreamOauthClientId || undefined : undefined,
      upstream_oauth_client_secret: upstreamOauthEnabled && isCustomProvider ? upstreamOauthClientSecret || undefined : undefined,
      template_id: templateId || undefined,
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

  // Branch dropdown. To keep the initial render and auto-detection fast, we DON'T fetch the
  // full branch list up front — the menu shows just the detected/default branch immediately.
  // The full list is fetched lazily the first time the user opens the dropdown (branchMenuOpen),
  // then merged into the options below.
  const [branchMenuOpen, setBranchMenuOpen] = useState(false);
  const { data: remoteBranches } = useQuery<string[]>({
    queryKey: ['github-branches', formData.github_repo, effectiveAccountId],
    queryFn: () => getBranches(formData.github_repo, effectiveAccountId || undefined),
    enabled: branchMenuOpen && !!formData.github_repo,
    staleTime: 5 * 60_000,
  });

  // Seeded with the currently-selected + default branch (always available, no fetch), then
  // extended with the lazily-fetched remote branches. `branchValue` always resolves to one of
  // these so the native <select> stays controlled without an out-of-range value warning.
  const branchOptions = useMemo(() => {
    const opts: string[] = [];
    for (const b of [formData.github_branch, selectedRepo?.default_branch, ...(remoteBranches ?? []), 'main']) {
      if (b && !opts.includes(b)) opts.push(b);
    }
    return opts;
  }, [formData.github_branch, selectedRepo, remoteBranches]);
  const branchValue =
    formData.github_branch && branchOptions.includes(formData.github_branch)
      ? formData.github_branch
      : branchOptions[0];

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

  const handleToggleOauth = () => {
    setUpstreamOauthEnabled((v) => !v);
    if (upstreamOauthEnabled) {
      setUpstreamOauthProvider('');
      setUpstreamOauthScopes('');
    }
  };

  const handleOauthProviderChange = (val: string) => {
    setUpstreamOauthProvider(val);
    const preset = upstreamProviders.find((p) => p.name === val);
    if (preset && !preset.is_managed) {
      setUpstreamOauthAuthUrl('');
      setUpstreamOauthTokenUrl('');
    }
    if (preset) setUpstreamOauthScopes(preset.default_scopes.join(' '));
  };

  return (
    <div className="max-w-2xl">
      {templateData && (
        <div className="mb-6 p-4 rounded-xl bg-violet-50 border border-violet-200 flex items-start gap-3">
          <Rocket className="w-5 h-5 text-violet-600 shrink-0 mt-0.5" />
          <div>
            <p className="text-sm font-semibold text-violet-900">{t('create.templateBannerTitle')}: <span className="font-normal">{templateData.name}</span></p>
            <p className="text-xs text-violet-700 mt-0.5">{t('create.templateBannerDesc')}</p>
            {templateData.required_env_var_keys.length > 0 && (
              <div className="flex flex-wrap gap-1.5 mt-2">
                {templateData.required_env_var_keys.map((k) => (
                  <span key={k} className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-amber-50 border border-amber-200 text-xs font-mono text-amber-700">
                    <Key className="w-2.5 h-2.5" />{k}
                  </span>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
      <form onSubmit={handleSubmit} className="space-y-8">
        {/* GitHub Repository Selection */}
        <section>
          <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-4">{t('create.githubRepo')}</h2>

          {/* Source Type Cards */}
          <div className="flex flex-wrap gap-3 mb-4">
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
              className={`flex items-center gap-2 px-4 py-2 rounded-xl border border-gray-300 transition-colors ${
                sourceType === 'my-repos'
                  ? 'bg-gray-100'
                  : 'bg-white hover:bg-gray-50'
              }`}
            >
              <SiGithub className="w-4 h-4 text-[#323232]" />
              <span className="font-medium text-sm text-[#323232]">
                {t('create.myRepos')}
              </span>
            </button>
            <button
              type="button"
              onClick={() => {
                setSourceType('public-url');
                setSelectedRepo(null);
                setFormData(prev => ({ ...prev, github_repo: '', name: '', slug: '' }));
              }}
              className={`flex items-center gap-2 px-4 py-2 rounded-xl border border-gray-300 transition-colors ${
                sourceType === 'public-url'
                  ? 'bg-gray-100'
                  : 'bg-white hover:bg-gray-50'
              }`}
            >
              <Link className="w-4 h-4 text-[#323232]" />
              <span className="font-medium text-sm text-[#323232]">
                {t('create.publicUrl')}
              </span>
            </button>
          </div>

          {sourceType === 'my-repos' ? (
            // My Repos Selection
            <div className="space-y-3">
              {/* GitHub Account Selector */}
              <div>
                <GitHubAccountSelector
                  accounts={linkedAccounts || []}
                  selectedAccountId={effectiveAccountId}
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
              <div className="flex items-center gap-2 h-10 w-full rounded-[10px] border border-input bg-background px-3 text-sm ring-offset-background focus-within:outline-none focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2 transition-colors">
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
              <Label htmlFor="name" className="text-xs">{t('create.name')}</Label>
              <Input
                id="name"
                placeholder={t('create.namePlaceholder')}
                value={formData.name}
                onChange={(e) => {
                  const name = e.target.value;
                  setFormData(prev => ({ ...prev, name, slug: generateSlug(name) }));
                }}
                required
                maxLength={255}
                className={`mt-2 ${formData.name.length > 255 ? 'border-red-500 focus-visible:ring-red-500' : ''}`}
              />
              {formData.name.length > 200 && (
                <p className={`text-xs mt-1 ${formData.name.length > 255 ? 'text-red-500' : 'text-gray-400'}`}>
                  {formData.name.length} / 255
                </p>
              )}
            </div>

            <div>
              <Label htmlFor="description" className="text-xs">{t('create.description')}</Label>
              <Input
                id="description"
                placeholder={t('create.descriptionBrief')}
                value={formData.description}
                onChange={(e) => setFormData(prev => ({ ...prev, description: e.target.value }))}
                maxLength={1000}
                className={`mt-2 ${(formData.description?.length ?? 0) > 1000 ? 'border-red-500 focus-visible:ring-red-500' : ''}`}
              />
              {(formData.description?.length ?? 0) > 800 && (
                <p className={`text-xs mt-1 ${(formData.description?.length ?? 0) > 1000 ? 'text-red-500' : 'text-gray-400'}`}>
                  {formData.description?.length ?? 0} / 1000
                </p>
              )}
            </div>

            <div className="space-y-4">

                {/* Machine memory */}
                <MemorySelect
                  value={formData.memory_mb ?? DEFAULT_MEMORY_MB}
                  onChange={(mb) => setFormData((prev) => ({ ...prev, memory_mb: mb }))}
                  maxMemoryMb={maxMemoryMb}
                  labelClassName="text-xs"
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
                      <Label className="text-xs cursor-pointer" onClick={() => setFormData(prev => ({ ...prev, auth_enabled: !prev.auth_enabled }))}>
                        {t('create.authEnabled')}
                      </Label>
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

                {/* Upstream OAuth Toggle */}
                <div className="pt-4 border-t border-gray-100">
                  <div className="flex items-start gap-3">
                    <button
                      type="button"
                      onClick={handleToggleOauth}
                      className={`mt-0.5 relative w-10 h-5 rounded-full transition-colors duration-200 flex-shrink-0 ${upstreamOauthEnabled ? 'bg-violet-500' : 'bg-[#d1d5db]'}`}
                    >
                      <span
                        className={`absolute top-0.5 w-4 h-4 bg-white rounded-full shadow-sm transition-transform duration-200 ${upstreamOauthEnabled ? 'left-[22px]' : 'left-0.5'}`}
                      />
                    </button>
                    <div className="flex-1">
                      <Label className="text-xs cursor-pointer" onClick={handleToggleOauth}>
                        {t('upstreamOauth.title')}
                      </Label>
                      <p className="text-xs text-gray-500">{t('upstreamOauth.toggleHelp')}</p>
                      {upstreamOauthEnabled && (
                        <div className="mt-3 space-y-3">
                          <div>
                            <Label className="text-xs mb-1 block">{t('upstreamOauth.providerLabel')}</Label>
                            <ProviderSelect
                              value={upstreamOauthProvider}
                              onChange={handleOauthProviderChange}
                              providers={upstreamProviders}
                              placeholder={t('upstreamOauth.providerPlaceholder')}
                            />
                            {upstreamProviders.find((p) => p.name === upstreamOauthProvider) && (
                              <span className={`mt-1 inline-block text-xs px-2 py-0.5 rounded-full ${upstreamProviders.find((p) => p.name === upstreamOauthProvider)?.is_managed ? 'bg-violet-100 text-violet-700' : 'bg-gray-100 text-gray-600'}`}>
                                {upstreamProviders.find((p) => p.name === upstreamOauthProvider)?.is_managed ? t('upstreamOauth.managedBadge') : t('upstreamOauth.customBadge')}
                              </span>
                            )}
                          </div>

                          {upstreamOauthProvider && (
                            <div className="space-y-1">
                              <Label className="text-xs">{t('upstreamOauth.scopesLabel')}</Label>
                              <p className="text-xs text-gray-500">{t('upstreamOauth.scopesHelp')}</p>
                              <Input
                                type="text"
                                placeholder={t('upstreamOauth.scopesPlaceholder')}
                                value={upstreamOauthScopes}
                                onChange={(e) => setUpstreamOauthScopes(e.target.value)}
                                className="font-mono text-xs mt-1"
                              />
                            </div>
                          )}

                          {upstreamOauthProvider === 'custom' && (
                            <div className="mt-3 p-3 rounded-lg border border-gray-200 bg-gray-50 space-y-3">
                              <p className="text-xs font-medium text-gray-700">{t('upstreamOauth.customSection')}</p>
                              <div>
                                <Label className="text-xs">{t('upstreamOauth.authUrlLabel')}</Label>
                                <Input type="url" placeholder={t('upstreamOauth.authUrlPlaceholder')} value={upstreamOauthAuthUrl} onChange={(e) => setUpstreamOauthAuthUrl(e.target.value)} className="mt-1 font-mono text-xs" />
                              </div>
                              <div>
                                <Label className="text-xs">{t('upstreamOauth.tokenUrlLabel')}</Label>
                                <Input type="url" placeholder={t('upstreamOauth.tokenUrlPlaceholder')} value={upstreamOauthTokenUrl} onChange={(e) => setUpstreamOauthTokenUrl(e.target.value)} className="mt-1 font-mono text-xs" />
                              </div>
                              <div>
                                <Label className="text-xs">{t('upstreamOauth.clientIdLabel')}</Label>
                                <Input type="text" placeholder={t('upstreamOauth.clientIdPlaceholder')} value={upstreamOauthClientId} onChange={(e) => setUpstreamOauthClientId(e.target.value)} className="mt-1 font-mono text-xs" />
                              </div>
                              <div>
                                <Label className="text-xs">{t('upstreamOauth.clientSecretLabel')}</Label>
                                <Input type="password" placeholder={t('upstreamOauth.clientSecretPlaceholder')} value={upstreamOauthClientSecret} onChange={(e) => setUpstreamOauthClientSecret(e.target.value)} className="mt-1 font-mono text-xs" />
                              </div>
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  </div>
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

        {/* Environment Variables */}
        <section>
          <div className="rounded-[10px] border border-input">
            <button
              type="button"
              onClick={() => setEnvOpen((o) => !o)}
              className="flex w-full items-center gap-1.5 p-4 text-sm font-medium text-gray-600 hover:text-gray-900 transition-colors"
            >
              <ChevronRight className={`w-4 h-4 transition-transform ${envOpen ? 'rotate-90' : ''}`} />
              {t('create.envVars')}
            </button>
            {envOpen && (
            <div className="space-y-4 border-t border-input p-4">
              {detecting && envVars.length === 0 && (
                <div className="space-y-2">
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
                <div className="space-y-2">
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
            )}
          </div>
        </section>

        {/* Build & deploy settings */}
        <section>
          <div className="rounded-[10px] border border-input">
            <button
              type="button"
              onClick={() => setAdvancedOpen((o) => !o)}
              className="flex w-full items-center gap-1.5 p-4 text-sm font-medium text-gray-600 hover:text-gray-900 transition-colors"
            >
              <ChevronRight className={`w-4 h-4 transition-transform ${advancedOpen ? 'rotate-90' : ''}`} />
              {t('create.advancedSettings')}
            </button>
                {advancedOpen && (
                <div className="space-y-4 border-t border-input p-4">
                <div>
                  <Label htmlFor="github_branch" className="text-xs">{t('create.branch')}</Label>
                  <div className="relative mt-2 w-full sm:w-56">
                    <GitBranch className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-gray-400" />
                    <select
                      id="github_branch"
                      value={branchValue}
                      onChange={(e) => setFormData(prev => ({ ...prev, github_branch: e.target.value }))}
                      // First interaction triggers the lazy fetch of the full branch list.
                      onMouseDown={() => setBranchMenuOpen(true)}
                      onFocus={() => setBranchMenuOpen(true)}
                      className="peer h-10 w-full appearance-none rounded-[10px] border border-input bg-background pl-9 pr-9 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                    >
                      {branchOptions.map((b) => (
                        <option key={b} value={b}>{b}</option>
                      ))}
                    </select>
                    <ChevronDown className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 text-gray-400 transition-colors peer-focus:text-gray-600" />
                  </div>
                </div>

                <div>
                  <Label className="text-xs">{t('create.runtime')}</Label>
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
                  <Label className="text-xs">{t('create.transport')}</Label>
                  {detecting ? (
                    <div className="flex flex-wrap gap-3 mt-2">
                      <Skeleton className="h-[42px] w-44" />
                      <Skeleton className="h-[42px] w-32" />
                    </div>
                  ) : (
                  <div className="flex flex-wrap gap-3 mt-2">
                    <button
                      type="button"
                      onClick={() => setFormData(prev => ({ ...prev, transport: 'sse' }))}
                      className={`flex items-center gap-2 px-4 py-2 rounded-xl border border-gray-300 transition-colors ${
                        formData.transport === 'sse' ? 'bg-gray-100' : 'bg-white hover:bg-gray-50'
                      }`}
                    >
                      <Globe className="w-4 h-4 text-[#323232]" />
                      <span className="font-medium text-sm text-[#323232]">Streamable HTTP</span>
                    </button>
                    <button
                      type="button"
                      onClick={() => setFormData(prev => ({ ...prev, transport: 'stdio', port: undefined }))}
                      className={`flex items-center gap-2 px-4 py-2 rounded-xl border border-gray-300 transition-colors ${
                        formData.transport === 'stdio' ? 'bg-gray-100' : 'bg-white hover:bg-gray-50'
                      }`}
                    >
                      <Terminal className="w-4 h-4 text-[#323232]" />
                      <span className="font-medium text-sm text-[#323232]">STDIO</span>
                    </button>
                  </div>
                  )}
                </div>

                <div>
                  <Label htmlFor="root_directory" className="text-xs">{t('create.rootDirectory')}</Label>
                  <div className="mt-2 flex items-center gap-2 h-10 w-full rounded-[10px] border border-input bg-background px-3 focus-within:outline-none focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2 transition-colors">
                    <Folder className="w-4 h-4 text-gray-400 flex-shrink-0" />
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
                  <Label htmlFor="mcp_path" className="text-xs">{t('create.mcpPath')}</Label>
                  <div className={`mt-2 flex items-center gap-2 h-10 w-full rounded-[10px] border border-input px-3 transition-colors ${
                    formData.transport === 'stdio' ? 'bg-gray-100 cursor-not-allowed' : 'bg-background focus-within:outline-none focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2'
                  }`}>
                    <Link className="w-4 h-4 text-gray-400 flex-shrink-0" />
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
                    <Label htmlFor="port" className="text-xs">{t('create.port')}</Label>
                    {detecting ? (
                    <Skeleton className="mt-2 h-10" />
                    ) : (
                    <div className="mt-2 flex items-center gap-2 h-10 w-full rounded-[10px] border border-input bg-background px-3 focus-within:outline-none focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2 transition-colors">
                      <Server className="w-4 h-4 text-gray-400 flex-shrink-0" />
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
                  <Label htmlFor="entry_command" className="text-xs">{t('create.entryCommand')}</Label>
                  {detecting ? (
                  <Skeleton className="mt-2 h-10" />
                  ) : (
                  <div className="mt-2 flex items-center gap-2 h-10 w-full rounded-[10px] border border-input bg-background px-3 focus-within:outline-none focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2 transition-colors">
                    <Terminal className="w-4 h-4 text-gray-400 flex-shrink-0" />
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
                  <Label htmlFor="build_command" className="text-xs">{t('create.buildCommand')}</Label>
                  {detecting ? (
                  <Skeleton className="mt-2 h-10" />
                  ) : (
                  <div className="mt-2 flex items-center gap-2 h-10 w-full rounded-[10px] border border-input bg-background px-3 focus-within:outline-none focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2 transition-colors">
                    <Terminal className="w-4 h-4 text-gray-400 flex-shrink-0" />
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
            onClick={() => router.back()}
            className="inline-flex items-center justify-center h-7 px-2.5 text-xs font-medium text-gray-700 bg-gray-100 border border-gray-300 rounded-[10px] hover:bg-gray-200 active:bg-gray-300 transition-colors"
          >
            {tCommon('cancel')}
          </Button>
          <Button
            type="submit"
            disabled={createMutation.isPending || !workspaceId || !formData.github_repo}
            className="inline-flex items-center justify-center gap-1.5 h-7 px-2.5 text-xs font-medium text-white bg-violet-600 border border-violet-700 rounded-[10px] hover:bg-violet-700 active:bg-violet-800 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            {createMutation.isPending ? (
              <div className="w-3.5 h-3.5 border-2 rounded-full border-white/30 border-t-white animate-spin" />
            ) : (
              <>
                <Plus className="w-3.5 h-3.5" />
                {t('create.submit')}
              </>
            )}
          </Button>
        </div>
      </form>
    </div>
  );
}

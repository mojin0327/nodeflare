'use client';

import { useMemo, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import { useRouter } from 'next/navigation';
import Link from 'next/link';
import { ChevronLeft, Aperture, Plus, Check, X, Search, ChevronDown } from 'lucide-react';
import { api } from '@/lib/api';
import { McpServerMinimal } from '@/types';
import { useWorkspace } from '@/hooks/use-workspace';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { useSetPageHeader } from '../../../page-header';

const COPY_FEEDBACK_DURATION_MS = 2000;

interface OAuthApp {
  id: string;
  client_id: string;
  client_secret?: string;
  client_name: string;
  redirect_uris: string[];
  server_id?: string;
  scopes: string[];
  created_at: string;
}

export default function NewOAuthAppPage() {
  const t = useTranslations('oauth');
  const tCommon = useTranslations('common');
  const tApiErrors = useTranslations('apiErrors');
  const router = useRouter();
  const queryClient = useQueryClient();

  useSetPageHeader(t('create.title'), <Aperture className="w-4 h-4" />);

  const { activeWorkspace } = useWorkspace();
  const workspaceId = activeWorkspace?.id;

  const [name, setName] = useState('');
  const [redirectUri, setRedirectUri] = useState('https://claude.ai/api/mcp/auth_callback');
  const [selectedServerId, setSelectedServerId] = useState<string | null>(null);
  const [selectedScopes, setSelectedScopes] = useState<string[]>(['*']);
  const [customScope, setCustomScope] = useState('');
  const [expiresInDays, setExpiresInDays] = useState<number | null>(30);
  const [serverSearchQuery, setServerSearchQuery] = useState('');
  const [isServerListOpen, setIsServerListOpen] = useState(false);

  // One-time secret
  const [newlyCreatedApp, setNewlyCreatedApp] = useState<OAuthApp | null>(null);
  const [copiedId, setCopiedId] = useState(false);
  const [copiedSecret, setCopiedSecret] = useState(false);

  // Fetch servers (minimal data only)
  const { data: servers } = useQuery<McpServerMinimal[]>({
    queryKey: ['servers-minimal', workspaceId],
    queryFn: () => api.get(`/workspaces/${workspaceId}/servers/minimal`),
    enabled: !!workspaceId,
  });

  // Filter servers for this workspace
  const workspaceServers = useMemo(
    () => servers?.filter(s => s.workspace_id === workspaceId) || [],
    [servers, workspaceId]
  );

  // Filter servers based on search query
  const filteredServers = useMemo(() => {
    const query = serverSearchQuery.toLowerCase();
    return workspaceServers.filter(
      (server) => server.name.toLowerCase().includes(query)
    );
  }, [workspaceServers, serverSearchQuery]);

  // Get selected server details
  const selectedServer = useMemo(
    () => selectedServerId && selectedServerId !== 'all'
      ? workspaceServers.find(s => s.id === selectedServerId)
      : null,
    [selectedServerId, workspaceServers]
  );

  const handleSelectServer = (serverId: string) => {
    setSelectedServerId(serverId);
    setServerSearchQuery('');
    setIsServerListOpen(false);
  };

  const SCOPE_OPTIONS = [
    { id: 'tools', label: 'Tools', scope: 'tools:*', desc: t('scopes.toolsDesc') },
    { id: 'resources', label: 'Resources', scope: 'resources:*', desc: t('scopes.resourcesDesc') },
    { id: 'prompts', label: 'Prompts', scope: 'prompts:*', desc: t('scopes.promptsDesc') },
  ];

  const createMutation = useMutation({
    mutationFn: async (data: { client_name: string; redirect_uris: string[]; server_id?: string; scopes: string[]; expires_in_days?: number }): Promise<OAuthApp> => {
      if (!workspaceId) throw new Error('No workspace found');
      return api.post(`/workspaces/${workspaceId}/oauth-apps`, data) as Promise<OAuthApp>;
    },
    onSuccess: (response) => {
      queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'oauth-apps'] });
      setNewlyCreatedApp(response);
    },
  });

  const toggleScope = (scope: string) => {
    if (scope === '*') {
      setSelectedScopes((prev) => prev.includes('*') ? [] : ['*']);
    } else {
      setSelectedScopes((prev) => {
        const filtered = prev.filter((s) => s !== '*');
        if (filtered.includes(scope)) {
          return filtered.filter((s) => s !== scope);
        } else {
          return [...filtered, scope];
        }
      });
    }
  };

  const addCustomScope = () => {
    if (customScope && !selectedScopes.includes(customScope)) {
      setSelectedScopes((prev) => {
        const filtered = prev.filter((s) => s !== '*');
        return [...filtered, customScope];
      });
      setCustomScope('');
    }
  };

  const removeScope = (scope: string) => {
    setSelectedScopes((prev) => {
      const result = prev.filter((s) => s !== scope);
      return result.length === 0 ? ['*'] : result;
    });
  };

  const createErrorMessage = useMemo(() => {
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
    return error?.message || tCommon('error');
  }, [createMutation.isError, createMutation.error, tApiErrors, tCommon]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedServerId) return;
    createMutation.mutate({
      client_name: name,
      redirect_uris: redirectUri ? [redirectUri] : [],
      server_id: selectedServerId === 'all' ? undefined : selectedServerId,
      scopes: selectedScopes,
      expires_in_days: expiresInDays ?? undefined,
    });
  };

  const handleCopyClientId = () => {
    if (newlyCreatedApp) {
      navigator.clipboard.writeText(newlyCreatedApp.client_id);
      setCopiedId(true);
      setTimeout(() => setCopiedId(false), COPY_FEEDBACK_DURATION_MS);
    }
  };

  const handleCopySecret = () => {
    if (newlyCreatedApp?.client_secret) {
      navigator.clipboard.writeText(newlyCreatedApp.client_secret);
      setCopiedSecret(true);
      setTimeout(() => setCopiedSecret(false), COPY_FEEDBACK_DURATION_MS);
    }
  };

  // Success view: one-time client_id / client_secret display (replaces the form)
  if (newlyCreatedApp) {
    return (
      <div className="max-w-2xl">
        <Link
          href="/dashboard/auth/oauth"
          className="inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700 mb-4 transition-colors"
        >
          <ChevronLeft className="w-4 h-4" />
          {tCommon('back')}
        </Link>

        <div className="p-5 rounded-2xl bg-gradient-to-r from-emerald-50 to-teal-50 border border-emerald-200">
          <div className="flex items-start gap-4">
            <div className="w-10 h-10 rounded-full flex items-center justify-center flex-shrink-0">
              <Check className="w-5 h-5 text-emerald-600" />
            </div>
            <div className="flex-1 min-w-0">
              <p className="font-medium text-emerald-800">{t('created.title')}</p>
              <p className="text-sm text-emerald-700 mt-1">{t('created.warning')}</p>

              <div className="mt-4 space-y-3">
                <div>
                  <label className="block text-xs font-medium text-emerald-700 mb-1">Client ID</label>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 px-3 py-2 bg-white rounded-lg border border-emerald-200 text-sm font-mono text-gray-800 truncate">
                      {newlyCreatedApp.client_id}
                    </code>
                    <Button
                      size="sm"
                      variant={copiedId ? "default" : "outline"}
                      className={copiedId ? "bg-emerald-600 hover:bg-emerald-600" : ""}
                      onClick={handleCopyClientId}
                    >
                      {copiedId ? "Copied!" : tCommon('copy')}
                    </Button>
                  </div>
                </div>
                <div>
                  <label className="block text-xs font-medium text-emerald-700 mb-1">Client Secret</label>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 px-3 py-2 bg-white rounded-lg border border-emerald-200 text-sm font-mono text-gray-800 truncate">
                      {newlyCreatedApp.client_secret}
                    </code>
                    <Button
                      size="sm"
                      variant={copiedSecret ? "default" : "outline"}
                      className={copiedSecret ? "bg-emerald-600 hover:bg-emerald-600" : ""}
                      onClick={handleCopySecret}
                    >
                      {copiedSecret ? "Copied!" : tCommon('copy')}
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>

        <div className="flex justify-end gap-2.5 pt-4 mt-6 border-t border-gray-100">
          <Button
            type="button"
            onClick={() => router.push('/dashboard/auth/oauth')}
            className="inline-flex items-center justify-center gap-1.5 h-7 px-2.5 text-xs font-medium text-white bg-gray-900 border border-gray-900 rounded-[10px] hover:bg-gray-800 active:bg-black disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            {tCommon('done')}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="max-w-2xl">
      <Link
        href="/dashboard/auth/oauth"
        className="inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700 mb-4 transition-colors"
      >
        <ChevronLeft className="w-4 h-4" />
        {tCommon('back')}
      </Link>

      <form onSubmit={handleSubmit} className="space-y-8">
        {/* Name */}
        <section>
          <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-4">{t('create.name')}</h2>
          <Input
            id="name"
            placeholder={t('create.namePlaceholder')}
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
          />
        </section>

        {/* Server */}
        <section>
          <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-4">{t('create.server')}</h2>
          <div className="relative">
            {/* Selected Server Display / Trigger */}
            <button
              type="button"
              onClick={() => setIsServerListOpen(!isServerListOpen)}
              className="w-full flex items-center justify-between px-4 py-3 rounded-[10px] border border-input bg-white hover:border-gray-300 transition-colors text-left"
            >
              {selectedServerId ? (
                <span className="font-medium text-gray-900">
                  {selectedServerId === 'all' ? t('create.allServers') : selectedServer?.name}
                </span>
              ) : (
                <span className="text-gray-400">{t('create.selectServer')}</span>
              )}
              <ChevronDown className={`w-5 h-5 text-gray-400 transition-transform ${isServerListOpen ? 'rotate-180' : ''}`} />
            </button>

            {/* Dropdown List */}
            {isServerListOpen && (
              <div className="absolute z-10 mt-2 w-full rounded-[10px] border border-input bg-white shadow-lg overflow-hidden">
                <div className="p-3 border-b border-gray-100">
                  <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-gray-50">
                    <Search className="w-4 h-4 text-gray-400" />
                    <input
                      type="text"
                      placeholder={t('create.searchServers')}
                      value={serverSearchQuery}
                      onChange={(e) => setServerSearchQuery(e.target.value)}
                      className="flex-1 bg-transparent text-sm focus:outline-none"
                    />
                  </div>
                </div>
                <div className="max-h-64 overflow-y-auto divide-y divide-gray-100">
                  {/* All Servers Option */}
                  <button
                    type="button"
                    onClick={() => handleSelectServer('all')}
                    className={`w-full flex items-center justify-between p-3 transition-colors text-left ${
                      selectedServerId === 'all'
                        ? 'bg-gray-100'
                        : 'hover:bg-gray-50'
                    }`}
                  >
                    <span className="font-medium text-gray-900">{t('create.allServers')}</span>
                    <div className="flex items-center gap-2 flex-shrink-0">
                      <span className="px-2 py-0.5 text-xs rounded-full bg-gray-100 text-gray-600">
                        {workspaceServers.length}
                      </span>
                      {selectedServerId === 'all' && (
                        <Check className="w-5 h-5 text-gray-900" />
                      )}
                    </div>
                  </button>

                  {/* Individual Servers */}
                  {filteredServers.length === 0 ? (
                    <div className="p-6 text-center text-gray-500">
                      <p className="text-sm">{serverSearchQuery ? t('create.noServersFound') : t('create.noServersAvailable')}</p>
                    </div>
                  ) : (
                    filteredServers.map((server) => (
                      <button
                        key={server.id}
                        type="button"
                        onClick={() => handleSelectServer(server.id)}
                        className={`w-full flex items-center justify-between p-3 transition-colors text-left ${
                          selectedServerId === server.id
                            ? 'bg-gray-100'
                            : 'hover:bg-gray-50'
                        }`}
                      >
                        <span className="font-medium text-gray-900 truncate">{server.name}</span>
                        {selectedServerId === server.id && (
                          <Check className="w-5 h-5 text-gray-900 flex-shrink-0" />
                        )}
                      </button>
                    ))
                  )}
                </div>
              </div>
            )}
          </div>

          <div className="mt-6">
            <Label htmlFor="redirectUri" className="text-xs">{t('create.redirectUri')}</Label>
            <Input
              id="redirectUri"
              placeholder="https://claude.ai/api/mcp/auth_callback"
              value={redirectUri}
              onChange={(e) => setRedirectUri(e.target.value)}
              className="mt-2"
            />
            <p className="text-xs text-gray-500 mt-2">{t('create.redirectUriHelp')}</p>
          </div>
        </section>

        {/* Scopes */}
        <section>
          <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-4">{t('scopes.title')}</h2>

          <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
            {/* Full Access */}
            <div
              onClick={() => toggleScope('*')}
              className="flex items-center gap-2 cursor-pointer select-none"
            >
              <span className="text-sm font-medium text-gray-700">{t('scopes.fullAccess')}</span>
              <div className={`w-11 h-6 rounded-full p-0.5 transition-colors ${
                selectedScopes.includes('*') ? 'bg-gray-900' : 'bg-gray-300'
              }`}>
                <div className={`w-5 h-5 rounded-full bg-white shadow-sm transition-transform duration-200 ${
                  selectedScopes.includes('*') ? 'translate-x-5' : 'translate-x-0'
                }`} />
              </div>
            </div>

            <div className="w-px h-6 bg-gray-200" />

            {/* Individual Scopes */}
            {SCOPE_OPTIONS.map((option) => {
              const isChecked = selectedScopes.includes(option.scope) || selectedScopes.includes('*');
              const isDisabled = selectedScopes.includes('*');
              return (
                <div
                  key={option.id}
                  onClick={() => !isDisabled && toggleScope(option.scope)}
                  className={`flex items-center gap-2 select-none ${isDisabled ? 'opacity-40 cursor-not-allowed' : 'cursor-pointer'}`}
                >
                  <span className="text-sm font-medium text-gray-700">{option.label}</span>
                  <div className={`w-11 h-6 rounded-full p-0.5 transition-colors ${
                    isChecked ? 'bg-gray-900' : 'bg-gray-300'
                  }`}>
                    <div className={`w-5 h-5 rounded-full bg-white shadow-sm transition-transform duration-200 ${
                      isChecked ? 'translate-x-5' : 'translate-x-0'
                    }`} />
                  </div>
                </div>
              );
            })}
          </div>

          <div className="mt-6">
            <Label htmlFor="customScope" className="text-xs">{t('customScope')}</Label>
            <div className="flex gap-2 mt-2">
              <Input
                id="customScope"
                placeholder="tools:call:specific_tool_name"
                value={customScope}
                onChange={(e) => setCustomScope(e.target.value)}
              />
              <Button type="button" variant="outline" onClick={addCustomScope}>
                {tCommon('add')}
              </Button>
            </div>
            <p className="text-xs text-gray-500 mt-2">{t('customScopeExamples')}</p>
          </div>

          {selectedScopes.length > 0 && !selectedScopes.includes('*') && (
            <div className="mt-6">
              <Label className="text-xs mb-2 block">{t('scopes.selected')}</Label>
              <div className="flex flex-wrap gap-1.5">
                {selectedScopes.map((scope) => (
                  <span
                    key={scope}
                    className="inline-flex items-center gap-1.5 px-2.5 py-1 text-sm bg-gray-100 text-gray-700 rounded-md"
                  >
                    <code className="text-xs font-mono">{scope}</code>
                    <button
                      type="button"
                      onClick={() => removeScope(scope)}
                      className="text-gray-400 hover:text-gray-600 transition-colors"
                    >
                      <X className="w-3.5 h-3.5" />
                    </button>
                  </span>
                ))}
              </div>
            </div>
          )}
        </section>

        {/* Expiration */}
        <section>
          <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-4">{tCommon('expiration')}</h2>
          <div className="relative w-full sm:w-64">
            <select
              value={expiresInDays === null ? 'never' : String(expiresInDays)}
              onChange={(e) => setExpiresInDays(e.target.value === 'never' ? null : Number(e.target.value))}
              className="peer h-10 w-full appearance-none rounded-[10px] border border-input bg-background pl-3 pr-9 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
            >
              <option value="1">{tCommon('expiry1d')}</option>
              <option value="7">{tCommon('expiry1w')}</option>
              <option value="30">{tCommon('expiry30d')}</option>
              <option value="never">{tCommon('expiryNever')}</option>
            </select>
            <ChevronDown className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 text-gray-400 transition-colors peer-focus:text-gray-600" />
          </div>
        </section>

        {/* Error Message */}
        {createMutation.isError && (
          <p className="text-sm text-red-600">{createErrorMessage}</p>
        )}

        {/* Actions */}
        <div className="flex justify-end gap-2.5 pt-4 border-t border-gray-100">
          <Button
            type="button"
            onClick={() => router.push('/dashboard/auth/oauth')}
            className="inline-flex items-center justify-center h-7 px-2.5 text-xs font-medium text-gray-700 bg-gray-100 border border-gray-300 rounded-[10px] hover:bg-gray-200 active:bg-gray-300 transition-colors"
          >
            {tCommon('cancel')}
          </Button>
          <Button
            type="submit"
            disabled={createMutation.isPending || !workspaceId || !selectedServerId}
            className="inline-flex items-center justify-center gap-1.5 h-7 px-2.5 text-xs font-medium text-white bg-gray-900 border border-gray-900 rounded-[10px] hover:bg-gray-800 active:bg-black disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
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

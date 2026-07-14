'use client';

import { useMemo, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import { useRouter } from 'next/navigation';
import { ChevronLeft, Aperture, Plus, Trash2 } from 'lucide-react';
import { api } from '@/lib/api';
import { McpServerMinimal } from '@/types';
import { useWorkspace } from '@/hooks/use-workspace';
import { Button } from '@/components/ui/button';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog';
import { formatSlashDate } from '@/lib/date-utils';
import { useSetPageHeader } from '../../page-header';

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

export default function OAuthAppsPage() {
  const t = useTranslations('oauth');
  const router = useRouter();
  useSetPageHeader(t('title'), <Aperture className="w-4 h-4" />);
  const { activeWorkspace, isLoading: isLoadingWorkspaces } = useWorkspace();
  const workspaceId = activeWorkspace?.id;

  const { data: oauthApps, isLoading: isLoadingApps } = useQuery<OAuthApp[]>({
    queryKey: ['workspaces', workspaceId, 'oauth-apps'],
    queryFn: () => api.get(`/workspaces/${workspaceId}/oauth-apps`),
    enabled: !!workspaceId,
  });

  const { data: servers } = useQuery<McpServerMinimal[]>({
    queryKey: ['servers-minimal', workspaceId],
    queryFn: () => api.get(`/workspaces/${workspaceId}/servers/minimal`),
    enabled: !!workspaceId,
  });

  // Create a map for quick server name lookup
  const serverMap = useMemo(
    () => new Map(servers?.map(s => [s.id, s.name]) || []),
    [servers]
  );

  const isLoading = isLoadingWorkspaces || isLoadingApps;

  return (
    <div>
      {/* Back Button + New */}
      <div className="flex items-center justify-between mb-4">
        <button
          onClick={() => router.push('/dashboard/auth')}
          className="flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700 transition-colors"
        >
          <ChevronLeft className="w-4 h-4" />
          {t('backToAuth')}
        </button>
        <Button
          size="sm"
          onClick={() => router.push('/dashboard/auth/oauth/new')}
          disabled={!workspaceId}
          className="h-7 text-xs px-2.5 bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white"
        >
          <Plus className="w-3.5 h-3.5 mr-1" />
          {t('new')}
        </Button>
      </div>

      {/* OAuth Apps List */}
      <div className="max-w-4xl">
        <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-4">{t('yourApps')}</h2>

        {isLoading ? (
          <div className="space-y-3">
            {[...Array(3)].map((_, i) => (
              <div key={i} className="h-20 bg-gray-100 animate-pulse rounded-xl" />
            ))}
          </div>
        ) : oauthApps && oauthApps.length > 0 ? (
          <div>
            {oauthApps.map((app, index) => (
              <OAuthAppRow
                key={app.id}
                app={app}
                workspaceId={workspaceId!}
                serverName={app.server_id ? serverMap.get(app.server_id) : undefined}
                t={t}
                isFirst={index === 0}
                isLast={index === oauthApps.length - 1}
              />
            ))}
          </div>
        ) : (
          <div className="py-16 text-center">
            <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-gray-100 flex items-center justify-center">
              <Aperture className="w-8 h-8 text-gray-400" />
            </div>
            <h2 className="text-lg font-medium text-gray-700 mb-2">{t('empty.title')}</h2>
            <p className="text-gray-500 max-w-md mx-auto">{t('empty.description')}</p>
          </div>
        )}
      </div>
    </div>
  );
}

function OAuthAppRow({
  app,
  workspaceId,
  serverName,
  t,
  isFirst,
  isLast,
}: {
  app: OAuthApp;
  workspaceId: string;
  serverName?: string;
  t: (key: string, values?: Record<string, string | number>) => string;
  isFirst: boolean;
  isLast: boolean;
}) {
  const queryClient = useQueryClient();
  const tCommon = useTranslations('common');
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [removing, setRemoving] = useState(false);

  const queryKey = useMemo(
    () => ['workspaces', workspaceId, 'oauth-apps'] as const,
    [workspaceId]
  );

  const deleteMutation = useMutation({
    mutationFn: () => api.delete(`/workspaces/${workspaceId}/oauth-apps/${app.id}`),
    // Optimistic removal: drop the app from the cache immediately, roll back on error.
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey });
      const previous = queryClient.getQueryData<OAuthApp[]>(queryKey);
      queryClient.setQueryData<OAuthApp[]>(queryKey, (old) =>
        old?.filter((x) => x.id !== app.id) ?? []
      );
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(queryKey, context.previous);
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey });
    },
  });

  const handleConfirm = () => {
    setConfirmOpen(false);
    setRemoving(true);
  };

  const onExitDone = (e: React.TransitionEvent<HTMLDivElement>) => {
    if (removing && e.propertyName === 'grid-template-rows') {
      deleteMutation.mutate();
    }
  };

  return (
    <div
      onTransitionEnd={onExitDone}
      className={`grid transition-all duration-300 ease-in-out ${
        removing ? 'grid-rows-[0fr] opacity-0 -translate-x-4' : 'grid-rows-[1fr]'
      }`}
    >
      <div className="overflow-hidden">
        <div
          className={`group flex items-center gap-4 px-4 py-2 bg-white border-x border-b border-gray-200 hover:bg-gray-50 transition-colors ${
            isFirst ? 'border-t rounded-t-lg' : ''
          } ${isLast ? 'rounded-b-lg' : ''}`}
        >
      {/* OAuth Icon */}
      <div className="w-8 h-8 flex items-center justify-center flex-shrink-0">
        <Aperture className="w-4 h-4 text-gray-500" />
      </div>

      {/* Main Content */}
      <div className="flex-1 min-w-0">
        {/* Top Row: Name + Created Date */}
        <div className="flex items-center justify-between">
          <span className="font-medium text-gray-900">{app.client_name}</span>
          <div className="flex items-center gap-3 text-xs text-gray-400">
            <span>{formatSlashDate(app.created_at)}{t('list.createdSuffix')}</span>
          </div>
        </div>
        {/* Bottom Row: Client ID + Server + Scopes */}
        <div className="flex items-center gap-2 mt-0.5">
          <code className="text-xs text-gray-400 font-mono">{app.client_id.slice(0, 16)}...</code>
          <span className="text-gray-300">•</span>
          <span className="px-2 py-0.5 text-xs bg-violet-50 text-violet-600 rounded">
            {serverName || t('create.allServers')}
          </span>
          <span className="text-gray-300">•</span>
          <div className="flex items-center gap-1">
            {app.scopes?.includes('*') ? (
              <span className="px-2 py-0.5 text-xs font-medium bg-violet-100 text-violet-700 rounded">
                {t('scopes.fullAccess')}
              </span>
            ) : (
              <>
                {app.scopes?.slice(0, 2).map((scope) => (
                  <span key={scope} className="px-2 py-0.5 text-xs font-mono bg-gray-100 text-gray-600 rounded">
                    {scope}
                  </span>
                ))}
                {app.scopes && app.scopes.length > 2 && (
                  <span className="px-1.5 py-0.5 text-xs text-gray-400">
                    +{app.scopes.length - 2}
                  </span>
                )}
              </>
            )}
          </div>
        </div>
      </div>

      {/* Delete Button */}
      <button
        onClick={() => setConfirmOpen(true)}
        disabled={deleteMutation.isPending || removing}
        className="p-2 text-gray-300 hover:text-red-600 hover:bg-red-50 rounded-md transition-colors"
        title={t('delete.title')}
      >
        <Trash2 className="w-4 h-4" />
      </button>

      {/* Confirm delete modal */}
      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent className="max-w-[calc(100%-2rem)] sm:max-w-md mx-4 sm:mx-auto">
          <AlertDialogHeader>
            <AlertDialogTitle>{t('delete.title')}</AlertDialogTitle>
            <AlertDialogDescription>{t('delete.confirm')}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{tCommon('cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleConfirm}
              className="bg-red-600 hover:bg-red-700"
            >
              {tCommon('delete')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
        </div>
      </div>
    </div>
  );
}

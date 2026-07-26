'use client';

import { useQueries } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import { api } from '@/lib/api';
import { findPlanLimits } from '@/lib/plans';
import { McpServerList } from '@/types';
import { useWorkspace } from '@/hooks/use-workspace';
import { mcpPublicUrl } from '@/lib/mcp-url';
import Link from 'next/link';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import { SiNodedotjs, SiPython, SiGo, SiRust, SiDocker } from 'react-icons/si';
import { Server, Plus, AlertCircle } from 'lucide-react';

interface Plan {
  plan: string;
  limits: {
    max_servers: number;
  };
}

export default function ServersPage() {
  const t = useTranslations('servers');

  const { activeWorkspace } = useWorkspace();

  // Fetch all data in parallel
  const [serversQuery, plansQuery] = useQueries({
    queries: [
      {
        queryKey: ['servers-list', activeWorkspace?.id],
        queryFn: () => api.get<McpServerList[]>(`/workspaces/${activeWorkspace!.id}/servers/summary`),
        enabled: !!activeWorkspace,
      },
      {
        queryKey: ['billing-plans'],
        queryFn: () => api.get<Plan[]>('/billing/plans'),
      },
    ],
  });

  const servers = serversQuery.data;
  const plans = plansQuery.data;
  const isLoading = serversQuery.isLoading;
  const isErrorServers = serversQuery.isError;

  const currentWorkspace = activeWorkspace;
  const currentPlanLimits = findPlanLimits(plans, currentWorkspace?.plan);
  const maxServers = currentPlanLimits?.max_servers || 3;
  const currentServerCount = isErrorServers ? 0 : (servers?.length || 0);
  const isAtLimit = !isErrorServers && currentServerCount >= maxServers;

  return (
    <div className="space-y-4 sm:space-y-6">
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        <div className="flex flex-col sm:flex-row sm:items-center gap-2 sm:gap-4">
          {/* Usage Badge */}
          <div className="flex items-center gap-2 px-2.5 sm:px-3 py-1 sm:py-1.5 rounded-lg bg-gray-100 text-xs sm:text-sm self-start">
            <Server className="w-3.5 h-3.5 sm:w-4 sm:h-4 text-gray-500" />
            <span className="text-gray-700">
              {t('usage', { current: currentServerCount, max: maxServers === 4294967295 ? '∞' : maxServers })}
            </span>
          </div>
        </div>
        <Link href="/dashboard/servers/new" className="self-start sm:self-auto">
          <Button size="sm" disabled={isAtLimit} className="h-7 text-xs px-2.5 bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white">
            <Plus className="w-3.5 h-3.5 mr-1" />
            {t('new')}
          </Button>
        </Link>
      </div>

      {/* Upgrade Banner when at limit */}
      {isAtLimit && currentWorkspace?.plan !== 'enterprise' && (
        <div className="flex items-center gap-3 text-sm text-gray-500">
          <div className="flex items-center gap-2">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-amber-400" />
            <span>{t('upgrade.serverLimit')}</span>
          </div>
          <Link
            href="/dashboard/billing"
            className="text-violet-600 hover:text-violet-700 font-medium hover:underline"
          >
            {t('upgrade.cta')} →
          </Link>
        </div>
      )}

      {isLoading ? (
        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-4">
          {[...Array(6)].map((_, i) => (
            <div key={i} className="rounded-lg border border-gray-200 p-6 animate-pulse">
              <div className="flex items-start justify-between mb-4">
                <div className="flex items-center gap-3">
                  <div className="w-9 h-9 bg-gray-200 rounded" />
                  <div>
                    <div className="h-4 w-28 bg-gray-200 rounded mb-1" />
                    <div className="h-3 w-20 bg-gray-100 rounded" />
                  </div>
                </div>
                <div className="h-6 w-20 bg-gray-100 rounded-full" />
              </div>
              <div className="space-y-2">
                <div className="h-3 w-full bg-gray-100 rounded" />
                <div className="h-3 w-3/4 bg-gray-100 rounded" />
              </div>
              <div className="mt-4 pt-4 border-t border-gray-200">
                <div className="h-3 w-full bg-gray-100 rounded" />
              </div>
            </div>
          ))}
        </div>
      ) : isErrorServers ? (
        <div className="flex flex-col items-center justify-center py-20">
          <AlertCircle className="w-12 h-12 text-red-400 mb-4" />
          <p className="text-muted-foreground mb-4">{t('loadError')}</p>
          <Button variant="outline" onClick={() => window.location.reload()}>
            {t('retry')}
          </Button>
        </div>
      ) : servers?.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-20">
          <Server className="w-12 h-12 text-gray-300 mb-4" />
          <p className="text-gray-400">{t('empty')}</p>
        </div>
      ) : (
        <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-4">
          {servers?.map((server) => (
            <ServerCard
              key={server.id}
              server={server}
              workspaceSlug={activeWorkspace?.slug}
              t={t}
            />
          ))}
        </div>
      )}
    </div>
  );
}

const runtimeStyles: Record<string, { icon: React.ReactNode; iconColor: string }> = {
  node: { icon: <SiNodedotjs className="w-5 h-5" />, iconColor: 'text-[#323232]' },
  python: { icon: <SiPython className="w-5 h-5" />, iconColor: 'text-[#323232]' },
  go: { icon: <SiGo className="w-6 h-6" />, iconColor: 'text-[#323232]' },
  rust: { icon: <SiRust className="w-5 h-5" />, iconColor: 'text-[#323232]' },
  docker: { icon: <SiDocker className="w-5 h-5" />, iconColor: 'text-[#323232]' },
};

const statusColors: Record<string, string> = {
  running: 'bg-green-500',
  building: 'bg-yellow-500',
  deploying: 'bg-blue-500',
  stopped: 'bg-gray-500',
  failed: 'bg-red-500',
  inactive: 'bg-gray-500',
};

function ServerCard({ server, workspaceSlug, t }: { server: McpServerList; workspaceSlug?: string; t: (key: string) => string }) {
  const runtime = runtimeStyles[server.runtime] || runtimeStyles.node;

  return (
    <Link href={`/dashboard/servers/${server.id}`}>
      <Card className="hover:shadow-md hover:scale-[1.02] transition-all cursor-pointer bg-card border border-gray-200 rounded-[8px] text-[#323232]">
        <CardContent className="p-6">
          <div className="flex items-start justify-between mb-4">
            <div className="flex items-center gap-3">
              <div className="p-2">
                <span className={runtime.iconColor}>{runtime.icon}</span>
              </div>
              <div>
                <h3 className="font-semibold text-[#323232]">{server.name}</h3>
                <p className="text-sm text-gray-500">{server.slug}</p>
              </div>
            </div>
            <div className="flex items-center gap-2 px-2 py-1 rounded-full bg-gray-100">
              <div
                className={`w-2 h-2 rounded-full ${
                  statusColors[server.status] ?? statusColors.inactive
                }`}
              />
              <span className="text-xs text-[#323232] font-medium">
                {t(`status.${server.status}`)}
              </span>
            </div>
          </div>

          <div className="space-y-2 text-sm">
            <div className="flex items-center text-[#323232]">
              <span className="mr-2 text-gray-500">{t('repo')}:</span>
              <span className="truncate">{server.github_repo}</span>
            </div>
            <div className="flex items-center text-[#323232]">
              <span className="mr-2 text-gray-500">{t('visibility')}:</span>
              <span className="capitalize">{server.visibility}</span>
            </div>
          </div>

          <div className="mt-4 pt-4 border-t border-gray-200">
            {server.endpoint_url && workspaceSlug ? (
              <code className="text-xs text-gray-600 break-all">
                {mcpPublicUrl(workspaceSlug, server.slug)}
              </code>
            ) : (
              <span className="text-xs text-gray-400 italic">{t('noEndpoint')}</span>
            )}
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

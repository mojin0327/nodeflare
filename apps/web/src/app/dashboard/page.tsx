'use client';

import { useQueries, useQuery } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import { api } from '@/lib/api';
import { McpServerBasic } from '@/types';
import { useWorkspace } from '@/hooks/use-workspace';
import { PatternAvatar } from '@/components/ui/pattern-avatar';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useEffect, useMemo, useState } from 'react';
import { LayoutDashboard, Plus, AlertCircle, ChevronRight, Play } from 'lucide-react';

interface BatchStatsResponse {
  servers: {
    server_id: string;
    total_requests: number;
    error_count: number;
  }[];
}

// Constants
const STATS_STALE_TIME_MS = 60 * 1000; // 1 minute
const PULSE_DURATION_MS = 1000;
const PULSE_INTERVAL_MS = 3000;
const DEFAULT_MAX_SERVERS = 3;
const DEFAULT_MAX_REQUESTS = 10000;

// Static style mappings (extracted to avoid re-creation on each render)
const STATUS_COLORS: Record<string, string> = {
  running: 'bg-green-500',
  building: 'bg-yellow-500 animate-pulse',
  deploying: 'bg-blue-500 animate-pulse',
  stopped: 'bg-gray-400',
  failed: 'bg-red-500',
  inactive: 'bg-gray-400',
};


// Pure function - no need for useCallback
function formatDate(dateStr: string): string {
  return dateStr.split('T')[0];
}

interface Plan {
  plan: string;
  limits: {
    max_servers: number;
    max_deployments_per_month: number;
    max_requests_per_month: number;
  };
}

export default function DashboardPage() {
  const t = useTranslations('dashboard');
  const tServers = useTranslations('servers');
  const tBilling = useTranslations('billing');
  const router = useRouter();

  const { activeWorkspace, isLoading: workspacesLoading } = useWorkspace();

  // Fetch all independent data in parallel using useQueries
  const [serversQuery, plansQuery] = useQueries({
    queries: [
      {
        queryKey: ['servers-basic', activeWorkspace?.id],
        queryFn: () => api.get<McpServerBasic[]>(`/workspaces/${activeWorkspace!.id}/servers/basic`),
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
  const isLoadingServers = serversQuery.isLoading;
  const isSuccessServers = serversQuery.isSuccess;
  const isErrorServers = serversQuery.isError;

  // 必要なデータの初期ローディング中（いずれかがロード中ならローディング表示）
  const isInitialLoading = serversQuery.isLoading || workspacesLoading || plansQuery.isLoading;

  const runningServers = servers?.filter((s) => s.status === 'running') ?? [];
  const currentWorkspace = activeWorkspace;
  const currentPlan = plans?.find(p => p.plan === (currentWorkspace?.plan || 'free'));

  // Fetch stats for all servers in batch (single request instead of N requests)
  const { data: batchStats, isLoading: isLoadingStats } = useQuery<BatchStatsResponse>({
    queryKey: ['workspaces', currentWorkspace?.id, 'batch-stats'],
    queryFn: () => api.get<BatchStatsResponse>(`/workspaces/${currentWorkspace?.id}/stats`),
    enabled: !!currentWorkspace?.id,
    staleTime: STATS_STALE_TIME_MS,
  });

  const aggregatedStats = useMemo(() => {
    let totalRequests = 0;
    let totalErrors = 0;

    if (batchStats?.servers) {
      batchStats.servers.forEach((serverStats) => {
        totalRequests += serverStats.total_requests;
        totalErrors += serverStats.error_count;
      });
    }

    const errorRate = totalRequests > 0 ? (totalErrors / totalRequests) * 100 : 0;
    const uptime = runningServers.length > 0 && servers ? (runningServers.length / servers.length) * 100 : 0;
    return { totalRequests, totalErrors, errorRate, uptime, isLoadingStats };
  }, [batchStats, runningServers.length, servers, isLoadingStats]);

  // 初期ローディング
  if (isInitialLoading) {
    return (
      <div className="flex items-center justify-center min-h-[50vh]">
        <div className="w-8 h-8 border-4 rounded-full border-gray-200 border-t-violet-600 animate-spin" />
      </div>
    );
  }

  const maxServers = currentPlan?.limits?.max_servers || DEFAULT_MAX_SERVERS;
  const maxRequests = currentPlan?.limits?.max_requests_per_month || DEFAULT_MAX_REQUESTS;
  const serverUsage = Math.min((servers?.length || 0) / maxServers * 100, 100);
  const requestUsage = Math.min(aggregatedStats.totalRequests / maxRequests * 100, 100);

  return (
    <div>
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-end gap-3 mb-6">
        <div className="flex items-center gap-2 px-3 sm:px-4 py-1.5 sm:py-2 rounded-full bg-gray-800 text-xs sm:text-sm border border-gray-700 self-start sm:self-auto">
          <span className="capitalize text-white font-medium">{currentWorkspace?.plan || 'free'}</span>
          <span className="text-gray-600">|</span>
          <Link href="/dashboard/billing" className="text-violet-400 hover:text-violet-300 font-medium">
            {tBilling('upgrade')}
          </Link>
        </div>
      </div>

      {/* Stats Row */}
      <div className="flex flex-wrap items-center gap-3 sm:gap-6 mb-6 text-xs sm:text-sm">
        <div className="flex items-center gap-2">
          <span className={`w-2 h-2 rounded-full ${isErrorServers ? 'bg-gray-400' : 'bg-green-500'}`} />
          <span className="text-gray-500">
            {isErrorServers ? '--' : t('runningCount', { running: runningServers.length, total: servers?.length || 0 })}
          </span>
        </div>
        <div className="text-gray-300 hidden sm:block">|</div>
        <div className="text-gray-500">
          <span className="text-gray-900 font-medium">
            {isErrorServers ? '--' : aggregatedStats.totalRequests.toLocaleString()}
          </span> {t('requests')}
        </div>
        <div className="text-gray-300 hidden sm:block">|</div>
        <div className="text-gray-500">
          <span className={aggregatedStats.totalErrors > 0 ? 'text-red-600 font-medium' : 'text-gray-900 font-medium'}>
            {isErrorServers ? '--' : aggregatedStats.totalErrors}
          </span> {t('errors')}
        </div>
        <div className="flex-1 hidden sm:block" />
        <Link href="/dashboard/servers/new" className="flex items-center gap-1.5 text-violet-600 hover:text-violet-700 ml-auto sm:ml-0">
          <Plus className="w-4 h-4" />
          <span className="hidden sm:inline">{t('newServer')}</span>
          <span className="sm:hidden">New</span>
        </Link>
      </div>

      {/* Servers */}
      <div className="rounded-xl border border-gray-200 overflow-hidden">
        <div className="bg-gray-50 px-4 py-2 border-b border-gray-200 flex items-center justify-between">
          <span className="text-xs font-medium text-gray-500 uppercase tracking-wide">{t('servers')}</span>
          <Link href="/dashboard/servers" className="text-xs text-gray-400 hover:text-gray-600">
            {t('viewAll')}
          </Link>
        </div>
        <div className="divide-y divide-gray-100">
          {isLoadingServers ? (
            <div className="flex items-center justify-center py-8">
              <div className="w-6 h-6 border-[3px] rounded-full border-gray-200 border-t-violet-600 animate-spin" />
            </div>
          ) : isErrorServers ? (
            <div className="flex flex-col items-center justify-center py-8 text-center">
              <AlertCircle className="w-8 h-8 text-red-400 mb-2" />
              <p className="text-sm text-gray-500">{t('serversLoadError')}</p>
              <button
                onClick={() => window.location.reload()}
                className="mt-2 text-xs text-violet-600 hover:text-violet-700"
              >
                {t('retry')}
              </button>
            </div>
          ) : (
            servers?.slice(0, 6).map((server) => (
              <ServerStatusRow key={server.id} server={server} t={tServers} />
            ))
          )}
        </div>
      </div>

      {/* Plan Usage */}
      <div className="mt-6 rounded-xl border border-gray-200 overflow-hidden">
        <div className="bg-gray-50 px-4 py-2 border-b border-gray-200 flex items-center justify-between">
          <span className="text-xs font-medium text-gray-500 uppercase tracking-wide">{t('planUsage')}</span>
          <Link href="/dashboard/billing" className="text-xs text-gray-400 hover:text-gray-600">
            {t('manage')}
          </Link>
        </div>
        <div className="flex flex-col sm:flex-row gap-4 sm:gap-8 p-4 bg-white">
          <div className="flex-1">
            <div className="flex items-baseline justify-between mb-2">
              <span className="text-sm text-gray-600">{t('servers')}</span>
              <span className="text-sm">
                <span className="font-semibold text-gray-900">{isErrorServers ? '--' : (servers?.length || 0)}</span>
                <span className="text-gray-400"> / {maxServers === 4294967295 ? '∞' : maxServers}</span>
              </span>
            </div>
            <div className="h-1.5 bg-gray-100 rounded-full overflow-hidden">
              <div
                className="h-full bg-gradient-to-r from-violet-400 to-violet-500 rounded-full"
                style={{ width: isErrorServers ? '0%' : (maxServers === 4294967295 ? '0%' : `${serverUsage}%`) }}
              />
            </div>
          </div>
          <div className="flex-1">
            <div className="flex items-baseline justify-between mb-2">
              <span className="text-sm text-gray-600">{t('requests')}</span>
              <span className="text-sm">
                <span className="font-semibold text-gray-900">{isErrorServers ? '--' : aggregatedStats.totalRequests.toLocaleString()}</span>
                <span className="text-gray-400"> / {maxRequests === 4294967295 ? '∞' : maxRequests.toLocaleString()}</span>
              </span>
            </div>
            <div className="h-1.5 bg-gray-100 rounded-full overflow-hidden">
              <div
                className="h-full bg-gradient-to-r from-emerald-400 to-emerald-500 rounded-full"
                style={{ width: isErrorServers ? '0%' : (maxRequests === 4294967295 ? '0%' : `${requestUsage}%`) }}
              />
            </div>
          </div>
        </div>
      </div>

      {/* News & Updates */}
      <NewsSection t={t} />
    </div>
  );
}


function ServerStatusRow({
  server,
  t,
}: {
  server: McpServerBasic;
  t: (key: string) => string;
}) {
  const [pulse, setPulse] = useState(false);

  useEffect(() => {
    if (server.status === 'running') {
      const interval = setInterval(() => {
        setPulse(true);
        setTimeout(() => setPulse(false), PULSE_DURATION_MS);
      }, PULSE_INTERVAL_MS);
      return () => clearInterval(interval);
    }
  }, [server.status]);

  return (
    <Link
      href={`/dashboard/servers/${server.id}`}
      className="group flex items-center gap-3 px-4 py-3 bg-white hover:bg-gray-50 transition-colors"
    >
      <div className="relative w-8 h-8 flex-shrink-0">
        <PatternAvatar
          seed={server.id}
          rounded="rounded-md"
          className="w-8 h-8"
        />
        {server.status === 'running' && (
          <span className={`absolute -top-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-green-500 border-2 border-white ${pulse ? 'animate-ping' : ''}`} />
        )}
      </div>
      <div className="flex-1 min-w-0">
        <span className="text-sm font-medium text-gray-900">{server.name}</span>
      </div>
      <div className="flex items-center gap-3 text-sm text-gray-400">
        <span className="hidden sm:inline capitalize">{server.runtime}</span>
        <div className="flex items-center gap-1.5">
          <span className={`w-1.5 h-1.5 rounded-full ${STATUS_COLORS[server.status]}`} />
          <span className="hidden sm:inline">{t(`status.${server.status}`)}</span>
        </div>
        <ChevronRight className="w-4 h-4 text-gray-300 group-hover:text-gray-400 transition-colors" />
      </div>
    </Link>
  );
}

interface Announcement {
  id: string;
  title: string;
  content?: string;
  type: string;
  published_at: string;
}

interface BlogPost {
  id: string;
  title: string;
  slug: string;
  publishDate?: string;
}

interface VideoItem {
  id: string;
  title: string;
  url: string;
  thumbnail: string;
}

function NewsSection({ t }: { t: (key: string) => string }) {
  // Fetch announcements from API
  const { data: announcements } = useQuery<Announcement[]>({
    queryKey: ['announcements'],
    queryFn: () => api.get('/announcements?limit=5'),
    staleTime: STATS_STALE_TIME_MS,
  });

  // Fetch blog posts from CMS
  const { data: blogPosts } = useQuery<BlogPost[]>({
    queryKey: ['dashboard-blog-posts'],
    queryFn: async () => {
      const res = await fetch('/api/blog');
      if (!res.ok) return [];
      return res.json();
    },
    staleTime: STATS_STALE_TIME_MS,
  });

  // YouTube videos - memoized to prevent recreation on each render
  const videos = useMemo<VideoItem[]>(() => [
    {
      id: '1',
      title: t('video1Title'),
      url: 'https://www.youtube.com/shorts/Pf0S4v6QsCY',
      thumbnail: 'https://img.youtube.com/vi/Pf0S4v6QsCY/mqdefault.jpg',
    },
    {
      id: '2',
      title: t('video2Title'),
      url: 'https://www.youtube.com/shorts/9zOwAuSYOTM',
      thumbnail: 'https://img.youtube.com/vi/9zOwAuSYOTM/mqdefault.jpg',
    },
  ], [t]);

  const messages = announcements || [];
  const blogs = blogPosts?.slice(0, 3) || [];

  return (
    <div className="mt-6 space-y-5">
      {/* Messages */}
      {messages.length > 0 && (
        <div className="space-y-1">
          {messages.map((item) => (
            <div key={item.id} className="text-sm text-gray-500">
              <span className="text-gray-400 mr-2">{formatDate(item.published_at)}</span>
              {item.title}
            </div>
          ))}
        </div>
      )}

      {/* Videos */}
      {videos.length > 0 && (
        <div>
          <div className="text-xs font-medium text-gray-400 uppercase tracking-wide mb-2">{t('videos')}</div>
          <div className="flex gap-3 overflow-x-auto pb-1">
            {videos.map((item) => (
              <a
                key={item.id}
                href={item.url}
                target="_blank"
                rel="noopener noreferrer"
                className="flex-shrink-0 group"
              >
                <div className="relative w-36 h-20 rounded-lg overflow-hidden bg-gray-100">
                  <img src={item.thumbnail} alt={item.title} className="w-full h-full object-cover" />
                  <div className="absolute inset-0 bg-black/20 flex items-center justify-center group-hover:bg-black/30 transition-colors">
                    <div className="w-8 h-8 rounded-full bg-white/90 flex items-center justify-center">
                      <Play className="w-3 h-3 text-gray-900 ml-0.5 fill-current" />
                    </div>
                  </div>
                </div>
                <p className="text-xs text-gray-500 mt-1 line-clamp-1 w-36">{item.title}</p>
              </a>
            ))}
          </div>
        </div>
      )}

      {/* Blogs */}
      {blogs.length > 0 && (
        <div>
          <div className="text-xs font-medium text-gray-400 uppercase tracking-wide mb-2">{t('blog')}</div>
          <div className="flex gap-3 overflow-x-auto">
            {blogs.map((post) => (
              <a
                key={post.id}
                href={`/blog/${post.slug}`}
                className="flex-shrink-0 p-3 rounded-lg border border-gray-100 hover:border-gray-200 transition-colors"
              >
                <p className="text-sm text-gray-900 w-44 line-clamp-2">{post.title}</p>
                {post.publishDate && (
                  <span className="text-xs text-gray-400 mt-1 block">{formatDate(post.publishDate)}</span>
                )}
              </a>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

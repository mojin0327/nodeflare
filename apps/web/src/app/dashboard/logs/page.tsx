'use client';

import { useQuery } from '@tanstack/react-query';
import { useTranslations, useLocale } from 'next-intl';
import { AlertCircle, AlertTriangle, FileText, Search, Pause, Play, RefreshCw, Download } from 'lucide-react';
import { api } from '@/lib/api';
import { RequestLog, PaginatedResponse, McpServerBasic } from '@/types';
import { useWorkspace } from '@/hooks/use-workspace';
import { useState, useMemo, useCallback } from 'react';
import { Button } from '@/components/ui/button';
import { jsPDF } from 'jspdf';
import autoTable from 'jspdf-autotable';

// Constants
const LIVE_REFETCH_INTERVAL_MS = 2000;

type StatusFilter = 'all' | 'success' | 'error';
type TimeFilter = 'all' | '1h' | '24h' | '7d' | '30d';

// Helper to parse status code from response_status string
const parseStatusCode = (status: string): number => {
  const match = status.match(/\d+/);
  return match ? parseInt(match[0], 10) : 0;
};

export default function LogsPage() {
  const t = useTranslations('logs');
  const tCommon = useTranslations('common');
  const locale = useLocale();
  const [page, setPage] = useState(1);
  const [selectedServerId, setSelectedServerId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [statusFilter] = useState<StatusFilter>('all');
  const [timeFilter] = useState<TimeFilter>('all');
  const [isLive, setIsLive] = useState(true);

  const { activeWorkspace } = useWorkspace();

  const { data: servers, isLoading: isLoadingServers, isError: isErrorServers } = useQuery<McpServerBasic[]>({
    queryKey: ['servers-basic', activeWorkspace?.id],
    queryFn: () => api.get(`/workspaces/${activeWorkspace!.id}/servers/basic`),
    enabled: !!activeWorkspace,
  });

  const selectedServer = servers?.find((s) => s.id === selectedServerId)
    || servers?.find((s) => s.status === 'running')
    || servers?.[0];
  const workspaceId = selectedServer?.workspace_id;
  const serverId = selectedServer?.id;

  // Backend enforces free plan restriction (1h limit)
  const buildQueryParams = () => {
    const params = new URLSearchParams();
    params.set('page', page.toString());
    params.set('per_page', '50');
    if (statusFilter !== 'all') params.set('status', statusFilter);
    if (timeFilter !== 'all') params.set('time_range', timeFilter);
    if (searchQuery) params.set('search', searchQuery);
    return params.toString();
  };

  const { data, isLoading: isLoadingLogs, isError: isErrorLogs, refetch } = useQuery<PaginatedResponse<RequestLog>>({
    queryKey: ['workspaces', workspaceId, 'servers', serverId, 'logs', page, statusFilter, timeFilter, searchQuery],
    queryFn: () => api.get(`/workspaces/${workspaceId}/servers/${serverId}/logs?${buildQueryParams()}`),
    enabled: !!workspaceId && !!serverId,
    refetchInterval: isLive ? LIVE_REFETCH_INTERVAL_MS : false,
  });

  // Timeline range
  const timeRange = useMemo(() => {
    if (!data?.data?.length) return null;
    const times = data.data.map(l => new Date(l.created_at).getTime());
    const min = Math.min(...times);
    const max = Math.max(...times);
    return { min, max };
  }, [data?.data]);

  const formatTime = useCallback((date: string) => {
    const d = new Date(date);
    const month = d.toLocaleString(locale, { month: 'short' }).toUpperCase();
    const day = d.getDate().toString().padStart(2, ' ');
    const time = d.toTimeString().split(' ')[0];
    const ms = d.getMilliseconds().toString().padStart(2, '0').slice(0, 2);
    return { month, day, time, ms };
  }, [locale]);

  const downloadPdf = useCallback(() => {
    if (!data?.data?.length) return;

    try {
      const doc = new jsPDF();
      const serverName = selectedServer?.name || 'Unknown Server';
      const now = new Date();
      const dateStr = now.toLocaleString();

      // Title
      doc.setFontSize(18);
      doc.text(t('pdf.title'), 14, 20);
      doc.setFontSize(11);
      doc.text(`${t('pdf.server')}: ${serverName}`, 14, 30);
      doc.text(`${t('pdf.generated')}: ${dateStr}`, 14, 37);
      doc.text(`${t('pdf.totalLogs')}: ${data.data.length}`, 14, 44);

      // Table data
      const tableData = data.data.map((log) => {
        const time = formatTime(log.created_at);
        return [
          `${time.month} ${time.day} ${time.time}.${time.ms}`,
          log.response_status,
          `${log.duration_ms}ms`,
          log.tool_name || '-',
        ];
      });

      autoTable(doc, {
        head: [[t('table.time'), t('table.status'), t('table.duration'), t('table.tool')]],
        body: tableData,
        startY: 52,
        styles: { fontSize: 8 },
        headStyles: { fillColor: [139, 92, 246] },
        columnStyles: {
          0: { cellWidth: 50 },
          1: { cellWidth: 40 },
          2: { cellWidth: 30 },
          3: { cellWidth: 60 },
        },
      });

      doc.save(`logs-${serverName}-${now.toISOString().split('T')[0]}.pdf`);
    } catch {
      // Silently fail - user will notice if download doesn't work
    }
  }, [data?.data, selectedServer?.name, formatTime, t]);

  if (isLoadingServers) {
    return (
      <div className="space-y-2">
        {[...Array(10)].map((_, i) => (
          <div key={i} className="h-8 bg-gray-100 animate-pulse rounded" />
        ))}
      </div>
    );
  }

  if (isErrorServers) {
    return (
      <div className="py-20 text-center">
        <AlertCircle className="w-12 h-12 text-red-400 mx-auto mb-4" />
        <p className="text-gray-500 mb-4">{t('loadError')}</p>
        <button
          onClick={() => window.location.reload()}
          className="text-sm text-violet-600 hover:text-violet-700"
        >
          {tCommon('retry')}
        </button>
      </div>
    );
  }

  if (!servers || servers.length === 0) {
    return (
      <div className="max-w-6xl">
        <div className="flex flex-col items-center justify-center py-20">
          <FileText className="w-12 h-12 text-gray-300 mb-4" />
          <p className="text-gray-400">{t('noServers')}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="max-w-6xl">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center space-x-4">
          {servers.length > 0 && (
            <div className="relative">
              <div className={`absolute left-3 top-1/2 -translate-y-1/2 w-2 h-2 rounded-full ${selectedServer?.status === 'running' ? 'bg-emerald-500' : selectedServer?.status === 'stopped' ? 'bg-gray-400' : 'bg-amber-500'}`} />
              <select
                className="pl-7 pr-10 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-200 rounded-lg focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent cursor-pointer appearance-none hover:border-gray-300 transition-colors"
                value={selectedServer?.id || ''}
                onChange={(e) => { setSelectedServerId(e.target.value); setPage(1); }}
              >
                {servers.map((server) => (
                  <option key={server.id} value={server.id}>{server.name}</option>
                ))}
              </select>
              <div className="absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none">
                <svg className="w-4 h-4 text-gray-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M6 9l6 6 6-6" />
                </svg>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Toolbar */}
      <div className="flex items-center gap-2 mb-4">
        <div className="flex-1 relative">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400" />
          <input
            type="text"
            placeholder={t('search')}
            value={searchQuery}
            onChange={(e) => { setSearchQuery(e.target.value); setPage(1); }}
            className="w-full pl-10 pr-4 py-2 text-sm border border-gray-200 rounded-lg focus:outline-none focus:border-gray-300"
          />
        </div>
        <button
          onClick={() => setIsLive(!isLive)}
          className={`flex items-center gap-2 px-3 py-2 rounded-lg border text-gray-600 hover:border-gray-300 transition-colors ${
            isLive ? 'border-violet-400 bg-violet-50' : 'border-gray-200'
          }`}
          title={isLive ? t('tooltip.stopAutoRefresh') : t('tooltip.startAutoRefresh')}
        >
          {isLive ? (
            <Pause className="w-4 h-4 text-violet-600" />
          ) : (
            <Play className="w-4 h-4" />
          )}
          <span className={`text-sm font-medium ${isLive ? 'text-violet-600' : ''}`}>{t('live')}</span>
          {isLive && <span className="w-2 h-2 rounded-full bg-violet-500 animate-pulse" />}
        </button>
        <button
          onClick={() => refetch()}
          className="p-2 border border-gray-200 rounded-lg hover:border-gray-300"
          title={t('tooltip.refresh')}
        >
          <RefreshCw className="w-5 h-5 text-gray-500" />
        </button>
        <button
          onClick={downloadPdf}
          disabled={!data?.data?.length}
          className="p-2 border border-gray-200 rounded-lg hover:border-gray-300 disabled:opacity-50 disabled:cursor-not-allowed"
          title={t('tooltip.downloadPdf')}
        >
          <Download className="w-5 h-5 text-gray-500" />
        </button>
      </div>

      {/* Timeline scrubber */}
      {timeRange && (
        <div className="mb-6 px-4">
          <div className="relative h-8">
            <div className="absolute inset-x-0 bottom-0 h-px bg-gray-200" />
            <div className="absolute left-0 top-0 text-xs text-gray-400">
              {new Date(timeRange.min).toLocaleTimeString()}
            </div>
            <div className="absolute left-1/2 top-0 -translate-x-1/2 text-xs text-gray-400">
              {new Date((timeRange.min + timeRange.max) / 2).toLocaleTimeString()}
            </div>
            <div className="absolute right-0 top-0 text-xs text-gray-400">
              {new Date(timeRange.max).toLocaleTimeString()}
            </div>
          </div>
        </div>
      )}

      {/* Table header */}
      <div className="grid grid-cols-[200px_120px_100px_1fr] gap-2 px-4 py-2 text-xs font-medium text-gray-500 border-b border-gray-200">
        <div>{t('table.time')}</div>
        <div>{t('table.status')}</div>
        <div>{t('table.duration')}</div>
        <div>{t('table.tool')}</div>
      </div>

      {/* Logs */}
      {isLoadingLogs ? (
        <div className="space-y-1 py-2">
          {[...Array(12)].map((_, i) => (
            <div key={i} className="h-10 bg-gray-50 animate-pulse rounded" />
          ))}
        </div>
      ) : isErrorLogs ? (
        <div className="py-16 text-center">
          <AlertCircle className="w-8 h-8 text-red-400 mx-auto mb-2" />
          <p className="text-gray-500 text-sm">{tCommon('error')}</p>
          <button onClick={() => refetch()} className="mt-2 text-xs text-violet-600 hover:text-violet-700">
            {tCommon('retry')}
          </button>
        </div>
      ) : !data?.data?.length ? (
        <div className="py-16 text-center text-gray-400 text-sm">
          {t('noLogs')}
        </div>
      ) : (
        <div className="divide-y divide-gray-100">
          {data.data.map((log) => {
            const time = formatTime(log.created_at);
            const statusCode = parseStatusCode(log.response_status);
            const isError = statusCode >= 400;

            return (
              <div
                key={log.id}
                className={`grid grid-cols-[200px_120px_100px_1fr] gap-2 px-4 py-2.5 text-sm hover:bg-gray-50 ${
                  isError ? 'bg-orange-50/50' : ''
                }`}
              >
                {/* Time */}
                <div className="flex items-center gap-2">
                  {isError && (
                    <AlertTriangle className="w-4 h-4 text-orange-500 flex-shrink-0" />
                  )}
                  <span className={`font-mono ${isError ? 'text-orange-600' : 'text-gray-500'}`}>
                    {time.month} {time.day}
                  </span>
                  <span className={`font-mono ${isError ? 'text-orange-700' : 'text-gray-900'}`}>
                    {time.time}.{time.ms}
                  </span>
                </div>

                {/* Status */}
                <div className="flex items-center font-mono">
                  <span className={
                    statusCode >= 500 ? 'text-red-600' :
                    statusCode >= 400 ? 'text-orange-500' :
                    statusCode >= 200 ? 'text-emerald-600' :
                    'text-gray-600'
                  }>
                    {log.response_status}
                  </span>
                </div>

                {/* Duration */}
                <div className="text-gray-500 font-mono text-xs flex items-center">
                  {log.duration_ms}ms
                </div>

                {/* Tool */}
                <div className="truncate">
                  {log.tool_name ? (
                    <span className="text-violet-600 font-medium">{log.tool_name}</span>
                  ) : (
                    <span className="text-gray-300">-</span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Pagination */}
      {data && data.total > 50 && (
        <div className="mt-4 flex items-center justify-between text-sm border-t border-gray-200 pt-4">
          <span className="text-gray-400 text-xs">
            {(page - 1) * 50 + 1}-{Math.min(page * 50, data.total)} of {data.total}
          </span>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" disabled={page === 1} onClick={() => setPage(p => p - 1)}>
              {tCommon('previous')}
            </Button>
            <Button variant="outline" size="sm" disabled={page * 50 >= data.total} onClick={() => setPage(p => p + 1)}>
              {tCommon('next')}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

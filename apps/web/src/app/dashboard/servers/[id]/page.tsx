'use client';

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useParams, useRouter } from 'next/navigation';
import { useTranslations } from 'next-intl';
import Link from 'next/link';
import { api } from '@/lib/api';
import { McpServer, Deployment, Secret, AccessMode, UpstreamOAuthProvider, ServerTemplate, PublishTemplateRequest } from '@/types';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import { createPortal } from 'react-dom';
import { toast } from 'sonner';
import { useServerStatusWebSocket, useDeploymentWebSocket } from '@/hooks/use-websocket';
import { AlertCircle, Server, Boxes, Github, Trash2, AlertTriangle, ExternalLink, Copy, ChevronRight, Check, Send, Plus, Key, Lock, Play, Rocket, Globe, Webhook, Settings, Eye, EyeOff, RefreshCw, Clipboard, X, Wrench, CheckCircle, Link2, BarChart3, HelpCircle, Share2, Pencil, Save } from 'lucide-react';
import {
  AreaChart,
  Area,
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from 'recharts';
import { BuildLogsPanel } from '@/components/deployment/build-logs-panel';
import { MemorySelect } from '@/components/servers/memory-select';
import { Select } from '@/components/ui/select';
import { ProviderSelect } from '@/components/servers/provider-select';
import { JazzAvatar } from '@/components/ui/jazz-avatar';
import { DEFAULT_MEMORY_MB, findPlanLimits } from '@/lib/plans';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog';
import { useSetPageHeader } from '../../page-header';
import { useWorkspace } from '@/hooks/use-workspace';
import { mcpPublicUrl } from '@/lib/mcp-url';
import { formatDateTime } from '@/lib/date-utils';

// Static status colors - moved outside component to prevent recreation
const STATUS_COLORS: Record<string, { bg: string; text: string; dot: string }> = {
  running: { bg: 'bg-green-50', text: 'text-green-700', dot: 'bg-green-500' },
  building: { bg: 'bg-yellow-50', text: 'text-yellow-700', dot: 'bg-yellow-500' },
  deploying: { bg: 'bg-blue-50', text: 'text-blue-700', dot: 'bg-blue-500' },
  stopped: { bg: 'bg-gray-50', text: 'text-gray-700', dot: 'bg-gray-400' },
  failed: { bg: 'bg-red-50', text: 'text-red-700', dot: 'bg-red-500' },
  inactive: { bg: 'bg-gray-50', text: 'text-gray-700', dot: 'bg-gray-400' },
};

const DEPLOYMENT_STATUS_COLORS: Record<string, { bg: string; text: string }> = {
  success: { bg: 'bg-green-100', text: 'text-green-700' },
  succeeded: { bg: 'bg-green-100', text: 'text-green-700' },
  building: { bg: 'bg-yellow-100', text: 'text-yellow-700' },
  deploying: { bg: 'bg-blue-100', text: 'text-blue-700' },
  failed: { bg: 'bg-red-100', text: 'text-red-700' },
  pending: { bg: 'bg-gray-100', text: 'text-gray-700' },
};

// Relative time formatter - moved outside to prevent recreation
function getRelativeTime(dateStr: string | null | undefined, t: (key: string) => string): string {
  if (!dateStr) return '-';
  const date = new Date(dateStr);
  if (isNaN(date.getTime())) return '-';

  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60000);
  const diffHours = Math.floor(diffMins / 60);
  const diffDays = Math.floor(diffHours / 24);
  const diffWeeks = Math.floor(diffDays / 7);

  if (diffMins < 1) return t('detail.justNow');
  if (diffMins < 60) return `${diffMins}${t('detail.minutesAgo')}`;
  if (diffHours < 24) return `${diffHours}${t('detail.hoursAgo')}`;
  if (diffDays < 7) return `${diffDays}${t('detail.daysAgo')}`;
  return `${diffWeeks}${t('detail.weeksAgo')}`;
}

interface DeploymentUsage {
  deployments_this_month: number;
  max_deployments: number;
}

export default function ServerDetailPage() {
  const t = useTranslations('servers');
  const tCommon = useTranslations('common');
  const tErrors = useTranslations('errors');
  const tApiErrors = useTranslations('apiErrors');
  const params = useParams();
  const router = useRouter();
  const queryClient = useQueryClient();
  const serverId = params.id as string;
  const [activeTab, setActiveTab] = useState<'deployments' | 'test' | 'secrets' | 'webhooks' | 'metrics' | 'settings'>('deployments');
  const [autoSelectDeploymentId, setAutoSelectDeploymentId] = useState<string | null>(null);
  const [showDeployInfo, setShowDeployInfo] = useState(false);
  const [deleteConfirmName, setDeleteConfirmName] = useState('');
  const [deployError, setDeployError] = useState<string | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [shareDialogOpen, setShareDialogOpen] = useState(false);
  const [shareTemplateName, setShareTemplateName] = useState('');
  const [shareTemplateDesc, setShareTemplateDesc] = useState('');
  const [shareSelectedKeys, setShareSelectedKeys] = useState<string[]>([]);

  const { activeWorkspace, workspaces, isLoading: isLoadingWorkspaces } = useWorkspace();

  const serverQuery = useQuery({
    queryKey: ['server', activeWorkspace?.id, serverId],
    queryFn: () => api.get<McpServer>(`/workspaces/${activeWorkspace!.id}/servers/${serverId}`),
    enabled: !!activeWorkspace && !!serverId,
    // Poll during active deployments as fallback when WS is unavailable
    refetchInterval: (query) => {
      const s = query.state.data as McpServer | undefined;
      return s?.status === 'building' || s?.status === 'deploying' ? 3000 : false;
    },
  });

  const server = serverQuery.data;
  const isLoadingServers = serverQuery.isLoading || isLoadingWorkspaces;
  const isErrorServers = serverQuery.isError;

  const workspaceId = activeWorkspace?.id;

  useSetPageHeader(server?.name ?? t('title'), <Boxes className="w-4 h-4" />);

  const deploymentsQuery = useQuery({
    queryKey: ['servers', serverId, 'deployments'],
    queryFn: () => api.get<Deployment[]>(`/workspaces/${workspaceId}/servers/${serverId}/deployments`),
    enabled: !!workspaceId,
    // Poll during active deployments as fallback when WS is unavailable
    refetchInterval: (query) => {
      const data = query.state.data as Deployment[] | undefined;
      return data?.some(d => ['pending', 'building', 'deploying', 'pushing'].includes(d.status))
        ? 3000
        : false;
    },
  });

  const deployments = deploymentsQuery.data;

  // Find the active deployment ID for WebSocket subscription
  const activeDeploymentId = useMemo(
    () => deployments?.find(
      d => d.status === 'pending' || d.status === 'building' || d.status === 'deploying' || d.status === 'pushing'
    )?.id ?? '',
    [deployments]
  );

  const refetchDeployments = deploymentsQuery.refetch;

  // Track deployment toast for notifications
  const deploymentToastIdRef = useRef<string | number | null>(null);
  const pendingDeploymentIdRef = useRef<string | null>(null);

  // Single source of truth: resolve toast when the tracked deployment reaches a terminal state.
  // Both the WS path (via refetchDeployments) and background polling feed into `deployments`.
  useEffect(() => {
    if (!deploymentToastIdRef.current || !pendingDeploymentIdRef.current) return;
    const deployment = deployments?.find(d => d.id === pendingDeploymentIdRef.current);
    if (!deployment) return;
    if (deployment.status === 'succeeded') {
      toast.success(t('deploy.success'), { id: deploymentToastIdRef.current });
    } else if (deployment.status === 'failed' || deployment.status === 'cancelled') {
      toast.error(t('deploy.failed'), { id: deploymentToastIdRef.current });
    } else {
      return;
    }
    deploymentToastIdRef.current = null;
    pendingDeploymentIdRef.current = null;
  }, [deployments, t]);

  // WS triggers a refetch to feed fresh data into the effect above
  useDeploymentWebSocket(activeDeploymentId, {
    onStatusUpdate: () => { refetchDeployments(); },
  });

  // Fetch secrets
  const secretsQuery = useQuery({
    queryKey: ['servers', serverId, 'secrets'],
    queryFn: () => api.get<Secret[]>(`/workspaces/${workspaceId}/servers/${serverId}/secrets`),
    enabled: !!workspaceId,
  });
  const secrets = secretsQuery.data;

  // Fetch deployment usage (count from backend instead of filtering all deployments)
  const deploymentUsageQuery = useQuery({
    queryKey: ['workspaces', workspaceId, 'deployments', 'usage'],
    queryFn: () => api.get<DeploymentUsage>(`/workspaces/${workspaceId}/deployments/usage`),
    enabled: !!workspaceId,
  });
  const deploymentUsage = deploymentUsageQuery.data;

  // Real-time server status via WebSocket
  useServerStatusWebSocket(
    workspaceId || '',
    serverId,
    {
      onStatusUpdate: (status) => {
        queryClient.setQueryData<McpServer>(['server', activeWorkspace?.id, serverId], (old) =>
          old
            ? { ...old, status: status.status, endpoint_url: status.endpoint_url || old.endpoint_url }
            : old
        );
      },
    }
  );

  // Use backend-provided deployment usage
  const deploymentsThisMonth = deploymentUsage?.deployments_this_month || 0;
  const maxDeployments = deploymentUsage?.max_deployments || 50;
  const isAtDeployLimit = deploymentsThisMonth >= maxDeployments;
  const currentWorkspace = workspaces?.find(w => w.id === workspaceId);

  const deployMutation = useMutation({
    mutationFn: () => api.post<Deployment>(`/workspaces/${workspaceId}/servers/${serverId}/deploy`),
    onMutate: () => {
      // Show loading toast when deployment starts
      deploymentToastIdRef.current = toast.loading(t('deploy.started'));
    },
    onSuccess: (data: Deployment) => {
      setDeployError(null);
      pendingDeploymentIdRef.current = data.id;
      // Immediately add the new deployment to the list so BuildLogsPanel mounts right away
      queryClient.setQueryData<Deployment[]>(
        ['servers', serverId, 'deployments'],
        (old) => [data, ...(old ?? [])],
      );
      queryClient.invalidateQueries({ queryKey: ['server', activeWorkspace?.id, serverId] });
      queryClient.invalidateQueries({ queryKey: ['servers', serverId, 'deployments'] });
      queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'deployments', 'usage'] });
      setActiveTab('deployments');
      setAutoSelectDeploymentId(data.id);
    },
    onError: (error: any) => {
      if (deploymentToastIdRef.current) {
        toast.dismiss(deploymentToastIdRef.current);
        deploymentToastIdRef.current = null;
      }
      pendingDeploymentIdRef.current = null;
      const errorCode = error?.code;
      if (errorCode) {
        try {
          const translated = tApiErrors(errorCode);
          if (translated && translated !== errorCode) {
            setDeployError(translated);
            toast.error(translated);
            return;
          }
        } catch {
          // Translation not found
        }
      }
      const errorMsg = error?.message || tCommon('error');
      setDeployError(errorMsg);
      toast.error(errorMsg);
    },
  });

  const SERVERS_LIST_KEY = ['servers-list', activeWorkspace?.id] as const;
  const deleteMutation = useMutation({
    mutationFn: () => api.delete(`/workspaces/${workspaceId}/servers/${serverId}`),
    // Optimistic: drop the server from the list and navigate away immediately, so the
    // UI feels instant; the actual teardown (and Fly app destruction) runs in the
    // background. Roll back and surface a toast if the delete fails.
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: SERVERS_LIST_KEY });
      const previous = queryClient.getQueryData<Array<{ id: string }>>(SERVERS_LIST_KEY);
      queryClient.setQueryData<Array<{ id: string }>>(SERVERS_LIST_KEY, (old) =>
        old?.filter((s) => s.id !== serverId)
      );
      router.push('/dashboard/servers');
      return { previous };
    },
    onError: (error: any, _vars, context) => {
      if (context?.previous) {
        queryClient.setQueryData(SERVERS_LIST_KEY, context.previous);
      }
      let message = error?.message || tCommon('error');
      const errorCode = error?.code;
      if (errorCode) {
        try {
          const translated = tApiErrors(errorCode);
          if (translated && translated !== errorCode) message = translated;
        } catch {
          // Translation not found
        }
      }
      toast.error(message);
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: SERVERS_LIST_KEY });
    },
  });

  // Fetch existing template for this server (to show published state)
  const { data: existingTemplate, refetch: refetchTemplate } = useQuery<ServerTemplate | null>({
    queryKey: ['server-template', workspaceId, serverId],
    queryFn: () => api.get<ServerTemplate | null>(`/workspaces/${workspaceId}/servers/${serverId}/template`),
    enabled: !!workspaceId && !!serverId,
  });

  // Fetch secrets so user can pick required env var keys
  const { data: serverSecrets = [] } = useQuery<Secret[]>({
    queryKey: ['secrets', workspaceId, serverId],
    queryFn: () => api.get<Secret[]>(`/workspaces/${workspaceId}/servers/${serverId}/secrets`),
    enabled: !!workspaceId && !!serverId && shareDialogOpen,
  });

  // Auto-select all secrets when opening the dialog for a brand-new template
  useEffect(() => {
    if (!shareDialogOpen || existingTemplate || serverSecrets.length === 0) return;
    setShareSelectedKeys(serverSecrets.map((s) => s.key));
  }, [shareDialogOpen, serverSecrets, existingTemplate]);

  const publishTemplateMutation = useMutation({
    mutationFn: (body: PublishTemplateRequest) =>
      api.post<ServerTemplate>(
        `/workspaces/${workspaceId}/servers/${serverId}/template`,
        body,
      ),
    onSuccess: () => {
      toast.success(t('detail.shareSuccess'));
      setShareDialogOpen(false);
      refetchTemplate();
    },
    onError: (error: any) => {
      toast.error(error?.message || t('detail.shareError'));
    },
  });

  const unpublishTemplateMutation = useMutation({
    mutationFn: () =>
      api.delete(`/workspaces/${workspaceId}/servers/${serverId}/template`),
    onSuccess: () => {
      toast.success(t('detail.unshareSuccess'));
      refetchTemplate();
    },
    onError: (error: any) => {
      toast.error(error?.message || t('detail.shareError'));
    },
  });

  const handleShareClick = () => {
    if (!server) return;
    if (server.visibility !== 'public') {
      console.error(
        `[NodeFlare] Share blocked: server "${server.name}" (${serverId}) has visibility="${server.visibility}". Only public servers can be shared as templates.`,
      );
      toast.error(t('detail.sharePublicOnly'));
      return;
    }
    setShareTemplateName(existingTemplate?.name ?? server.name);
    setShareTemplateDesc(existingTemplate?.description ?? server.description ?? '');
    setShareSelectedKeys(existingTemplate?.required_env_var_keys ?? []);
    setShareDialogOpen(true);
  };

  if (isLoadingServers) {
    return (
      <div className="max-w-5xl space-y-4 sm:space-y-6">
        {/* Header: avatar + name/repo | buttons */}
        <div className="flex flex-col sm:flex-row sm:items-start justify-between gap-4">
          <div className="flex items-center gap-3 sm:gap-4">
            <div className="w-11 h-11 bg-gray-200 rounded-full animate-pulse flex-shrink-0" />
            <div>
              <div className="h-6 w-48 bg-gray-200 rounded animate-pulse mb-1.5" />
              <div className="h-4 w-64 bg-gray-100 rounded animate-pulse" />
            </div>
          </div>
          <div className="flex items-center gap-2 self-start sm:self-auto">
            <div className="h-9 w-36 bg-gray-200 rounded animate-pulse" />
            <div className="w-9 h-9 bg-gray-100 rounded animate-pulse" />
            <div className="w-9 h-9 bg-gray-100 rounded animate-pulse" />
          </div>
        </div>

        {/* Endpoint URL area */}
        <div className="h-10 bg-gray-100 rounded-lg animate-pulse" />

        {/* Tabs */}
        <div className="flex gap-1 border-b border-gray-200 overflow-x-auto pb-0">
          {[...Array(6)].map((_, i) => (
            <div key={i} className="h-9 w-24 bg-gray-100 rounded-t animate-pulse flex-shrink-0" />
          ))}
        </div>

        {/* Tab content: deployment rows */}
        <div className="space-y-3">
          {[...Array(4)].map((_, i) => (
            <div key={i} className="h-16 bg-gray-100 rounded-lg animate-pulse" />
          ))}
        </div>
      </div>
    );
  }

  if (isErrorServers) {
    return (
      <div className="flex flex-col items-center justify-center py-16">
        <AlertCircle className="w-12 h-12 text-red-400 mb-4" />
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

  if (!server) {
    return (
      <div className="py-16 text-center">
        <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-gray-100 flex items-center justify-center">
          <Server className="w-8 h-8 text-gray-400" />
        </div>
        <p className="text-gray-500">{tErrors('serverNotFound')}</p>
      </div>
    );
  }

  // Calculate effective status: if latest deployment failed, show failed status
  const latestDeployment = deployments?.[0];
  const effectiveStatus = latestDeployment?.status === 'failed' ? 'failed' : server.status;

  const statusStyle = STATUS_COLORS[effectiveStatus] || STATUS_COLORS.inactive;

  // Resolve the workspace slug for the public proxy URL: match the server's
  // workspace by id, falling back to the active workspace.
  const workspaceSlug =
    workspaces?.find((w) => w.id === server.workspace_id)?.slug ?? activeWorkspace?.slug;

  // tabs - no useMemo needed, t function from useTranslations may be unstable
  const tabs = [
    { id: 'deployments' as const, label: t('detail.deployments'), count: deployments?.length },
    { id: 'test' as const, label: t('test.title') },
    { id: 'secrets' as const, label: t('detail.secrets'), count: secrets?.length },
    { id: 'webhooks' as const, label: t('webhooks.title') },
    { id: 'metrics' as const, label: t('metrics.title') },
    { id: 'settings' as const, label: t('detail.settings') },
  ];

  return (
    <div className="max-w-5xl space-y-4 sm:space-y-6">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-start justify-between gap-4">
        <div className="flex items-center gap-3 sm:gap-4">
          <div className="flex-shrink-0">
            <JazzAvatar seed={server.id} diameter={44} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 flex-wrap">
              <h1 className="text-lg sm:text-2xl font-semibold text-gray-900 truncate">{server.name}</h1>
              <span className={`text-xs sm:text-sm font-medium flex items-center gap-1.5 sm:gap-2 ${statusStyle.text}`}>
                <span className={`w-1.5 h-1.5 sm:w-2 sm:h-2 rounded-full ${statusStyle.dot}`} />
                {t(`status.${effectiveStatus}`)}
              </span>
            </div>
            <p className="text-xs sm:text-sm text-gray-500 truncate">{server.github_repo}</p>
          </div>
        </div>

        <div className="flex items-center gap-2 self-start sm:self-auto">
          {/* Deploy Button */}
          <div className="flex flex-col items-end gap-1">
            <Button
              onClick={() => { setDeployError(null); deployMutation.mutate(); }}
              disabled={deployMutation.isPending || isAtDeployLimit}
              className="bg-gradient-to-r from-violet-600 to-purple-600 hover:from-violet-500 hover:to-purple-500 text-white shadow-lg shadow-violet-500/25"
            >
              {deployMutation.isPending ? (
                <div className="w-4 h-4 border-2 rounded-full border-white/30 border-t-white animate-spin" />
              ) : (
                <>
                  <Github className="w-4 h-4 mr-2" />
                  {t('detail.pullAndDeploy')}
                </>
              )}
            </Button>
            {deployError && (
              <p className="text-xs text-red-600">{deployError}</p>
            )}
          </div>

          {/* Share Button */}
          <Button
            variant="outline"
            size="icon"
            onClick={handleShareClick}
            title={t('detail.shareServer')}
            className={existingTemplate ? 'text-violet-600 border-violet-300 hover:border-violet-400' : 'text-gray-400 hover:text-violet-600 hover:border-violet-300'}
          >
            <Share2 className="w-4 h-4" />
          </Button>

          {/* Delete Button */}
          <AlertDialog onOpenChange={(open) => !open && setDeleteConfirmName('')}>
            <AlertDialogTrigger asChild>
              <Button variant="outline" size="icon" className="text-gray-400 hover:text-red-500 hover:border-red-300">
                <Trash2 className="w-4 h-4" />
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent className="max-w-[calc(100%-2rem)] sm:max-w-md mx-4 sm:mx-auto">
              <AlertDialogHeader className="space-y-4">
                <AlertDialogTitle className="flex items-center justify-center gap-2 text-gray-400">
                  <AlertCircle className="w-5 h-5 text-red-500" />
                  {t('detail.deleteServer')}
                </AlertDialogTitle>
                <AlertDialogDescription className="text-center space-y-3">
                  <p>{t('detail.deleteConfirm')}</p>
                  <div className="p-3 rounded-lg bg-red-50 border border-red-200">
                    <p className="text-sm text-red-700 font-medium">
                      {t('detail.deleteWarning')}
                    </p>
                  </div>
                </AlertDialogDescription>
              </AlertDialogHeader>
              <div className="py-4">
                <label className="block text-sm font-medium text-gray-500 mb-2">
                  {t('detail.deleteConfirmLabel')}
                </label>
                <div className="text-sm text-gray-500 mb-2">
                  <code className="px-2 py-1 bg-gray-100 rounded font-mono text-red-600">{server.name}</code>
                </div>
                <Input
                  value={deleteConfirmName}
                  onChange={(e) => setDeleteConfirmName(e.target.value)}
                  placeholder={server.name}
                  className="font-mono"
                />
                {deleteError && (
                  <p className="mt-2 text-sm text-red-600">{deleteError}</p>
                )}
              </div>
              <AlertDialogFooter>
                <AlertDialogCancel className="flex-1">{tCommon('cancel')}</AlertDialogCancel>
                <AlertDialogAction
                  onClick={() => { setDeleteError(null); deleteMutation.mutate(); }}
                  disabled={deleteConfirmName !== server.name || deleteMutation.isPending}
                  className="flex-1 bg-red-600 hover:bg-red-700 disabled:bg-gray-300 disabled:cursor-not-allowed"
                >
                  {deleteMutation.isPending ? (
                    <div className="w-4 h-4 border-2 rounded-full border-white/30 border-t-white animate-spin" />
                  ) : (
                    tCommon('delete')
                  )}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        </div>
      </div>

      {/* Share / Publish Template Dialog */}
      {shareDialogOpen && createPortal(
        <div className="fixed inset-0 z-[100] flex items-center justify-center p-4 bg-black/40" onClick={() => setShareDialogOpen(false)}>
          <div className="bg-white rounded-2xl shadow-xl w-full max-w-md p-6 space-y-4" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between">
              <h2 className="text-base font-semibold text-gray-900 flex items-center gap-2">
                <Share2 className="w-4 h-4 text-violet-600" />
                {t('detail.shareDialogTitle')}
              </h2>
              <button onClick={() => setShareDialogOpen(false)} className="text-gray-400 hover:text-gray-600">
                <X className="w-4 h-4" />
              </button>
            </div>

            <p className="text-sm text-gray-500">{t('detail.shareDialogDesc')}</p>

            <div className="space-y-3">
              <div>
                <Label className="text-xs font-medium text-gray-700">{t('detail.shareTemplateName')}</Label>
                <Input
                  value={shareTemplateName}
                  onChange={(e) => setShareTemplateName(e.target.value)}
                  className="mt-1 text-sm"
                />
              </div>
              <div>
                <Label className="text-xs font-medium text-gray-700">{t('detail.shareTemplateDesc')}</Label>
                <textarea
                  value={shareTemplateDesc}
                  onChange={(e) => setShareTemplateDesc(e.target.value)}
                  className="mt-1 w-full text-sm border border-input rounded-md px-3 py-2 bg-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring resize-none"
                  placeholder="Optional — shown on the template detail page"
                  rows={5}
                />
              </div>
              <div>
                <Label className="text-xs font-medium text-gray-700">{t('detail.shareEnvVarKeys')}</Label>
                <p className="text-xs text-gray-400 mt-0.5 mb-2">{t('detail.shareEnvVarKeysHint')}</p>
                {serverSecrets.length === 0 ? (
                  <p className="text-xs text-gray-400 italic">No secrets found on this server.</p>
                ) : (
                  <div className="flex flex-wrap gap-2">
                    {serverSecrets.map((s) => {
                      const selected = shareSelectedKeys.includes(s.key);
                      return (
                        <button
                          key={s.key}
                          type="button"
                          onClick={() =>
                            setShareSelectedKeys((prev) =>
                              selected ? prev.filter((k) => k !== s.key) : [...prev, s.key]
                            )
                          }
                          className={`inline-flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-mono border transition-colors ${
                            selected
                              ? 'bg-amber-50 border-amber-300 text-amber-700'
                              : 'bg-gray-50 border-gray-200 text-gray-500 hover:border-amber-200'
                          }`}
                        >
                          <Key className="w-3 h-3" />
                          {s.key}
                          {selected && <Check className="w-3 h-3" />}
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            </div>

            <div className="flex justify-end gap-2 pt-4 border-t border-gray-100">
              {existingTemplate && (
                <button
                  type="button"
                  onClick={() => unpublishTemplateMutation.mutate()}
                  disabled={unpublishTemplateMutation.isPending}
                  className="mr-auto px-4 py-2 text-sm font-medium text-red-600 hover:text-red-700 transition-colors disabled:opacity-50"
                >
                  {unpublishTemplateMutation.isPending ? tCommon('loading') : t('detail.shareUnpublish')}
                </button>
              )}
              <button
                type="button"
                onClick={() => setShareDialogOpen(false)}
                className="px-4 py-2 text-sm font-medium text-gray-700 hover:text-gray-900 transition-colors"
              >
                {tCommon('cancel')}
              </button>
              <button
                type="button"
                onClick={() =>
                  publishTemplateMutation.mutate({
                    name: shareTemplateName || undefined,
                    description: shareTemplateDesc || undefined,
                    required_env_var_keys: shareSelectedKeys,
                  })
                }
                disabled={publishTemplateMutation.isPending || !shareTemplateName.trim()}
                className="px-4 py-2 text-sm font-medium text-white bg-violet-600 rounded-lg hover:bg-violet-700 disabled:opacity-50 transition-colors flex items-center gap-2"
              >
                {publishTemplateMutation.isPending ? (
                  <div className="w-4 h-4 border-2 rounded-full border-white/30 border-t-white animate-spin" />
                ) : t('detail.sharePublish')}
              </button>
            </div>

            {existingTemplate && (
              <p className="text-xs text-violet-600 text-center flex items-center justify-center gap-1">
                <CheckCircle className="w-3.5 h-3.5" />
                {t('detail.sharePublished')}
              </p>
            )}
          </div>
        </div>,
        document.body
      )}

      {/* Upgrade Banner */}
      {isAtDeployLimit && currentWorkspace?.plan !== 'enterprise' && (
        <div className="p-5 rounded-2xl bg-gradient-to-r from-amber-50 to-orange-50 border border-amber-200">
          <div className="flex items-center gap-4">
            <div className="w-10 h-10 rounded-full bg-amber-100 flex items-center justify-center flex-shrink-0">
              <AlertTriangle className="w-5 h-5 text-amber-600" />
            </div>
            <div className="flex-1">
              <p className="font-medium text-amber-800">{t('upgrade.title')}</p>
              <p className="text-sm text-amber-700 mt-1">{t('upgrade.deployLimit')}</p>
            </div>
            <Link href="/dashboard/billing">
              <Button variant="outline" className="border-amber-300 text-amber-700 hover:bg-amber-100">
                {t('upgrade.cta')}
              </Button>
            </Link>
          </div>
        </div>
      )}

      {/* Info Pills */}
      <div className="flex items-center gap-2 text-xs sm:text-sm overflow-x-auto scrollbar-hide pb-1 -mx-4 px-4 sm:mx-0 sm:px-0 sm:flex-wrap sm:overflow-visible">
        {(server.status === 'running' || server.status === 'stopped') && server.endpoint_url && workspaceSlug && (
          <button
            onClick={() => {
              navigator.clipboard.writeText(mcpPublicUrl(workspaceSlug, server.slug, server.mcp_path));
            }}
            className="flex items-center gap-1.5 px-2.5 sm:px-3 py-1 sm:py-1.5 bg-violet-50 hover:bg-violet-100 rounded-full transition-colors group flex-shrink-0"
            title={t('detail.copyEndpoint')}
          >
            <span className="text-gray-500 hidden sm:inline">{t('detail.endpoint')}</span>
            <code className="font-medium text-violet-700 font-mono text-xs sm:text-sm truncate max-w-[150px] sm:max-w-none">{mcpPublicUrl(workspaceSlug, server.slug, server.mcp_path)}</code>
            <Copy className="w-3 h-3 sm:w-3.5 sm:h-3.5 text-violet-400 group-hover:text-violet-600 flex-shrink-0" />
          </button>
        )}
        <span className="px-2.5 sm:px-3 py-1 sm:py-1.5 bg-gray-100 rounded-full flex-shrink-0">
          <span className="text-gray-500">{t('detail.runtime')}</span>
          <span className="ml-1 sm:ml-1.5 font-medium text-gray-900 capitalize">{server.runtime}</span>
        </span>
        <span className="px-2.5 sm:px-3 py-1 sm:py-1.5 bg-gray-100 rounded-full flex-shrink-0">
          <span className="text-gray-500">{t('visibility')}</span>
          <span className="ml-1 sm:ml-1.5 font-medium text-gray-900 capitalize">{server.visibility}</span>
        </span>
        <span className="px-2.5 sm:px-3 py-1 sm:py-1.5 bg-gray-100 rounded-full flex-shrink-0">
          <span className="text-gray-500">{t('create.branch')}</span>
          <span className="ml-1 sm:ml-1.5 font-medium text-gray-900 font-mono">{server.github_branch}</span>
        </span>
        <div className="relative flex-shrink-0">
          <button
            onClick={() => setShowDeployInfo(!showDeployInfo)}
            className="flex items-center gap-1 sm:gap-1.5 px-2.5 sm:px-3 py-1 sm:py-1.5 bg-gray-100 hover:bg-gray-200 rounded-full transition-colors"
          >
            <span className="text-gray-500">{t('detail.deploys')}</span>
            <span className="font-medium text-gray-900">{deploymentsThisMonth}/{maxDeployments === 4294967295 ? '∞' : maxDeployments}</span>
            <AlertCircle className="w-3 h-3 sm:w-3.5 sm:h-3.5 text-gray-400" />
          </button>
          {showDeployInfo && (
            <div className="absolute top-full left-0 mt-2 w-72 p-4 rounded-xl bg-white border border-gray-200 shadow-xl z-50">
              <p className="font-medium text-gray-900 mb-2">{t('detail.deployInfo')}</p>
              <div className="flex items-center gap-2 mb-3">
                <span className="text-2xl font-bold text-violet-600">{deploymentsThisMonth}</span>
                <span className="text-gray-400">/</span>
                <span className="text-lg text-gray-500">{maxDeployments === 4294967295 ? '∞' : maxDeployments}</span>
              </div>
              <p className="text-sm text-gray-500">
                {maxDeployments === 4294967295 ? t('detail.deployInfoUnlimited') : t('detail.deployInfoDesc', { max: maxDeployments })}
              </p>
              <Link
                href="/dashboard/billing"
                className="inline-flex items-center gap-1 text-sm text-violet-600 hover:text-violet-700 mt-3 font-medium"
                onClick={() => setShowDeployInfo(false)}
              >
                {t('detail.viewPlan')}
              </Link>
            </div>
          )}
        </div>
      </div>

      {/* Tabs */}
      <div>
        <div className="relative -mx-4 px-4 sm:mx-0 sm:px-0">
          {/* Bottom border line - positioned behind tabs */}
          <div className="absolute bottom-0 left-0 right-0 h-[2px] bg-gray-200" />
          <div className="flex gap-1 overflow-x-auto scrollbar-hide">
            {tabs.map((tab) => (
              <button
                key={tab.id}
                onClick={() => setActiveTab(tab.id)}
                className={`relative px-3 sm:px-4 py-2 sm:py-2.5 text-xs sm:text-sm font-medium border-b-2 transition-colors whitespace-nowrap flex-shrink-0 ${
                  activeTab === tab.id
                    ? 'border-violet-600 text-violet-600'
                    : 'border-transparent text-gray-500 hover:text-gray-700'
                }`}
              >
                {tab.label}
                {'count' in tab && tab.count !== undefined && (
                  <span className={`ml-1.5 sm:ml-2 px-1.5 sm:px-2 py-0.5 rounded-full text-xs ${
                    activeTab === tab.id ? 'bg-violet-100 text-violet-700' : 'bg-gray-100 text-gray-600'
                  }`}>
                    {tab.count}
                  </span>
                )}
              </button>
            ))}
          </div>
        </div>

        <div className="mt-6">
          {activeTab === 'deployments' && (
            <DeploymentsTab deployments={deployments ?? []} workspaceId={workspaceId} serverId={serverId} t={t} tCommon={tCommon} autoSelectDeploymentId={autoSelectDeploymentId} />
          )}
          {activeTab === 'test' && (
            <TestTab
              serverId={serverId}
              workspaceId={workspaceId!}
              t={t}
              tCommon={tCommon}
              tApiErrors={tApiErrors}
            />
          )}
          {activeTab === 'secrets' && (
            <SecretsTab
              secrets={secrets ?? []}
              serverId={serverId}
              workspaceId={workspaceId!}
              t={t}
              tCommon={tCommon}
            />
          )}
          {activeTab === 'webhooks' && (
            <WebhooksTab
              serverId={serverId}
              workspaceId={workspaceId!}
              t={t}
              tCommon={tCommon}
            />
          )}
          {activeTab === 'metrics' && (
            <MetricsTab
              serverId={serverId}
              workspaceId={workspaceId!}
              serverStatus={server.status}
              t={t}
            />
          )}
          {activeTab === 'settings' && (
            <SettingsTab
              server={server}
              workspaceId={workspaceId!}
              t={t}
              tCommon={tCommon}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function DeploymentsTab({ deployments, workspaceId, serverId, t, tCommon, autoSelectDeploymentId }: { deployments: Deployment[]; workspaceId?: string; serverId: string; t: (key: string) => string; tCommon: (key: string) => string; autoSelectDeploymentId?: string | null }) {
  const [selectedDeployment, setSelectedDeployment] = useState<string | null>(null);
  const [buildElapsed, setBuildElapsed] = useState<Record<string, number>>({});

  // Auto-open log panel when a new deployment is triggered from the deploy button
  useEffect(() => {
    if (autoSelectDeploymentId) {
      setSelectedDeployment(autoSelectDeploymentId);
    }
  }, [autoSelectDeploymentId]);

  // Timer for build elapsed time
  useEffect(() => {
    const buildingDeployments = deployments?.filter(
      d => d.status === 'building' || d.status === 'deploying'
    );

    if (!buildingDeployments?.length) {
      setBuildElapsed({});
      return;
    }

    const updateElapsed = () => {
      const now = Date.now();
      const elapsed: Record<string, number> = {};
      buildingDeployments.forEach(d => {
        elapsed[d.id] = Math.floor((now - new Date(d.created_at).getTime()) / 1000);
      });
      setBuildElapsed(elapsed);
    };

    updateElapsed();
    const interval = setInterval(updateElapsed, 1000);
    return () => clearInterval(interval);
  }, [deployments]);

  if (deployments.length === 0) {
    return (
      <div className="py-16 text-center">
        <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-gray-100 flex items-center justify-center">
          <Rocket className="w-8 h-8 text-gray-400" />
        </div>
        <p className="text-gray-500">{t('detail.noDeployments')}</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Build Logs Panel */}
      {selectedDeployment && (
        <div className="mb-4">
          <div className="flex items-center justify-between mb-2">
            <h3 className="text-sm font-medium text-gray-600">
              {t('detail.buildLogs')} - {selectedDeployment.slice(0, 8)}
            </h3>
            <button
              onClick={() => setSelectedDeployment(null)}
              className="text-sm text-gray-400 hover:text-gray-600"
            >
              {tCommon('close')}
            </button>
          </div>
          <BuildLogsPanel
            deploymentId={selectedDeployment}
            workspaceId={workspaceId}
            serverId={serverId}
            maxHeight="300px"
          />
        </div>
      )}

      {/* Deployments List */}
      <div className="flex gap-5 max-w-4xl mx-auto">
        {/* Timeline */}
        <div className="relative flex flex-col items-center">
          {/* Timeline line */}
          <div className="absolute top-3 bottom-3 w-px bg-gray-300" />
          {/* Timeline dots */}
          {deployments.map((_, index) => (
            <div
              key={index}
              className="relative w-3 h-3 rounded-full bg-white border-2 border-gray-400 z-10 my-4 first:mt-3"
              style={{ marginTop: index === 0 ? '12px' : '40px' }}
            />
          ))}
        </div>

        {/* Cards */}
        <div className="flex-1 border border-gray-200 rounded-lg overflow-hidden">
          {deployments.map((deployment, index) => {
            const style = DEPLOYMENT_STATUS_COLORS[deployment.status] || DEPLOYMENT_STATUS_COLORS.pending;
            const isSelected = selectedDeployment === deployment.id;
            const isBuilding = deployment.status === 'building' || deployment.status === 'deploying' || deployment.status === 'pending' || deployment.status === 'pushing';
            const isSuccess = deployment.status === 'succeeded';
            const isLast = index === deployments.length - 1;

            return (
              <div
                key={deployment.id}
                className={`flex items-start justify-between gap-4 py-3 px-4 bg-white transition-all cursor-pointer hover:bg-gray-50 ${
                  isSelected ? 'bg-violet-50' : ''
                } ${!isLast ? 'border-b border-gray-200' : ''}`}
                onClick={() => setSelectedDeployment(isSelected ? null : deployment.id)}
              >
                <div className="flex-1 min-w-0">
                  {/* Commit SHA */}
                  <p className="font-semibold text-[#323232] truncate font-mono">
                    {deployment.commit_sha.substring(0, 7)}
                  </p>
                  {/* Meta info */}
                  <div className="flex items-center gap-2 mt-1 text-sm text-gray-500">
                    <span>v{deployments.length - index}</span>
                    <span>·</span>
                    <span>{getRelativeTime(deployment.created_at, t)}</span>
                    <span>·</span>
                    {isSuccess ? (
                      <span className="flex items-center gap-1 text-green-600">
                        <Check className="w-4 h-4" />
                        {t(`status.${deployment.status}`)}
                      </span>
                    ) : isBuilding ? (
                      <span className="flex items-center gap-1 text-yellow-600">
                        <span className="w-2 h-2 rounded-full bg-yellow-500 animate-pulse" />
                        {t('status.building')}
                        {buildElapsed[deployment.id] !== undefined && (
                          <span className="text-gray-400 font-mono ml-1">
                            {buildElapsed[deployment.id] >= 60
                              ? `${Math.floor(buildElapsed[deployment.id] / 60)}m ${buildElapsed[deployment.id] % 60}s`
                              : `${buildElapsed[deployment.id]}s`}
                          </span>
                        )}
                      </span>
                    ) : deployment.status === 'failed' ? (
                      <span className="flex items-center gap-1 text-red-600">
                        <X className="w-4 h-4" />
                        {t(`status.${deployment.status}`)}
                      </span>
                    ) : (
                      <span className={style.text}>{t(`status.${deployment.status}`)}</span>
                    )}
                  </div>
                </div>
                {/* Right side - commit SHA and actions */}
                <div className="flex items-center gap-2 flex-shrink-0">
                  <code className="text-sm font-mono text-gray-500">{deployment.commit_sha.slice(0, 7)}</code>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      navigator.clipboard.writeText(deployment.commit_sha);
                    }}
                    className="p-1 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded transition-colors"
                    title={t('detail.copySha') || 'Copy SHA'}
                  >
                    <Copy className="w-4 h-4" />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

interface HealthCheckResponse {
  status: string;
  endpoint_url: string | null;
  connection: {
    reachable: boolean;
    latency_ms: number | null;
    mcp_version: string | null;
  };
  tools: Array<{
    name: string;
    description: string | null;
    input_schema: Record<string, unknown> | null;
  }> | null;
  error: string | null;
}

interface ExecuteToolResponse {
  success: boolean;
  result: unknown;
  error: string | null;
  latency_ms: number;
}

function TestTab({
  serverId,
  workspaceId,
  t,
  tCommon,
  tApiErrors
}: {
  serverId: string;
  workspaceId: string;
  t: (key: string, values?: Record<string, string | number>) => string;
  tCommon: (key: string) => string;
  tApiErrors: (key: string) => string;
}) {
  const queryClient = useQueryClient();
  const [selectedTool, setSelectedTool] = useState<string | null>(null);
  const [toolArgs, setToolArgs] = useState<string>('{}');
  const [executeResult, setExecuteResult] = useState<ExecuteToolResponse | null>(null);
  const [isDescriptionExpanded, setIsDescriptionExpanded] = useState(false);
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  // Check if there's cached test data
  const cachedData = queryClient.getQueryData<HealthCheckResponse>(['servers', serverId, 'test']);
  const [hasTestedOnce, setHasTestedOnce] = useState(!!cachedData);

  const healthCheckQuery = useQuery<HealthCheckResponse>({
    queryKey: ['servers', serverId, 'test'],
    queryFn: () => api.get(`/workspaces/${workspaceId}/servers/${serverId}/test`),
    enabled: false,
    staleTime: 5 * 60 * 1000, // Keep data fresh for 5 minutes
  });

  const handleRunTest = () => {
    setHasTestedOnce(true);
    const toastId = toast.loading(t('test.testing'));
    // refetch() always fetches fresh data since enabled: false
    healthCheckQuery.refetch().then((result) => {
      if (result.data?.status === 'healthy') {
        toast.success(t('test.testSuccess'), { id: toastId });
      } else {
        toast.error(t('test.testFailed'), { id: toastId });
      }
    }).catch(() => {
      toast.error(t('test.testFailed'), { id: toastId });
    });
  };

  const executeMutation = useMutation({
    mutationFn: async ({ toolName, args }: { toolName: string; args: string }) => {
      let parsedArgs = {};
      try {
        parsedArgs = JSON.parse(args);
      } catch {
        throw new Error('Invalid JSON arguments');
      }
      return api.post<ExecuteToolResponse>(`/workspaces/${workspaceId}/servers/${serverId}/test/execute`, {
        tool_name: toolName,
        arguments: parsedArgs,
      });
    },
    onSuccess: (data) => {
      setExecuteResult(data);
    },
    onError: (error: any) => {
      // Try to translate error code if available
      let errorMessage = error.message;
      if (error.code) {
        try {
          const translated = tApiErrors(error.code);
          if (translated && translated !== error.code) {
            errorMessage = translated;
          }
        } catch {
          // Translation not found
        }
      }
      setExecuteResult({
        success: false,
        result: null,
        error: errorMessage,
        latency_ms: 0,
      });
    },
  });

  const healthCheck = healthCheckQuery.data;
  const isLoading = healthCheckQuery.isLoading || healthCheckQuery.isFetching;
  const selectedToolData = healthCheck?.tools?.find(tool => tool.name === selectedTool);

  // Initial state
  if (!hasTestedOnce && !healthCheck) {
    return (
      <div className="flex flex-col items-center justify-center py-16">
        <div className="w-12 h-12 mb-4 rounded-full bg-gray-100 flex items-center justify-center">
          <CheckCircle className="w-6 h-6 text-gray-400" />
        </div>
        <p className="text-gray-500 text-sm mb-4">{t('test.description')}</p>
        <button
          onClick={handleRunTest}
          className="px-4 py-2 text-sm font-medium text-white bg-violet-600 rounded-lg hover:bg-violet-700 transition-colors"
        >
          {t('test.runTest')}
        </button>
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <div className="h-14 bg-gray-100 rounded-lg animate-pulse" />
        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-3">
          {[...Array(6)].map((_, i) => (
            <div key={i} className="h-20 bg-gray-100 rounded-xl animate-pulse" />
          ))}
        </div>
      </div>
    );
  }

  const isHealthy = healthCheck?.status === 'healthy';
  const isNotDeployed = healthCheck?.status === 'not_deployed';

  return (
    <div className="space-y-6">
      {/* Status Bar */}
      <div className="flex items-center justify-between p-4 rounded-lg bg-gray-50 border border-gray-200">
        <div className="flex items-center gap-4">
          <div className={`w-2.5 h-2.5 rounded-full ${
            isHealthy ? 'bg-emerald-500' : isNotDeployed ? 'bg-gray-400' : 'bg-red-500'
          }`} />
          <span className="text-sm font-medium" style={{ color: '#323232' }}>
            {healthCheck?.status === 'healthy' ? t('test.statusHealthy') :
             healthCheck?.status === 'not_deployed' ? t('test.statusNotDeployed') :
             healthCheck?.status === 'unreachable' ? t('test.statusUnreachable') :
             t('test.statusError')}
          </span>
          {healthCheck?.connection.latency_ms && (
            <span className="text-xs text-gray-500">{healthCheck.connection.latency_ms}ms</span>
          )}
          {healthCheck?.connection.mcp_version && (
            <span className="text-xs text-gray-400">MCP {healthCheck.connection.mcp_version}</span>
          )}
          {healthCheck?.tools && (
            <span className="text-xs text-gray-400">{healthCheck.tools.length} tools</span>
          )}
        </div>
        <button
          onClick={handleRunTest}
          disabled={isLoading}
          className="text-sm text-violet-600 hover:text-violet-700 font-medium disabled:opacity-50"
        >
          {t('test.runTestAgain')}
        </button>
      </div>

      {/* Error */}
      {healthCheck?.error && (
        <div className="p-3 rounded-lg bg-red-50 border border-red-200">
          <p className="text-sm text-red-700">{healthCheck.error}</p>
        </div>
      )}

      {/* Tools Grid */}
      {healthCheck?.tools && healthCheck.tools.length > 0 && (
        <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-3">
          {healthCheck.tools.map((tool) => (
            <button
              key={tool.name}
              onClick={() => {
                setSelectedTool(tool.name);
                setExecuteResult(null);
                setToolArgs('{}');
                setIsDescriptionExpanded(false);
              }}
              className="group p-4 rounded-xl bg-white border border-gray-200 hover:border-violet-300 hover:shadow-sm transition-all text-left"
            >
              <div className="flex items-start gap-3">
                <div className="w-9 h-9 rounded-lg flex items-center justify-center flex-shrink-0">
                  <Wrench className="w-4.5 h-4.5 text-violet-600" />
                </div>
                <div className="min-w-0">
                  <p className="text-sm font-medium text-gray-900 truncate">{tool.name}</p>
                  {tool.description && (
                    <p className="text-xs text-gray-500 truncate mt-0.5">{tool.description}</p>
                  )}
                </div>
              </div>
            </button>
          ))}
        </div>
      )}

      {/* No Tools */}
      {healthCheck?.status === 'healthy' && (!healthCheck.tools || healthCheck.tools.length === 0) && (
        <div className="py-8 text-center text-gray-500 text-sm">
          {t('test.noTools')}
        </div>
      )}

      {/* Tool Modal - rendered via Portal to avoid stacking context issues */}
      {mounted && selectedTool && selectedToolData && createPortal(
        <div className="fixed inset-0 z-[100] flex items-center justify-center p-4 bg-black/30" onClick={() => setSelectedTool(null)}>
          <div
            className="w-full max-w-lg bg-white rounded-xl shadow-xl overflow-hidden"
            onClick={(e) => e.stopPropagation()}
          >
            {/* Header */}
            <div className="flex items-center justify-between px-5 py-4 border-b border-gray-100">
              <div className="flex items-center gap-3">
                <div className="w-9 h-9 rounded-lg flex items-center justify-center">
                  <Wrench className="w-4.5 h-4.5 text-violet-600" />
                </div>
                <h3 className="font-semibold text-gray-900">{selectedTool}</h3>
              </div>
              <button
                onClick={() => setSelectedTool(null)}
                className="p-1.5 rounded-lg hover:bg-gray-100 transition-colors"
              >
                <X className="w-5 h-5 text-gray-400" />
              </button>
            </div>

            {/* Body */}
            <div className="p-5 space-y-4 max-h-[60vh] overflow-y-auto">
              {/* Tool Description (expandable) */}
              {selectedToolData.description && (
                <div className="text-sm text-gray-600">
                  <p className={isDescriptionExpanded ? '' : 'line-clamp-3'}>
                    {selectedToolData.description}
                  </p>
                  {selectedToolData.description.length > 150 && (
                    <button
                      onClick={() => setIsDescriptionExpanded(!isDescriptionExpanded)}
                      className="text-xs text-violet-600 hover:text-violet-700 mt-1"
                    >
                      {isDescriptionExpanded ? t('test.showLess') : t('test.showMore')}
                    </button>
                  )}
                </div>
              )}

              {/* Schema */}
              {selectedToolData.input_schema && (
                <details className="group">
                  <summary className="cursor-pointer text-xs text-gray-500 hover:text-gray-700 flex items-center gap-1.5 font-medium uppercase tracking-wide">
                    <ChevronRight className="w-3.5 h-3.5 transition-transform group-open:rotate-90" />
                    Schema
                  </summary>
                  <pre className="mt-2 p-3 rounded-lg bg-gray-900 text-gray-300 text-xs overflow-x-auto">
                    {JSON.stringify(selectedToolData.input_schema, null, 2)}
                  </pre>
                </details>
              )}

              {/* Arguments */}
              <div>
                <label className="block text-xs text-gray-500 font-medium uppercase tracking-wide mb-2">
                  {t('test.arguments')}
                </label>
                <textarea
                  value={toolArgs}
                  onChange={(e) => setToolArgs(e.target.value)}
                  placeholder='{"key": "value"}'
                  rows={4}
                  className="w-full px-3 py-2.5 text-sm font-mono bg-gray-900 text-gray-300 border border-gray-700 rounded-lg focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent placeholder-gray-600"
                />
              </div>

              {/* Result */}
              {executeResult && (
                <div className={`rounded-lg border overflow-hidden ${
                  executeResult.success ? 'border-emerald-200' : 'border-red-200'
                }`}>
                  <div className={`px-3 py-2 flex items-center justify-between text-sm ${
                    executeResult.success ? 'bg-emerald-50 text-emerald-700' : 'bg-red-50 text-red-700'
                  }`}>
                    <span className="font-medium">
                      {executeResult.success ? t('test.resultSuccess') : t('test.resultError')}
                    </span>
                    <span className="text-xs opacity-70">{executeResult.latency_ms}ms</span>
                  </div>
                  <div className="p-3 bg-white">
                    {executeResult.error ? (
                      <p className="text-sm text-red-600">{executeResult.error}</p>
                    ) : (
                      <pre className="text-xs text-gray-700 overflow-x-auto whitespace-pre-wrap font-mono">
                        {JSON.stringify(executeResult.result, null, 2)}
                      </pre>
                    )}
                  </div>
                </div>
              )}
            </div>

            {/* Footer */}
            <div className="flex justify-end gap-2 px-5 py-4 border-t border-gray-100 bg-gray-50">
              <button
                onClick={() => setSelectedTool(null)}
                className="px-4 py-2 text-sm font-medium text-gray-700 hover:text-gray-900 transition-colors"
              >
                {tCommon('cancel')}
              </button>
              <button
                onClick={() => executeMutation.mutate({ toolName: selectedTool, args: toolArgs })}
                disabled={executeMutation.isPending}
                className="px-4 py-2 text-sm font-medium text-white bg-violet-600 rounded-lg hover:bg-violet-700 disabled:opacity-50 transition-colors flex items-center gap-2"
              >
                {executeMutation.isPending ? (
                  <div className="w-4 h-4 border-2 rounded-full border-white/30 border-t-white animate-spin" />
                ) : (
                  <>
                    <Play className="w-4 h-4" />
                    {t('test.execute')}
                  </>
                )}
              </button>
            </div>
          </div>
        </div>,
        document.body
      )}

    </div>
  );
}

function SecretsTab({
  secrets,
  serverId,
  workspaceId,
  t,
  tCommon
}: {
  secrets: Secret[];
  serverId: string;
  workspaceId: string;
  t: (key: string) => string;
  tCommon: (key: string) => string;
}) {
  const [newKey, setNewKey] = useState('');
  const [newValue, setNewValue] = useState('');
  const [editingKey, setEditingKey] = useState<string | null>(null);
  const [editKey, setEditKey] = useState('');
  const [editValue, setEditValue] = useState('');
  const queryClient = useQueryClient();

  const invalidateSecrets = () =>
    queryClient.invalidateQueries({ queryKey: ['servers', serverId, 'secrets'] });

  const createMutation = useMutation({
    mutationFn: () =>
      api.post(`/workspaces/${workspaceId}/servers/${serverId}/secrets`, { key: newKey, value: newValue }),
    onSuccess: () => {
      invalidateSecrets();
      setNewKey('');
      setNewValue('');
    },
    onError: (error: any) => {
      toast.error(error?.message || tCommon('error'));
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ key, value }: { key: string; value: string }) =>
      api.post(`/workspaces/${workspaceId}/servers/${serverId}/secrets`, { key, value }),
    onSuccess: () => {
      invalidateSecrets();
      setEditingKey(null);
      toast.success(t('detail.updateSecretSuccess'));
    },
    onError: (error: any) => {
      toast.error(error?.message || t('detail.updateSecretError'));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (secretKey: string) =>
      api.delete(`/workspaces/${workspaceId}/servers/${serverId}/secrets/${secretKey}`),
    onSuccess: () => {
      invalidateSecrets();
      toast.success(t('detail.deleteSecretSuccess'));
    },
    onError: (error: any) => {
      toast.error(error?.message || t('detail.deleteSecretError'));
    },
  });

  const startEdit = (key: string) => {
    setEditingKey(key);
    setEditKey(key);
    setEditValue('');
  };

  const cancelEdit = () => {
    setEditingKey(null);
    setEditKey('');
    setEditValue('');
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (newKey && newValue) {
      createMutation.mutate();
    }
  };

  return (
    <div className="rounded-xl border border-gray-200 overflow-hidden">
      <table className="w-full">
        <thead>
          <tr className="bg-gray-50 border-b border-gray-200">
            <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider w-1/3 border-r border-gray-200">
              {t('detail.keyPlaceholder')}
            </th>
            <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider border-r border-gray-200">
              {t('detail.valuePlaceholder')}
            </th>
            <th className="px-4 py-3 w-20"></th>
          </tr>
        </thead>
        <tbody className="bg-white divide-y divide-gray-200">
          {secrets.map((secret) => {
            const isEditing = editingKey === secret.key;
            return (
              <tr key={secret.key} className="group hover:bg-gray-50 transition-colors">
                {isEditing ? (
                  <>
                    <td className="px-4 py-2 border-r border-gray-200">
                      <input
                        type="text"
                        value={editKey}
                        onChange={(e) => setEditKey(e.target.value.toUpperCase())}
                        className="w-full px-2 py-1.5 text-sm font-mono bg-white border border-gray-200 rounded-md focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
                      />
                    </td>
                    <td className="px-4 py-2 border-r border-gray-200">
                      <input
                        autoFocus
                        type="password"
                        value={editValue}
                        onChange={(e) => setEditValue(e.target.value)}
                        placeholder={t('detail.valuePlaceholder')}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter' && editKey && editValue) {
                            updateMutation.mutate({ key: editKey, value: editValue });
                          } else if (e.key === 'Escape') {
                            cancelEdit();
                          }
                        }}
                        className="w-full px-2 py-1.5 text-sm font-mono bg-white border border-gray-200 rounded-md focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
                      />
                    </td>
                    <td className="px-4 py-2">
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => updateMutation.mutate({ key: editKey, value: editValue })}
                          disabled={!editKey || !editValue || updateMutation.isPending}
                          className="p-1.5 text-violet-500 hover:text-violet-700 transition-colors disabled:text-gray-300 disabled:cursor-not-allowed"
                          title={tCommon('save')}
                        >
                          {updateMutation.isPending ? (
                            <div className="w-4 h-4 border-2 rounded-full border-violet-200 border-t-violet-600 animate-spin" />
                          ) : (
                            <Check className="w-4 h-4" />
                          )}
                        </button>
                        <button
                          onClick={cancelEdit}
                          disabled={updateMutation.isPending}
                          className="p-1.5 text-gray-400 hover:text-gray-600 transition-colors"
                          title={tCommon('cancel')}
                        >
                          <X className="w-4 h-4" />
                        </button>
                      </div>
                    </td>
                  </>
                ) : (
                  <>
                    <td className="px-4 py-3 border-r border-gray-200">
                      <code className="text-sm font-mono text-gray-900">{secret.key}</code>
                    </td>
                    <td className="px-4 py-3 border-r border-gray-200">
                      <span className="text-sm text-gray-400 font-mono">••••••••••••</span>
                    </td>
                    <td className="px-4 py-3">
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => startEdit(secret.key)}
                          disabled={deleteMutation.isPending || !!editingKey}
                          className="p-1.5 text-gray-400 hover:text-violet-600 transition-colors disabled:text-gray-200 disabled:cursor-not-allowed"
                          title={tCommon('edit')}
                        >
                          <Pencil className="w-4 h-4" />
                        </button>
                        <button
                          onClick={() => deleteMutation.mutate(secret.key)}
                          disabled={deleteMutation.isPending || !!editingKey}
                          className="p-1.5 text-gray-400 hover:text-red-600 transition-colors disabled:text-gray-200 disabled:cursor-not-allowed"
                          title={tCommon('delete')}
                        >
                          <Trash2 className="w-4 h-4" />
                        </button>
                      </div>
                    </td>
                  </>
                )}
              </tr>
            );
          })}
          {/* Inline Add Row */}
          <tr className="bg-gray-50/50">
            <td className="px-4 py-2 border-r border-gray-200">
              <input
                type="text"
                value={newKey}
                onChange={(e) => setNewKey(e.target.value.toUpperCase())}
                placeholder="NEW_KEY"
                className="w-full px-2 py-1.5 text-sm font-mono bg-white border border-gray-200 rounded-md focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
              />
            </td>
            <td className="px-4 py-2 border-r border-gray-200">
              <input
                type="password"
                value={newValue}
                onChange={(e) => setNewValue(e.target.value)}
                placeholder="Enter value..."
                onKeyDown={(e) => {
                  if (e.key === 'Enter') handleSubmit(e);
                }}
                className="w-full px-2 py-1.5 text-sm font-mono bg-white border border-gray-200 rounded-md focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
              />
            </td>
            <td className="px-4 py-2">
              <button
                onClick={handleSubmit}
                disabled={!newKey || !newValue || createMutation.isPending}
                className="p-1.5 text-violet-500 hover:text-violet-700 transition-colors disabled:text-gray-300 disabled:cursor-not-allowed"
                title={tCommon('add')}
              >
                {createMutation.isPending ? (
                  <div className="w-4 h-4 border-2 rounded-full border-violet-200 border-t-violet-600 animate-spin" />
                ) : (
                  <Plus className="w-4 h-4" />
                )}
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  );
}

function SettingsTab({
  server,
  workspaceId,
  t,
  tCommon
}: {
  server: McpServer;
  workspaceId: string;
  t: (key: string) => string;
  tCommon: (key: string) => string;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(server.name);
  const [description, setDescription] = useState(server.description || '');
  const [visibility, setVisibility] = useState(server.visibility);
  const [accessMode, setAccessMode] = useState<AccessMode>(server.access_mode || 'public');
  const [branch, setBranch] = useState(server.github_branch);
  const [rootDirectory, setRootDirectory] = useState(server.root_directory || '');
  const [mcpPath, setMcpPath] = useState(server.mcp_path || '/mcp');
  const [entryCommand, setEntryCommand] = useState(server.entry_command || '');
  const [buildCommand, setBuildCommand] = useState(server.build_command || '');
  const [authEnabled, setAuthEnabled] = useState(server.auth_enabled ?? true);
  const [memoryMb, setMemoryMb] = useState(server.memory_mb ?? DEFAULT_MEMORY_MB);
  const [port, setPort] = useState<number | ''>(server.port ?? '');
  const [toolFilterByScope, setToolFilterByScope] = useState(server.tool_list_filter_by_scope ?? true);
  const [toolSchemaSlim, setToolSchemaSlim] = useState(server.tool_schema_slim ?? false);
  const [toolSearchMode, setToolSearchMode] = useState(server.tool_search_mode ?? false);
  const [toolCodeMode, setToolCodeMode] = useState(server.tool_code_mode ?? true);
  const [upstreamOauthEnabled, setUpstreamOauthEnabled] = useState<boolean>(!!server.upstream_oauth_provider);
  const [upstreamOauthProvider, setUpstreamOauthProvider] = useState<string>(server.upstream_oauth_provider ?? '');
  const [upstreamOauthScopes, setUpstreamOauthScopes] = useState<string>((server.upstream_oauth_scopes ?? []).join(' '));
  const [upstreamOauthAuthUrl, setUpstreamOauthAuthUrl] = useState<string>(server.upstream_oauth_authorization_url ?? '');
  const [upstreamOauthTokenUrl, setUpstreamOauthTokenUrl] = useState<string>(server.upstream_oauth_token_url ?? '');
  const [upstreamOauthClientId, setUpstreamOauthClientId] = useState<string>(server.upstream_oauth_client_id ?? '');
  const [upstreamOauthClientSecret, setUpstreamOauthClientSecret] = useState<string>('');
  const [isSaving, setIsSaving] = useState(false);

  // Plan limits cap which memory sizes are selectable (Free is limited to 256MB).
  const { data: plans } = useQuery<{ plan: string; limits: { max_memory_mb: number } }[]>({
    queryKey: ['billing-plans'],
    queryFn: () => api.get('/billing/plans'),
  });
  const { workspaces } = useWorkspace();
  const maxMemoryMb = findPlanLimits(plans, workspaces?.find((w) => w.id === workspaceId)?.plan)?.max_memory_mb ?? 256;

  const { data: upstreamProviders = [] } = useQuery<UpstreamOAuthProvider[]>({
    queryKey: ['upstream-oauth-providers'],
    queryFn: () => api.get('/oauth/upstream-providers'),
  });

  const handleOauthProviderChange = (val: string) => {
    setUpstreamOauthProvider(val);
    const preset = upstreamProviders.find((p) => p.name === val);
    if (preset && !preset.is_managed) {
      setUpstreamOauthAuthUrl('');
      setUpstreamOauthTokenUrl('');
    }
    if (preset) setUpstreamOauthScopes(preset.default_scopes.join(' '));
  };

  const handleSave = async () => {
    setIsSaving(true);
    try {
      const isCustomProvider = upstreamOauthProvider === 'custom';
      await api.patch(`/workspaces/${workspaceId}/servers/${server.id}`, {
        name,
        description: description || null,
        visibility,
        access_mode: accessMode,
        github_branch: branch,
        root_directory: rootDirectory || '',
        mcp_path: mcpPath || '/mcp',
        entry_command: entryCommand || undefined,
        build_command: buildCommand || undefined,
        auth_enabled: authEnabled,
        memory_mb: memoryMb,
        port: typeof port === 'number' ? port : undefined,
        tool_list_filter_by_scope: toolFilterByScope,
        tool_schema_slim: toolSchemaSlim,
        tool_search_mode: toolSearchMode,
        tool_code_mode: toolCodeMode,
        upstream_oauth_provider: upstreamOauthEnabled && upstreamOauthProvider ? upstreamOauthProvider : null,
        upstream_oauth_scopes: upstreamOauthEnabled && upstreamOauthProvider
          ? upstreamOauthScopes.split(/\s+/).filter(Boolean)
          : null,
        upstream_oauth_authorization_url: upstreamOauthEnabled && isCustomProvider ? upstreamOauthAuthUrl || null : null,
        upstream_oauth_token_url: upstreamOauthEnabled && isCustomProvider ? upstreamOauthTokenUrl || null : null,
        upstream_oauth_client_id: upstreamOauthEnabled && isCustomProvider ? upstreamOauthClientId || null : null,
        upstream_oauth_client_secret: upstreamOauthEnabled && isCustomProvider && upstreamOauthClientSecret
          ? upstreamOauthClientSecret
          : undefined,
      });
      // Refresh the list views (keyed with their own prefixes) and this server's detail.
      queryClient.invalidateQueries({ queryKey: ['servers-list'] });
      queryClient.invalidateQueries({ queryKey: ['servers-minimal'] });
      queryClient.invalidateQueries({ queryKey: ['servers-basic'] });
      queryClient.invalidateQueries({ queryKey: ['server', workspaceId, server.id] });
    } catch {
      // Error already handled by mutation
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="max-w-2xl space-y-6">
      <div className="space-y-4">
        <div className="space-y-2">
          <Label htmlFor="name">{t('create.name')}</Label>
          <Input
            id="name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="bg-white"
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="description">{t('create.description')}</Label>
          <Input
            id="description"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder={t('create.descriptionPlaceholder')}
            className="bg-white"
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="branch">{t('create.branch')}</Label>
          <Input
            id="branch"
            value={branch}
            onChange={(e) => setBranch(e.target.value)}
            className="bg-white"
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="rootDirectory">{t('create.rootDirectory')}</Label>
          <p className="text-xs text-gray-500">{t('create.rootDirectoryHelp')}</p>
          <Input
            id="rootDirectory"
            value={rootDirectory}
            onChange={(e) => setRootDirectory(e.target.value)}
            placeholder="packages/mcp-server"
            className="bg-white"
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="mcpPath">{t('create.mcpPath')}</Label>
          <p className="text-xs text-gray-500">{t('create.mcpPathHelp')}</p>
          <Input
            id="mcpPath"
            value={mcpPath}
            onChange={(e) => setMcpPath(e.target.value)}
            placeholder="/mcp"
            className="bg-white"
          />
        </div>

        {server.transport === 'sse' && (
          <div className="space-y-2">
            <Label htmlFor="port">{t('create.port')}</Label>
            <p className="text-xs text-gray-500">{t('create.portHelp')}</p>
            <Input
              id="port"
              type="number"
              min={1}
              max={65535}
              value={port}
              onChange={(e) => setPort(e.target.value === '' ? '' : Number(e.target.value))}
              placeholder={String(server.runtime === 'python' ? 8000 : (server.runtime === 'go' || server.runtime === 'rust') ? 8080 : 3000)}
              className="bg-white"
            />
          </div>
        )}

        <div className="space-y-2">
          <Label htmlFor="entryCommand">{t('create.entryCommand')}</Label>
          <p className="text-xs text-gray-500">{t('create.entryCommandHelp')}</p>
          <Input
            id="entryCommand"
            value={entryCommand}
            onChange={(e) => setEntryCommand(e.target.value)}
            placeholder="python server.py"
            className="bg-white font-mono"
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="buildCommand">{t('create.buildCommand')}</Label>
          <p className="text-xs text-gray-500">{t('create.buildCommandHelp')}</p>
          <Input
            id="buildCommand"
            value={buildCommand}
            onChange={(e) => setBuildCommand(e.target.value)}
            placeholder="npm run build"
            className="bg-white font-mono"
          />
        </div>

        <MemorySelect value={memoryMb} onChange={setMemoryMb} maxMemoryMb={maxMemoryMb} />

        <div>
          <Label className="block mb-2">{t('create.visibility')}</Label>
          <div className="inline-flex p-0.5 bg-gray-200/60 rounded-[10px] border border-gray-200">
            {(['public', 'private', 'team'] as const).map((v) => (
              <button
                key={v}
                type="button"
                onClick={() => setVisibility(v)}
                className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                  visibility === v
                    ? 'bg-white text-gray-800 shadow border border-gray-100'
                    : 'text-gray-400 hover:text-gray-600'
                }`}
              >
                {t(`create.visibility${v.charAt(0).toUpperCase() + v.slice(1)}`)}
              </button>
            ))}
          </div>
        </div>

        <div>
          <Label className="block mb-2">{t('accessMode.title')}</Label>
          <p className="text-xs text-gray-500 mb-2">{t('accessMode.description')}</p>
          <div className="inline-flex p-0.5 bg-gray-200/60 rounded-[10px] border border-gray-200">
            {(['public', 'vpn_only'] as const).map((mode) => (
              <button
                key={mode}
                type="button"
                onClick={() => setAccessMode(mode)}
                className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                  accessMode === mode
                    ? 'bg-white text-gray-800 shadow border border-gray-100'
                    : 'text-gray-400 hover:text-gray-600'
                }`}
              >
                {t(`accessMode.${mode}`)}
              </button>
            ))}
          </div>
        </div>

        <div>
          <Label className="block mb-2">{t('authEnabled.title')}</Label>
          <p className="text-xs text-gray-500 mb-2">{t('authEnabled.description')}</p>
          <div className="inline-flex p-0.5 bg-gray-200/60 rounded-[10px] border border-gray-200">
            <button
              type="button"
              onClick={() => setAuthEnabled(true)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                authEnabled
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('authEnabled.on')}
            </button>
            <button
              type="button"
              onClick={() => setAuthEnabled(false)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                !authEnabled
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('authEnabled.off')}
            </button>
          </div>
        </div>

        <div>
          <Label className="block mb-2">{t('toolFilter.title')}</Label>
          <p className="text-xs text-gray-500 mb-2">{t('toolFilter.description')}</p>
          <div className="inline-flex p-0.5 bg-gray-200/60 rounded-[10px] border border-gray-200">
            <button
              type="button"
              onClick={() => setToolFilterByScope(true)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                toolFilterByScope
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('toolFilter.on')}
            </button>
            <button
              type="button"
              onClick={() => setToolFilterByScope(false)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                !toolFilterByScope
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('toolFilter.off')}
            </button>
          </div>
        </div>

        <div>
          <Label className="block mb-2">{t('toolSlim.title')}</Label>
          <p className="text-xs text-gray-500 mb-2">{t('toolSlim.description')}</p>
          <div className="inline-flex p-0.5 bg-gray-200/60 rounded-[10px] border border-gray-200">
            <button
              type="button"
              onClick={() => setToolSchemaSlim(true)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                toolSchemaSlim
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('toolSlim.on')}
            </button>
            <button
              type="button"
              onClick={() => setToolSchemaSlim(false)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                !toolSchemaSlim
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('toolSlim.off')}
            </button>
          </div>
        </div>

        <div>
          <Label className="block mb-2">{t('toolSearch.title')}</Label>
          <p className="text-xs text-gray-500 mb-2">{t('toolSearch.description')}</p>
          <div className="inline-flex p-0.5 bg-gray-200/60 rounded-[10px] border border-gray-200">
            <button
              type="button"
              onClick={() => setToolSearchMode(true)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                toolSearchMode
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('toolSearch.on')}
            </button>
            <button
              type="button"
              onClick={() => setToolSearchMode(false)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                !toolSearchMode
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('toolSearch.off')}
            </button>
          </div>
        </div>

        <div>
          <Label className="block mb-2">{t('toolCode.title')}</Label>
          <p className="text-xs text-gray-500 mb-2">{t('toolCode.description')}</p>
          <div className="inline-flex p-0.5 bg-gray-200/60 rounded-[10px] border border-gray-200">
            <button
              type="button"
              onClick={() => setToolCodeMode(true)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                toolCodeMode
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('toolCode.on')}
            </button>
            <button
              type="button"
              onClick={() => setToolCodeMode(false)}
              className={`px-2.5 py-1 text-xs font-medium rounded-[10px] transition-all ${
                !toolCodeMode
                  ? 'bg-white text-gray-800 shadow border border-gray-100'
                  : 'text-gray-400 hover:text-gray-600'
              }`}
            >
              {t('toolCode.off')}
            </button>
          </div>
        </div>

        <div>
          <div className="flex items-center justify-between mb-2">
            <Label className="block">{t('upstreamOauth.title')}</Label>
            <button
              type="button"
              role="switch"
              aria-checked={upstreamOauthEnabled}
              onClick={() => {
                setUpstreamOauthEnabled((v) => !v);
                if (upstreamOauthEnabled) {
                  setUpstreamOauthProvider('');
                  setUpstreamOauthScopes('');
                }
              }}
              className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors focus:outline-none ${
                upstreamOauthEnabled ? 'bg-violet-600' : 'bg-gray-200'
              }`}
            >
              <span className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform ${upstreamOauthEnabled ? 'translate-x-4' : 'translate-x-1'}`} />
            </button>
          </div>
          <p className="text-xs text-gray-500 mb-3">{t('upstreamOauth.toggleHelp')}</p>

          {upstreamOauthEnabled && (
            <div className="space-y-3">
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
                    value={upstreamOauthScopes}
                    onChange={(e) => setUpstreamOauthScopes(e.target.value)}
                    placeholder={t('upstreamOauth.scopesPlaceholder')}
                    className="bg-white font-mono text-xs mt-1"
                  />
                </div>
              )}

              {upstreamOauthProvider === 'custom' && (
                <div className="mt-3 p-3 rounded-lg border border-gray-200 bg-gray-50 space-y-3">
                  <p className="text-xs font-medium text-gray-700">{t('upstreamOauth.customSection')}</p>
                  <div>
                    <Label className="text-xs">{t('upstreamOauth.authUrlLabel')}</Label>
                    <Input type="url" placeholder={t('upstreamOauth.authUrlPlaceholder')} value={upstreamOauthAuthUrl} onChange={(e) => setUpstreamOauthAuthUrl(e.target.value)} className="mt-1 bg-white font-mono text-xs" />
                  </div>
                  <div>
                    <Label className="text-xs">{t('upstreamOauth.tokenUrlLabel')}</Label>
                    <Input type="url" placeholder={t('upstreamOauth.tokenUrlPlaceholder')} value={upstreamOauthTokenUrl} onChange={(e) => setUpstreamOauthTokenUrl(e.target.value)} className="mt-1 bg-white font-mono text-xs" />
                  </div>
                  <div>
                    <Label className="text-xs">{t('upstreamOauth.clientIdLabel')}</Label>
                    <Input type="text" placeholder={t('upstreamOauth.clientIdPlaceholder')} value={upstreamOauthClientId} onChange={(e) => setUpstreamOauthClientId(e.target.value)} className="mt-1 bg-white font-mono text-xs" />
                  </div>
                  <div>
                    <Label className="text-xs">{t('upstreamOauth.clientSecretLabel')}</Label>
                    <Input type="password" placeholder={server.upstream_oauth_client_id ? t('upstreamOauth.clientSecretHint') : t('upstreamOauth.clientSecretPlaceholder')} value={upstreamOauthClientSecret} onChange={(e) => setUpstreamOauthClientSecret(e.target.value)} className="mt-1 bg-white font-mono text-xs" />
                    {server.upstream_oauth_client_id && (
                      <p className="text-xs text-gray-400 mt-1">{t('upstreamOauth.clientSecretHint')}</p>
                    )}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>

      <div className="pt-4 border-t border-gray-200">
        <button
          onClick={handleSave}
          disabled={isSaving}
          className="inline-flex items-center justify-center gap-1.5 h-7 px-2.5 text-xs font-medium text-white bg-violet-600 border border-violet-700 rounded-[10px] hover:bg-violet-700 active:bg-violet-800 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {isSaving ? (
            <div className="w-3.5 h-3.5 border-2 rounded-full border-white/30 border-t-white animate-spin" />
          ) : (
            <Save className="w-3.5 h-3.5" />
          )}
          {t('detail.save')}
        </button>
      </div>

    </div>
  );
}

interface Webhook {
  id: string;
  name: string;
  webhook_url: string;
  webhook_type: string;
  events: string[];
  is_active: boolean;
  last_triggered_at: string | null;
  last_status: string | null;
  created_at: string;
}

function WebhooksTab({
  serverId,
  workspaceId,
  t,
  tCommon
}: {
  serverId: string;
  workspaceId: string;
  t: (key: string) => string;
  tCommon: (key: string) => string;
}) {
  const queryClient = useQueryClient();
  const [isAdding, setIsAdding] = useState(false);
  const [testingWebhooks, setTestingWebhooks] = useState<Set<string>>(new Set());
  const [newWebhook, setNewWebhook] = useState({
    name: '',
    webhook_url: '',
    webhook_type: 'custom',
    events: ['deploy_success', 'deploy_failure'],
    secret: '',
  });

  const { data: webhooks = [], isLoading } = useQuery<Webhook[]>({
    queryKey: ['webhooks', serverId],
    queryFn: () => api.get(`/workspaces/${workspaceId}/servers/${serverId}/webhooks`),
  });

  const createMutation = useMutation({
    mutationFn: (data: typeof newWebhook) =>
      api.post(`/workspaces/${workspaceId}/servers/${serverId}/webhooks`, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['webhooks', serverId] });
      setIsAdding(false);
      setNewWebhook({
        name: '',
        webhook_url: '',
        webhook_type: 'custom',
        events: ['deploy_success', 'deploy_failure'],
        secret: '',
      });
      toast.success(t('webhooks.createSuccess'));
    },
    onError: () => {
      toast.error(t('webhooks.createError'));
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ id, is_active }: { id: string; is_active: boolean }) =>
      api.patch(`/workspaces/${workspaceId}/servers/${serverId}/webhooks/${id}`, { is_active }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['webhooks', serverId] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) =>
      api.delete(`/workspaces/${workspaceId}/servers/${serverId}/webhooks/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['webhooks', serverId] });
      toast.success(t('webhooks.deleteSuccess'));
    },
  });

  // Async test function - allows parallel testing with toast notifications
  const handleTestWebhook = useCallback(async (webhook: Webhook) => {
    const webhookId = webhook.id;
    const webhookName = webhook.name;

    // Add to testing set
    setTestingWebhooks(prev => new Set(prev).add(webhookId));

    // Show loading toast
    const toastId = toast.loading(t('webhooks.testingWebhook').replace('{name}', webhookName));

    try {
      await api.post(`/workspaces/${workspaceId}/servers/${serverId}/webhooks/${webhookId}/test`);
      queryClient.invalidateQueries({ queryKey: ['webhooks', serverId] });
      toast.success(t('webhooks.testSuccess').replace('{name}', webhookName), { id: toastId });
    } catch {
      toast.error(t('webhooks.testError').replace('{name}', webhookName), { id: toastId });
    } finally {
      // Remove from testing set
      setTestingWebhooks(prev => {
        const next = new Set(prev);
        next.delete(webhookId);
        return next;
      });
    }
  }, [workspaceId, serverId, queryClient, t]);

  const handleEventToggle = (event: string) => {
    const events = newWebhook.events.includes(event)
      ? newWebhook.events.filter(e => e !== event)
      : [...newWebhook.events, event];
    setNewWebhook({ ...newWebhook, events });
  };

  // eventOptions - no useMemo needed, t function may be unstable
  const eventOptions = [
    { id: 'deploy_success', label: t('webhooks.eventDeploySuccess'), desc: t('webhooks.eventDeploySuccessDesc') },
    { id: 'deploy_failure', label: t('webhooks.eventDeployFailure'), desc: t('webhooks.eventDeployFailureDesc') },
    { id: 'deploy_started', label: t('webhooks.eventDeployStarted'), desc: t('webhooks.eventDeployStartedDesc') },
  ];

  if (isLoading) {
    return (
      <div className="space-y-4">
        <div className="flex justify-between items-center">
          <div className="h-8 w-32 bg-gray-100 rounded-lg animate-pulse" />
          <div className="h-9 w-36 bg-gray-100 rounded-lg animate-pulse" />
        </div>
        {[...Array(3)].map((_, i) => (
          <div key={i} className="h-20 bg-gray-100 rounded-xl animate-pulse" />
        ))}
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Add Webhook Form */}
      {isAdding ? (
        <div className="p-6 rounded-2xl bg-white border border-gray-200 shadow-sm">
          <div className="flex items-center justify-between mb-6">
            <h3 className="text-sm font-medium text-gray-400 uppercase tracking-wide">{t('webhooks.add')}</h3>
            <button onClick={() => setIsAdding(false)} className="text-gray-400 hover:text-gray-600">
              <X className="w-5 h-5" />
            </button>
          </div>
          <div className="space-y-4">
            <div>
              <Label>{t('webhooks.name')}</Label>
              <Input
                value={newWebhook.name}
                onChange={(e) => setNewWebhook({ ...newWebhook, name: e.target.value })}
                placeholder={t('webhooks.namePlaceholder')}
                className="mt-1 bg-white"
              />
            </div>
            <div>
              <Label>{t('webhooks.url')}</Label>
              <Input
                value={newWebhook.webhook_url}
                onChange={(e) => setNewWebhook({ ...newWebhook, webhook_url: e.target.value })}
                placeholder={t('webhooks.urlPlaceholder')}
                className="mt-1 bg-white"
              />
            </div>
            <div>
              <Label>{t('webhooks.type')}</Label>
              <select
                value={newWebhook.webhook_type}
                onChange={(e) => setNewWebhook({ ...newWebhook, webhook_type: e.target.value })}
                className="mt-1 w-full px-3 py-2 rounded-lg border border-gray-300 bg-white focus:outline-none focus:ring-2 focus:ring-violet-500"
              >
                <option value="custom">{t('webhooks.typeCustom')}</option>
                <option value="slack">{t('webhooks.typeSlack')}</option>
                <option value="discord">{t('webhooks.typeDiscord')}</option>
              </select>
            </div>
            <div>
              <Label className="mb-3 block">{t('webhooks.events')}</Label>
              <div className="flex gap-2">
                {eventOptions.map((event) => {
                  const isSelected = newWebhook.events.includes(event.id);
                  const colors = {
                    deploy_success: { bg: 'bg-emerald-500', light: 'bg-emerald-50 border-emerald-200 text-emerald-700' },
                    deploy_failure: { bg: 'bg-red-500', light: 'bg-red-50 border-red-200 text-red-700' },
                    deploy_started: { bg: 'bg-blue-500', light: 'bg-blue-50 border-blue-200 text-blue-700' },
                  }[event.id] || { bg: 'bg-gray-500', light: 'bg-gray-50 border-gray-200 text-gray-700' };

                  return (
                    <button
                      key={event.id}
                      type="button"
                      onClick={() => handleEventToggle(event.id)}
                      className={`relative flex items-center gap-2 px-4 py-2.5 text-sm font-medium rounded-xl border transition-all ${
                        isSelected
                          ? colors.light
                          : 'bg-white border-gray-200 text-gray-400 hover:border-gray-300 hover:text-gray-500'
                      }`}
                    >
                      <span className={`w-4 h-4 rounded-full border-2 flex items-center justify-center ${
                        isSelected ? `${colors.bg} border-transparent` : 'border-gray-300 bg-white'
                      }`}>
                        {isSelected && (
                          <Check className="w-2.5 h-2.5 text-white" strokeWidth={4} />
                        )}
                      </span>
                      {event.label}
                    </button>
                  );
                })}
              </div>
            </div>
            <div>
              <Label>{t('webhooks.secret')}</Label>
              <Input
                type="password"
                value={newWebhook.secret}
                onChange={(e) => setNewWebhook({ ...newWebhook, secret: e.target.value })}
                placeholder={t('webhooks.secretPlaceholder')}
                className="mt-1 bg-white"
              />
            </div>
            <div className="flex gap-3 pt-2">
              <Button
                onClick={() => createMutation.mutate(newWebhook)}
                disabled={!newWebhook.name || !newWebhook.webhook_url || newWebhook.events.length === 0 || createMutation.isPending}
                className="bg-violet-600 hover:bg-violet-700"
              >
                {createMutation.isPending ? tCommon('loading') : t('webhooks.add')}
              </Button>
              <Button variant="outline" onClick={() => setIsAdding(false)}>
                {tCommon('cancel')}
              </Button>
            </div>
          </div>
        </div>
      ) : (
        <button
          onClick={() => setIsAdding(true)}
          className="inline-flex items-center gap-2 px-4 py-2 rounded-lg border border-violet-300 bg-violet-50 hover:bg-violet-100 text-violet-600 transition-all"
        >
          <Plus className="w-4 h-4" />
          <span className="text-sm font-medium">{t('webhooks.add')}</span>
        </button>
      )}

      {/* Webhooks List */}
      {webhooks.length === 0 && !isAdding ? (
        <div className="py-12 text-center">
          <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-gray-100 flex items-center justify-center">
            <Link2 className="w-8 h-8 text-gray-400" />
          </div>
          <p className="text-gray-500 mb-2">{t('webhooks.empty')}</p>
          <p className="text-sm text-gray-400">{t('webhooks.emptyDesc')}</p>
        </div>
      ) : webhooks.length > 0 ? (
        <div className="space-y-3">
          {webhooks.map((webhook) => (
            <div
              key={webhook.id}
              className={`group p-4 rounded-xl bg-white border transition-all ${
                webhook.is_active ? 'border-gray-100 hover:border-gray-200 hover:shadow-md' : 'border-gray-100 opacity-60'
              }`}
            >
              <div className="flex items-center gap-4">
                <div className={`w-10 h-10 rounded-lg flex items-center justify-center ${
                  webhook.webhook_type === 'slack' ? 'bg-purple-100' :
                  webhook.webhook_type === 'discord' ? 'bg-indigo-100' : 'bg-gray-100'
                }`}>
                  {webhook.webhook_type === 'slack' ? (
                    <svg className="w-5 h-5 text-purple-600" viewBox="0 0 24 24" fill="currentColor">
                      <path d="M5.042 15.165a2.528 2.528 0 0 1-2.52 2.523A2.528 2.528 0 0 1 0 15.165a2.527 2.527 0 0 1 2.522-2.52h2.52v2.52zM6.313 15.165a2.527 2.527 0 0 1 2.521-2.52 2.527 2.527 0 0 1 2.521 2.52v6.313A2.528 2.528 0 0 1 8.834 24a2.528 2.528 0 0 1-2.521-2.522v-6.313zM8.834 5.042a2.528 2.528 0 0 1-2.521-2.52A2.528 2.528 0 0 1 8.834 0a2.528 2.528 0 0 1 2.521 2.522v2.52H8.834zM8.834 6.313a2.528 2.528 0 0 1 2.521 2.521 2.528 2.528 0 0 1-2.521 2.521H2.522A2.528 2.528 0 0 1 0 8.834a2.528 2.528 0 0 1 2.522-2.521h6.312zM18.956 8.834a2.528 2.528 0 0 1 2.522-2.521A2.528 2.528 0 0 1 24 8.834a2.528 2.528 0 0 1-2.522 2.521h-2.522V8.834zM17.688 8.834a2.528 2.528 0 0 1-2.523 2.521 2.527 2.527 0 0 1-2.52-2.521V2.522A2.527 2.527 0 0 1 15.165 0a2.528 2.528 0 0 1 2.523 2.522v6.312zM15.165 18.956a2.528 2.528 0 0 1 2.523 2.522A2.528 2.528 0 0 1 15.165 24a2.527 2.527 0 0 1-2.52-2.522v-2.522h2.52zM15.165 17.688a2.527 2.527 0 0 1-2.52-2.523 2.526 2.526 0 0 1 2.52-2.52h6.313A2.527 2.527 0 0 1 24 15.165a2.528 2.528 0 0 1-2.522 2.523h-6.313z"/>
                    </svg>
                  ) : webhook.webhook_type === 'discord' ? (
                    <svg className="w-5 h-5 text-indigo-600" viewBox="0 0 24 24" fill="currentColor">
                      <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z"/>
                    </svg>
                  ) : (
                    <Link2 className="w-5 h-5 text-gray-500" />
                  )}
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <p className="font-medium text-gray-900">{webhook.name}</p>
                    <span className={`text-xs px-2 py-0.5 rounded-full ${
                      webhook.webhook_type === 'slack' ? 'bg-purple-100 text-purple-700' :
                      webhook.webhook_type === 'discord' ? 'bg-indigo-100 text-indigo-700' : 'bg-gray-100 text-gray-600'
                    }`}>
                      {webhook.webhook_type}
                    </span>
                  </div>
                  <p className="text-sm text-gray-500 truncate">{webhook.webhook_url}</p>
                  <div className="flex items-center gap-4 mt-1">
                    <div className="flex gap-1">
                      {webhook.events.map(event => (
                        <span key={event} className="text-xs px-1.5 py-0.5 rounded bg-gray-100 text-gray-600">
                          {event.replace('deploy_', '')}
                        </span>
                      ))}
                    </div>
                    {webhook.last_triggered_at && (
                      <span className="text-xs text-gray-400">
                        Last: {formatDateTime(webhook.last_triggered_at)}
                        {webhook.last_status && (
                          <span className={webhook.last_status === 'success' ? 'text-green-600 ml-1' : 'text-red-600 ml-1'}>
                            ({webhook.last_status})
                          </span>
                        )}
                      </span>
                    )}
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => handleTestWebhook(webhook)}
                    disabled={testingWebhooks.has(webhook.id)}
                    className="px-3 py-1.5 text-sm text-violet-600 hover:bg-violet-50 rounded-lg transition-colors disabled:opacity-50"
                    title={t('webhooks.test')}
                  >
                    {testingWebhooks.has(webhook.id) ? (
                      <span className="flex items-center gap-1.5">
                        <span className="w-3 h-3 border-2 border-violet-600 border-t-transparent rounded-full animate-spin" />
                        {t('webhooks.testing')}
                      </span>
                    ) : t('webhooks.test')}
                  </button>
                  <button
                    onClick={() => toggleMutation.mutate({ id: webhook.id, is_active: !webhook.is_active })}
                    disabled={toggleMutation.isPending}
                    className={`relative w-10 h-5 rounded-full transition-colors ${
                      webhook.is_active ? 'bg-violet-600' : 'bg-gray-300'
                    }`}
                  >
                    <span className={`absolute top-0.5 w-4 h-4 bg-white rounded-full transition-transform ${
                      webhook.is_active ? 'left-[22px]' : 'left-0.5'
                    }`} />
                  </button>
                  <button
                    onClick={() => {
                      if (confirm(t('webhooks.deleteConfirm'))) {
                        deleteMutation.mutate(webhook.id);
                      }
                    }}
                    className="p-1.5 text-gray-400 hover:text-red-600 hover:bg-red-50 rounded-lg opacity-0 group-hover:opacity-100 transition-all"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

interface MetricDataPoint {
  timestamp: number;
  value: number;
  instance?: string;
}

interface AppMetrics {
  memory_used: MetricDataPoint[];
  memory_total: MetricDataPoint[];
  cpu_usage: MetricDataPoint[];
  network_rx: MetricDataPoint[];
  network_tx: MetricDataPoint[];
}

function MetricsTab({ serverId, workspaceId, serverStatus, t }: { serverId: string; workspaceId: string; serverStatus: string; t: (key: string) => string }) {
  const { data: metrics, isLoading, error, refetch } = useQuery<AppMetrics>({
    queryKey: ['servers', serverId, 'metrics'],
    queryFn: () => api.get<AppMetrics>(`/workspaces/${workspaceId}/servers/${serverId}/metrics`),
    enabled: serverStatus === 'running',
    refetchInterval: 30000, // Refresh every 30 seconds
  });

  // Helper functions (not hooks, safe to define here)
  const formatBytes = (bytes: number) => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  };

  const formatPercent = (value: number) => {
    return (value * 100).toFixed(1) + '%';
  };

  const getLatestValue = (data: MetricDataPoint[]) => {
    if (!data || data.length === 0) return 0;
    return data[data.length - 1].value;
  };

  // Compute values from metrics
  const memoryUsed = getLatestValue(metrics?.memory_used || []);
  const memoryTotal = getLatestValue(metrics?.memory_total || []);
  const cpuUsage = getLatestValue(metrics?.cpu_usage || []);
  const networkRx = getLatestValue(metrics?.network_rx || []);
  const networkTx = getLatestValue(metrics?.network_tx || []);

  // All useMemo hooks must be called before any conditional returns
  const cpuChartData = useMemo(() => {
    if (!metrics?.cpu_usage) return [];
    return metrics.cpu_usage.map((point) => ({
      time: new Date(point.timestamp * 1000).toLocaleTimeString('ja-JP', { hour: '2-digit', minute: '2-digit' }),
      cpu: parseFloat((point.value * 100).toFixed(1)),
    }));
  }, [metrics?.cpu_usage]);

  const memoryChartData = useMemo(() => {
    if (!metrics?.memory_used || !metrics?.memory_total) return [];
    const totalMap = new Map(metrics.memory_total.map((p) => [p.timestamp, p.value]));
    return metrics.memory_used.map((point) => {
      const total = totalMap.get(point.timestamp) || memoryTotal;
      return {
        time: new Date(point.timestamp * 1000).toLocaleTimeString('ja-JP', { hour: '2-digit', minute: '2-digit' }),
        used: parseFloat((point.value / (1024 * 1024)).toFixed(1)),
        total: parseFloat((total / (1024 * 1024)).toFixed(1)),
      };
    });
  }, [metrics?.memory_used, metrics?.memory_total, memoryTotal]);

  const networkChartData = useMemo(() => {
    if (!metrics?.network_rx || !metrics?.network_tx) return [];
    const txMap = new Map(metrics.network_tx.map((p) => [p.timestamp, p.value]));
    return metrics.network_rx.map((point) => ({
      time: new Date(point.timestamp * 1000).toLocaleTimeString('ja-JP', { hour: '2-digit', minute: '2-digit' }),
      rx: parseFloat((point.value / 1024).toFixed(2)),
      tx: parseFloat(((txMap.get(point.timestamp) || 0) / 1024).toFixed(2)),
    }));
  }, [metrics?.network_rx, metrics?.network_tx]);

  // Sparkline data (last 20 points for compact display)
  const cpuSparkline = useMemo(() => cpuChartData.slice(-20), [cpuChartData]);
  const memorySparkline = useMemo(() => memoryChartData.slice(-20), [memoryChartData]);
  const networkSparkline = useMemo(() => networkChartData.slice(-20), [networkChartData]);

  const cpuTrend = useMemo(() => {
    if (!metrics?.cpu_usage || metrics.cpu_usage.length < 2) return null;
    const avg = metrics.cpu_usage.reduce((sum, d) => sum + d.value, 0) / metrics.cpu_usage.length;
    const diff = (cpuUsage - avg) * 100;
    return { diff: Math.abs(diff).toFixed(1), isUp: diff > 0 };
  }, [metrics?.cpu_usage, cpuUsage]);

  const memoryTrend = useMemo(() => {
    if (!metrics?.memory_used || metrics.memory_used.length < 2) return null;
    const avg = metrics.memory_used.reduce((sum, d) => sum + d.value, 0) / metrics.memory_used.length;
    const diff = ((memoryUsed - avg) / (1024 * 1024));
    return { diff: Math.abs(diff).toFixed(1), isUp: diff > 0 };
  }, [metrics?.memory_used, memoryUsed]);

  // Freshness: servers scale to zero, so the newest sample may be old ("last seen") or
  // missing entirely (idle / never scraped). We surface that instead of rendering 0 as
  // if it were live.
  const latestTimestamp = useMemo(() => {
    const all = [
      ...(metrics?.cpu_usage || []),
      ...(metrics?.memory_used || []),
      ...(metrics?.network_rx || []),
      ...(metrics?.network_tx || []),
    ];
    if (all.length === 0) return null;
    return all.reduce((max, p) => Math.max(max, p.timestamp), 0);
  }, [metrics]);

  const hasMetricData = latestTimestamp !== null;
  const isMetricStale =
    latestTimestamp !== null && Math.floor(Date.now() / 1000) - latestTimestamp > 600; // >10min
  const lastSeenLabel =
    latestTimestamp !== null
      ? new Date(latestTimestamp * 1000).toLocaleString('ja-JP', {
          month: '2-digit',
          day: '2-digit',
          hour: '2-digit',
          minute: '2-digit',
        })
      : '';

  // Now safe to have conditional returns after all hooks are called
  if (serverStatus !== 'running') {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <div className="w-16 h-16 rounded-full bg-gray-100 flex items-center justify-center mb-4">
          <BarChart3 className="w-8 h-8 text-gray-400" />
        </div>
        <p className="text-gray-500">{t('metrics.serverNotRunning')}</p>
        <p className="text-sm text-gray-400 mt-1">{t('metrics.deployFirst')}</p>
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          {[...Array(3)].map((_, i) => (
            <div key={i} className="h-24 bg-gray-100 rounded-xl animate-pulse" />
          ))}
        </div>
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <div className="h-72 bg-gray-100 rounded-xl animate-pulse" />
          <div className="h-72 bg-gray-100 rounded-xl animate-pulse" />
          <div className="h-72 bg-gray-100 rounded-xl animate-pulse lg:col-span-2" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <p className="text-red-500">{t('metrics.loadError')}</p>
        {/* Surface the real cause (e.g. a Prometheus 401/403 or wrong org) instead of a
            silent all-zero dashboard. */}
        <p className="text-xs text-gray-400 mt-1 max-w-md break-words">
          {error instanceof Error ? error.message : String(error)}
        </p>
        <button
          onClick={() => refetch()}
          className="mt-4 px-4 py-2 text-sm text-violet-600 hover:text-violet-700"
        >
          {t('metrics.retry')}
        </button>
      </div>
    );
  }

  // No samples at all in the window: the machine is idle/scaled to zero (or was never
  // scraped). Show that honestly rather than a grid of zeros.
  if (!hasMetricData) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-center">
        <div className="w-16 h-16 rounded-full bg-gray-100 flex items-center justify-center mb-4">
          <BarChart3 className="w-8 h-8 text-gray-400" />
        </div>
        <p className="text-gray-500">{t('metrics.idle')}</p>
        <p className="text-sm text-gray-400 mt-1">{t('metrics.idleHint')}</p>
        <button
          onClick={() => refetch()}
          className="mt-4 px-4 py-2 text-sm text-violet-600 hover:text-violet-700"
        >
          {t('metrics.retry')}
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Freshness badge: distinguish a live sample from a stale "last seen" one, since
          scale-to-zero means the latest point can be hours old. */}
      <div className="flex items-center gap-2 text-xs">
        {isMetricStale ? (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-amber-50 text-amber-700 border border-amber-200">
            {t('metrics.lastSeen')} {lastSeenLabel}
          </span>
        ) : (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-green-50 text-green-700 border border-green-200">
            <span className="w-1.5 h-1.5 rounded-full bg-green-500" />
            {t('metrics.live')}
          </span>
        )}
      </div>

      {/* Metrics Summary Cards with Sparklines */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {/* CPU Card */}
        <div className="bg-white rounded-xl border border-gray-200 p-4">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium text-gray-500">{t('metrics.cpu')}</span>
            {cpuTrend && (
              <span className={`text-xs flex items-center gap-0.5 ${cpuTrend.isUp ? 'text-red-500' : 'text-green-500'}`}>
                {cpuTrend.isUp ? '↑' : '↓'} {cpuTrend.diff}%
              </span>
            )}
          </div>
          <div className="flex items-end justify-between mt-1">
            <span className="text-2xl font-semibold" style={{ color: '#323232' }}>{formatPercent(cpuUsage)}</span>
            <div className="w-24 h-10">
              {cpuSparkline.length > 1 ? (
                <ResponsiveContainer width="100%" height="100%">
                  <LineChart data={cpuSparkline}>
                    <Line
                      type="monotone"
                      dataKey="cpu"
                      stroke="#22c55e"
                      strokeWidth={1.5}
                      dot={false}
                    />
                  </LineChart>
                </ResponsiveContainer>
              ) : (
                <div className="w-full h-full flex items-center justify-center text-gray-300 text-xs">--</div>
              )}
            </div>
          </div>
        </div>

        {/* Memory Card */}
        <div className="bg-white rounded-xl border border-gray-200 p-4">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium text-gray-500">{t('metrics.memory')}</span>
            {memoryTrend && (
              <span className={`text-xs flex items-center gap-0.5 ${memoryTrend.isUp ? 'text-orange-500' : 'text-green-500'}`}>
                {memoryTrend.isUp ? '↑' : '↓'} {memoryTrend.diff} MB
              </span>
            )}
          </div>
          <div className="flex items-end justify-between mt-1">
            <div>
              <span className="text-2xl font-semibold" style={{ color: '#323232' }}>{formatBytes(memoryUsed)}</span>
              <span className="text-sm ml-1" style={{ color: '#323232' }}>/ {formatBytes(memoryTotal)}</span>
            </div>
            <div className="w-24 h-10">
              {memorySparkline.length > 1 ? (
                <ResponsiveContainer width="100%" height="100%">
                  <LineChart data={memorySparkline}>
                    <Line
                      type="monotone"
                      dataKey="used"
                      stroke="#3b82f6"
                      strokeWidth={1.5}
                      dot={false}
                    />
                  </LineChart>
                </ResponsiveContainer>
              ) : (
                <div className="w-full h-full flex items-center justify-center text-gray-300 text-xs">--</div>
              )}
            </div>
          </div>
        </div>

        {/* Network Card (Combined) */}
        <div className="bg-white rounded-xl border border-gray-200 p-4">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium text-gray-500">{t('metrics.network')}</span>
          </div>
          <div className="flex items-end justify-between mt-1">
            <div className="space-y-0.5">
              <div className="flex items-center gap-2">
                <span className="text-violet-500 text-xs">↓</span>
                <span className="text-lg font-semibold" style={{ color: '#323232' }}>{formatBytes(networkRx)}/s</span>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-orange-500 text-xs">↑</span>
                <span className="text-lg font-semibold" style={{ color: '#323232' }}>{formatBytes(networkTx)}/s</span>
              </div>
            </div>
            <div className="w-24 h-10">
              {networkSparkline.length > 1 ? (
                <ResponsiveContainer width="100%" height="100%">
                  <LineChart data={networkSparkline}>
                    <Line type="monotone" dataKey="rx" stroke="#8b5cf6" strokeWidth={1.5} dot={false} />
                    <Line type="monotone" dataKey="tx" stroke="#f97316" strokeWidth={1.5} dot={false} />
                  </LineChart>
                </ResponsiveContainer>
              ) : (
                <div className="w-full h-full flex items-center justify-center text-gray-300 text-xs">--</div>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Charts Section */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* CPU Usage Chart */}
        <div className="bg-white rounded-xl border border-gray-200 p-4">
          <h3 className="text-sm font-medium text-gray-500 mb-4">{t('metrics.cpuHistory')}</h3>
          <div className="h-64">
            {cpuChartData.length > 0 ? (
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart data={cpuChartData} margin={{ top: 10, right: 10, left: 0, bottom: 0 }}>
                  <defs>
                    <linearGradient id="cpuGradient" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%" stopColor="#22c55e" stopOpacity={0.3} />
                      <stop offset="95%" stopColor="#22c55e" stopOpacity={0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="#f0f0f0" />
                  <XAxis dataKey="time" tick={{ fontSize: 11, fill: '#9ca3af' }} tickLine={false} axisLine={false} />
                  <YAxis tick={{ fontSize: 11, fill: '#9ca3af' }} tickLine={false} axisLine={false} domain={[0, 100]} unit="%" />
                  <Tooltip
                    contentStyle={{ backgroundColor: '#fff', border: '1px solid #e5e7eb', borderRadius: '8px', fontSize: '12px' }}
                    formatter={(value) => [`${value}%`, 'CPU']}
                  />
                  <Area type="monotone" dataKey="cpu" stroke="#22c55e" strokeWidth={2} fill="url(#cpuGradient)" />
                </AreaChart>
              </ResponsiveContainer>
            ) : (
              <div className="flex items-center justify-center h-full text-gray-400 text-sm">{t('metrics.noData')}</div>
            )}
          </div>
        </div>

        {/* Memory Usage Chart */}
        <div className="bg-white rounded-xl border border-gray-200 p-4">
          <h3 className="text-sm font-medium text-gray-500 mb-4">{t('metrics.memoryHistory')}</h3>
          <div className="h-64">
            {memoryChartData.length > 0 ? (
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart data={memoryChartData} margin={{ top: 10, right: 10, left: 0, bottom: 0 }}>
                  <defs>
                    <linearGradient id="memoryGradient" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%" stopColor="#3b82f6" stopOpacity={0.3} />
                      <stop offset="95%" stopColor="#3b82f6" stopOpacity={0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="#f0f0f0" />
                  <XAxis dataKey="time" tick={{ fontSize: 11, fill: '#9ca3af' }} tickLine={false} axisLine={false} />
                  <YAxis tick={{ fontSize: 11, fill: '#9ca3af' }} tickLine={false} axisLine={false} unit=" MB" />
                  <Tooltip
                    contentStyle={{ backgroundColor: '#fff', border: '1px solid #e5e7eb', borderRadius: '8px', fontSize: '12px' }}
                    formatter={(value, name) => [`${value} MB`, name === 'used' ? t('metrics.memoryUsed') : t('metrics.memoryTotal')]}
                  />
                  <Legend formatter={(value) => (value === 'used' ? t('metrics.memoryUsed') : t('metrics.memoryTotal'))} />
                  <Area type="monotone" dataKey="used" stroke="#3b82f6" strokeWidth={2} fill="url(#memoryGradient)" />
                  <Area type="monotone" dataKey="total" stroke="#94a3b8" strokeWidth={1} strokeDasharray="4 4" fill="none" />
                </AreaChart>
              </ResponsiveContainer>
            ) : (
              <div className="flex items-center justify-center h-full text-gray-400 text-sm">{t('metrics.noData')}</div>
            )}
          </div>
        </div>

        {/* Network I/O Chart */}
        <div className="bg-white rounded-xl border border-gray-200 p-4 lg:col-span-2">
          <h3 className="text-sm font-medium text-gray-500 mb-4">{t('metrics.networkHistory')}</h3>
          <div className="h-64">
            {networkChartData.length > 0 ? (
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart data={networkChartData} margin={{ top: 10, right: 10, left: 0, bottom: 0 }}>
                  <defs>
                    <linearGradient id="rxGradient" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%" stopColor="#8b5cf6" stopOpacity={0.3} />
                      <stop offset="95%" stopColor="#8b5cf6" stopOpacity={0} />
                    </linearGradient>
                    <linearGradient id="txGradient" x1="0" y1="0" x2="0" y2="1">
                      <stop offset="5%" stopColor="#f97316" stopOpacity={0.3} />
                      <stop offset="95%" stopColor="#f97316" stopOpacity={0} />
                    </linearGradient>
                  </defs>
                  <CartesianGrid strokeDasharray="3 3" stroke="#f0f0f0" />
                  <XAxis dataKey="time" tick={{ fontSize: 11, fill: '#9ca3af' }} tickLine={false} axisLine={false} />
                  <YAxis tick={{ fontSize: 11, fill: '#9ca3af' }} tickLine={false} axisLine={false} unit=" KB/s" />
                  <Tooltip
                    contentStyle={{ backgroundColor: '#fff', border: '1px solid #e5e7eb', borderRadius: '8px', fontSize: '12px' }}
                    formatter={(value, name) => [`${value} KB/s`, name === 'rx' ? t('metrics.inbound') : t('metrics.outbound')]}
                  />
                  <Legend formatter={(value) => (value === 'rx' ? t('metrics.inbound') : t('metrics.outbound'))} />
                  <Area type="monotone" dataKey="rx" stroke="#8b5cf6" strokeWidth={2} fill="url(#rxGradient)" />
                  <Area type="monotone" dataKey="tx" stroke="#f97316" strokeWidth={2} fill="url(#txGradient)" />
                </AreaChart>
              </ResponsiveContainer>
            ) : (
              <div className="flex items-center justify-center h-full text-gray-400 text-sm">{t('metrics.noData')}</div>
            )}
          </div>
        </div>
      </div>

      {/* Refresh button with tooltip */}
      <div className="flex justify-end items-center gap-2">
        <div className="relative group">
          <button className="p-1.5 text-gray-400 hover:text-gray-600 rounded-full hover:bg-gray-100 transition-colors">
            <HelpCircle className="w-4 h-4" />
          </button>
          <div className="absolute bottom-full right-0 mb-2 w-64 p-3 bg-gray-900 text-white text-xs rounded-lg opacity-0 invisible group-hover:opacity-100 group-hover:visible transition-all duration-200 z-10">
            <p className="font-medium mb-1">{t('metrics.infoTitle')}</p>
            <p className="text-gray-300">{t('metrics.infoDescription')}</p>
            <div className="absolute bottom-0 right-4 translate-y-1/2 rotate-45 w-2 h-2 bg-gray-900" />
          </div>
        </div>
        <button
          onClick={() => refetch()}
          className="inline-flex items-center gap-2 px-3 py-1.5 text-sm text-gray-600 hover:text-gray-800 hover:bg-gray-100 rounded-lg transition-colors"
        >
          <RefreshCw className="w-4 h-4" />
          {t('metrics.refresh')}
        </button>
      </div>
    </div>
  );
}

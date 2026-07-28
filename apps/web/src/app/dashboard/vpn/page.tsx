'use client';

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import { api } from '@/lib/api';
import { WireGuardPeer, WireGuardConfig } from '@/types';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Shield, Check, Trash2 } from 'lucide-react';
import { useWorkspace } from '@/hooks/use-workspace';

export default function VPNPage() {
  const t = useTranslations('vpn');
  const tCommon = useTranslations('common');
  const queryClient = useQueryClient();

  const [showForm, setShowForm] = useState(false);
  const [peerName, setPeerName] = useState('');
  const [generatedConfig, setGeneratedConfig] = useState<WireGuardConfig | null>(null);
  const [copied, setCopied] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  const { activeWorkspace, isLoading: isLoadingWorkspaces } = useWorkspace();

  const workspaceId = activeWorkspace?.id;

  const { data: peers, isLoading: isLoadingPeers } = useQuery<WireGuardPeer[]>({
    queryKey: ['wireguard-peers', workspaceId],
    queryFn: () => api.get(`/workspaces/${workspaceId}/wireguard`),
    enabled: !!workspaceId,
  });

  const createMutation = useMutation({
    mutationFn: (data: { name: string; region: string }) =>
      api.post<WireGuardConfig>(`/workspaces/${workspaceId}/wireguard`, data),
    onSuccess: (config) => {
      setGeneratedConfig(config);
      setShowForm(false);
      setPeerName('');
      queryClient.invalidateQueries({ queryKey: ['wireguard-peers', workspaceId] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (name: string) =>
      api.delete(`/workspaces/${workspaceId}/wireguard/${encodeURIComponent(name)}`),
    onSuccess: () => {
      setDeleteTarget(null);
      queryClient.invalidateQueries({ queryKey: ['wireguard-peers', workspaceId] });
    },
  });

  const handleCreate = (e: React.FormEvent) => {
    e.preventDefault();
    if (!peerName.trim()) return;
    // Fixed to US East (iad)
    createMutation.mutate({ name: peerName, region: 'iad' });
  };

  const handleCopy = () => {
    if (generatedConfig) {
      navigator.clipboard.writeText(generatedConfig.config_file);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const handleDownload = () => {
    if (generatedConfig) {
      const blob = new Blob([generatedConfig.config_file], { type: 'text/plain' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `${generatedConfig.peer_name}.conf`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    }
  };

  const isLoadingData = isLoadingWorkspaces || isLoadingPeers;
  const hasPeers = peers && peers.length > 0;

  return (
    <div className="space-y-8">
      {/* Header */}
      <div className="flex items-center justify-end">
        {!isLoadingData && hasPeers && !showForm && !generatedConfig && (
          <Button size="sm" onClick={() => setShowForm(true)}>
            + {t('createConnection')}
          </Button>
        )}
      </div>

      {/* Generated Config */}
      {generatedConfig && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 text-green-600 text-sm font-medium">
              <Check className="w-4 h-4" />
              {t('configGenerated')}
            </div>
            <button onClick={() => setGeneratedConfig(null)} className="text-gray-400 hover:text-gray-600 text-sm">
              {tCommon('close')}
            </button>
          </div>

          <div className="text-sm text-amber-700 bg-amber-50 border border-amber-200 px-3 py-2 rounded">
            {t('importantNote')} {t('configOnlyOnce')}
          </div>

          <div className="flex gap-6 text-sm">
            <div>
              <span className="text-gray-500">Name:</span>
              <span className="ml-2 font-mono">{generatedConfig.peer_name}</span>
            </div>
            <div>
              <span className="text-gray-500">IP:</span>
              <span className="ml-2 font-mono">{generatedConfig.peer_ip}</span>
            </div>
          </div>

          <pre className="text-xs font-mono bg-gray-900 text-gray-100 p-4 rounded overflow-x-auto">
            {generatedConfig.config_file}
          </pre>

          <div className="flex gap-2">
            <Button size="sm" variant="outline" onClick={handleCopy}>
              {copied ? tCommon('copied') : tCommon('copy')}
            </Button>
            <Button size="sm" onClick={handleDownload}>
              {t('download')}
            </Button>
          </div>
        </div>
      )}

      {/* Loading State */}
      {isLoadingData && (
        <div>
          <div className="grid grid-cols-[1fr_1fr_1fr_2rem] gap-2 pb-2 border-b border-gray-200 mb-1">
            {[...Array(3)].map((_, i) => (
              <div key={i} className="h-3 w-16 bg-gray-200 rounded animate-pulse" />
            ))}
            <div />
          </div>
          <div className="divide-y divide-gray-100">
            {[...Array(3)].map((_, i) => (
              <div key={i} className="grid grid-cols-[1fr_1fr_1fr_2rem] gap-2 py-3">
                <div className="h-4 w-24 bg-gray-200 rounded animate-pulse" />
                <div className="h-4 w-16 bg-gray-100 rounded animate-pulse" />
                <div className="h-4 w-28 bg-gray-100 rounded animate-pulse" />
                <div />
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Connections Table */}
      {!isLoadingData && hasPeers && !generatedConfig && (
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-gray-500 border-b">
              <th className="pb-2 font-medium">Name</th>
              <th className="pb-2 font-medium">Region</th>
              <th className="pb-2 font-medium">IP</th>
              <th className="pb-2 font-medium w-8"></th>
            </tr>
          </thead>
          <tbody className="divide-y">
            {peers.map((peer) => (
              <tr key={peer.name} className="group">
                <td className="py-3 font-medium text-gray-900">{peer.name}</td>
                <td className="py-3 text-gray-600">
                  <span className="fi fi-us mr-2"></span>
                  US East
                </td>
                <td className="py-3 font-mono text-gray-500">{peer.peer_ip}</td>
                <td className="py-3">
                  <button
                    onClick={() => setDeleteTarget(peer.name)}
                    className="text-gray-300 hover:text-red-500 opacity-0 group-hover:opacity-100 transition-opacity"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {/* Create Form */}
      {!isLoadingData && (showForm || !hasPeers) && !generatedConfig && (
        <form onSubmit={handleCreate} className="space-y-5 max-w-md">
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">{t('connectionName')}</label>
            <Input
              value={peerName}
              onChange={(e) => setPeerName(e.target.value)}
              placeholder={t('connectionNamePlaceholder')}
            />
            <p className="text-xs text-gray-500 mt-1">{t('connectionNameHint')}</p>
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-700 mb-2">Region</label>
            <div className="flex items-center gap-3 px-4 py-3 rounded-xl border-2 border-gray-100 bg-gray-50 text-gray-700">
              <span className="fi fi-us text-xl"></span>
              <span>US East (Virginia)</span>
            </div>
          </div>

          {createMutation.isError && (
            <p className="text-sm text-red-600">{t('createError')}</p>
          )}

          <div className="flex gap-2">
            {showForm && hasPeers && (
              <Button type="button" variant="outline" onClick={() => setShowForm(false)}>
                {tCommon('cancel')}
              </Button>
            )}
            <Button type="submit" disabled={!peerName.trim() || !workspaceId || createMutation.isPending}>
              {createMutation.isPending ? t('creating') : t('create')}
            </Button>
          </div>
        </form>
      )}

      {/* Info */}
      <p className="text-sm text-gray-500">
        {t('wireGuardInfo')}{' '}
        <a href="https://www.wireguard.com/install/" target="_blank" rel="noopener noreferrer" className="text-violet-600 hover:underline">
          wireguard.com/install
        </a>
      </p>

      {/* Delete Modal */}
      {deleteTarget && (
        <div className="fixed inset-0 bg-black/30 flex items-center justify-center z-50">
          <div className="bg-white rounded-2xl p-5 max-w-xs w-full mx-4">
            <p className="text-sm text-gray-700 mb-4">
              {t('deleteConfirmMessage', { name: deleteTarget })}
            </p>
            <div className="flex justify-end gap-2">
              <Button size="sm" variant="outline" onClick={() => setDeleteTarget(null)}>
                {tCommon('cancel')}
              </Button>
              <Button
                size="sm"
                variant="destructive"
                onClick={() => deleteMutation.mutate(deleteTarget)}
                disabled={deleteMutation.isPending}
              >
                {tCommon('delete')}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

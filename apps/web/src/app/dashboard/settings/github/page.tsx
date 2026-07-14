'use client';

import { useEffect } from 'react';
import { useTranslations } from 'next-intl';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useSearchParams } from 'next/navigation';
import { Github, Plus, Trash2, Star, ExternalLink, Check, AlertCircle, ChevronLeft } from 'lucide-react';
import { SiGithub } from 'react-icons/si';
import Link from 'next/link';
import { Button } from '@/components/ui/button';
import {
  AlertDialog,
  AlertDialogTrigger,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogFooter,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogAction,
  AlertDialogCancel,
} from '@/components/ui/alert-dialog';
import { getLinkedAccounts, unlinkAccount, setPrimaryAccount, getLinkUrl, LinkedGitHubAccount } from '@/lib/github-api';
import { useSetPageHeader } from '../../page-header';
import { formatDate } from '@/lib/date-utils';

export default function GitHubSettingsPage() {
  const t = useTranslations('github');
  const tCommon = useTranslations('common');
  const queryClient = useQueryClient();
  const searchParams = useSearchParams();
  useSetPageHeader(t('settingsTitle'), <SiGithub className="w-4 h-4" />);

  const success = searchParams.get('success');
  const error = searchParams.get('error');

  // Refetch accounts when returning from OAuth flow
  useEffect(() => {
    if (success === 'github_linked') {
      queryClient.invalidateQueries({ queryKey: ['linked-github-accounts'] });
    }
  }, [success, queryClient]);

  const { data: accounts, isLoading } = useQuery<LinkedGitHubAccount[]>({
    queryKey: ['linked-github-accounts'],
    queryFn: getLinkedAccounts,
  });

  const unlinkMutation = useMutation({
    mutationFn: unlinkAccount,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['linked-github-accounts'] });
    },
  });

  const setPrimaryMutation = useMutation({
    mutationFn: setPrimaryAccount,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['linked-github-accounts'] });
    },
  });

  const handleAddAccount = () => {
    window.location.href = getLinkUrl('/dashboard/settings/github');
  };

  return (
    <div className="max-w-2xl">
      {/* Back Link */}
      <Link
        href="/dashboard/settings"
        className="inline-flex items-center gap-1 text-sm text-gray-500 hover:text-gray-700 mb-4 transition-colors"
      >
        <ChevronLeft className="w-4 h-4" />
        {t('backToSettings')}
      </Link>

      {/* Header */}
      <div className="flex items-center justify-end mb-6">
        <Button
          onClick={handleAddAccount}
          className="gap-2"
        >
          <Plus className="w-4 h-4" />
          {t('addAccount')}
        </Button>
      </div>

      {/* Success Message */}
      {success === 'github_linked' && (
        <div className="mb-6 p-4 rounded-xl bg-green-50 border border-green-200 flex items-center gap-3">
          <div className="w-8 h-8 rounded-full bg-green-100 flex items-center justify-center flex-shrink-0">
            <Check className="w-4 h-4 text-green-600" />
          </div>
          <p className="text-green-700 font-medium">{t('accountLinkedSuccess')}</p>
        </div>
      )}

      {/* Error Message */}
      {error && (
        <div className="mb-6 p-4 rounded-xl bg-red-50 border border-red-200 flex items-center gap-3">
          <div className="w-8 h-8 rounded-full bg-red-100 flex items-center justify-center flex-shrink-0">
            <AlertCircle className="w-4 h-4 text-red-600" />
          </div>
          <p className="text-red-700 font-medium">{t('accountLinkError')}: {error}</p>
        </div>
      )}

      {/* Description */}
      <p className="text-gray-600 mb-6">{t('settingsDescription')}</p>

      {/* Accounts List */}
      {isLoading ? (
        <div className="flex items-center justify-center py-12">
          <div className="w-8 h-8 border-4 rounded-full border-gray-200 border-t-violet-600 animate-spin" />
        </div>
      ) : accounts && accounts.length > 0 ? (
        <div className="space-y-3">
          {accounts.map((account) => (
            <div
              key={account.id}
              className="flex items-center gap-4 p-4 rounded-xl bg-white border border-gray-200 hover:border-gray-300 transition-colors"
            >
              {/* Avatar */}
              {account.github_avatar_url ? (
                <img
                  src={account.github_avatar_url}
                  alt={account.github_username}
                  className="w-12 h-12 rounded-xl"
                />
              ) : (
                <div className="w-12 h-12 rounded-xl bg-gray-900 flex items-center justify-center">
                  <SiGithub className="w-6 h-6 text-white" />
                </div>
              )}

              {/* Info */}
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="font-semibold text-gray-900 truncate">
                    {account.github_username}
                  </span>
                  {account.is_primary && (
                    <span className="inline-flex items-center gap-1 px-2 py-0.5 text-xs font-medium bg-violet-100 text-violet-700 rounded-full">
                      <Star className="w-3 h-3" />
                      {t('primary')}
                    </span>
                  )}
                </div>
                <p className="text-sm text-gray-500 truncate">
                  {t('connectedOn', { date: formatDate(account.created_at) })}
                </p>
              </div>

              {/* Actions */}
              <div className="flex items-center gap-2">
                {!account.is_primary && accounts.length > 1 && (
                  <button
                    onClick={() => setPrimaryMutation.mutate(account.id)}
                    disabled={setPrimaryMutation.isPending}
                    className="px-3 py-1.5 text-sm text-gray-600 hover:text-violet-600 hover:bg-violet-50 rounded-lg transition-colors"
                  >
                    {t('setAsPrimary')}
                  </button>
                )}
                <a
                  href={`https://github.com/${account.github_username}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="p-2 text-gray-400 hover:text-gray-600 hover:bg-gray-100 rounded-lg transition-colors"
                  title={t('viewOnGitHub')}
                >
                  <ExternalLink className="w-4 h-4" />
                </a>
                <AlertDialog>
                  <AlertDialogTrigger asChild>
                    <button
                      disabled={unlinkMutation.isPending}
                      className="p-2 text-gray-400 hover:text-red-600 hover:bg-red-50 rounded-lg transition-colors"
                      title={t('unlink')}
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </AlertDialogTrigger>
                  <AlertDialogContent className="max-w-[calc(100%-2rem)] sm:max-w-md mx-4 sm:mx-auto">
                    <AlertDialogHeader className="space-y-4">
                      <AlertDialogTitle className="flex items-center justify-center gap-2 text-gray-400">
                        <AlertCircle className="w-5 h-5 text-red-500" />
                        {t('unlink')}
                      </AlertDialogTitle>
                      <AlertDialogDescription className="text-center space-y-3">
                        <p>{t('confirmUnlink', { username: account.github_username })}</p>
                      </AlertDialogDescription>
                    </AlertDialogHeader>
                    <AlertDialogFooter>
                      <AlertDialogCancel className="flex-1">{tCommon('cancel')}</AlertDialogCancel>
                      <AlertDialogAction
                        onClick={() => unlinkMutation.mutate(account.id)}
                        className="flex-1 bg-red-600 hover:bg-red-700"
                      >
                        {t('unlink')}
                      </AlertDialogAction>
                    </AlertDialogFooter>
                  </AlertDialogContent>
                </AlertDialog>
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="text-center py-12 px-6 rounded-xl border-2 border-dashed border-gray-200">
          <div className="w-16 h-16 mx-auto mb-4 rounded-2xl bg-gray-100 flex items-center justify-center">
            <SiGithub className="w-8 h-8 text-gray-400" />
          </div>
          <h3 className="font-semibold text-gray-900 mb-2">{t('noAccountsTitle')}</h3>
          <p className="text-gray-500 mb-4">{t('noAccountsDescription')}</p>
          <Button onClick={handleAddAccount} className="gap-2">
            <Plus className="w-4 h-4" />
            {t('connectGitHub')}
          </Button>
        </div>
      )}

      {/* Mutation Error */}
      {(unlinkMutation.isError || setPrimaryMutation.isError) && (
        <div className="mt-4 p-4 rounded-xl bg-red-50 border border-red-200 flex items-center gap-3">
          <AlertCircle className="w-5 h-5 text-red-600 flex-shrink-0" />
          <p className="text-red-700">{t('operationFailed')}</p>
        </div>
      )}

      {/* Help Section */}
      <div className="mt-8 p-4 rounded-xl bg-gray-50 border border-gray-200">
        <h3 className="font-medium text-gray-900 mb-2">{t('helpTitle')}</h3>
        <ul className="text-sm text-gray-600 space-y-2">
          <li className="flex items-start gap-2">
            <span className="text-violet-500">•</span>
            {t('helpItem1')}
          </li>
          <li className="flex items-start gap-2">
            <span className="text-violet-500">•</span>
            {t('helpItem2')}
          </li>
          <li className="flex items-start gap-2">
            <span className="text-violet-500">•</span>
            {t('helpItem3')}
          </li>
        </ul>
      </div>
    </div>
  );
}

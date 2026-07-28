'use client';

import { useState } from 'react';
import { useTranslations } from 'next-intl';
import { useAuth } from '@/hooks/use-auth';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { getApiErrorMessage, NotificationSettings } from '@/types';
import { LocaleSwitcher } from '@/components/locale-switcher';
import { Edit, LogOut, AlertTriangle, Trash2, AlertCircle, XCircle, Github, ChevronRight } from 'lucide-react';
import Link from 'next/link';


export default function SettingsPage() {
  const t = useTranslations('settings');
  const tCommon = useTranslations('common');
  const { user, logout, refreshUser } = useAuth();
  const queryClient = useQueryClient();
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [confirmText, setConfirmText] = useState('');

  // Profile editing state
  const [isEditingProfile, setIsEditingProfile] = useState(false);
  const [profileName, setProfileName] = useState(user?.name || '');

  // Profile save mutation
  const profileMutation = useMutation({
    mutationFn: (name: string) => api.patch('/auth/profile', { name }),
    onSuccess: () => {
      refreshUser?.();
      setIsEditingProfile(false);
    },
  });

  // Account delete mutation
  const deleteMutation = useMutation({
    mutationFn: () => api.delete('/auth/account'),
    onSuccess: () => {
      logout();
    },
  });

  // Notification settings
  const { data: notificationSettings } = useQuery<NotificationSettings>({
    queryKey: ['notificationSettings'],
    queryFn: () => api.get('/user/notifications'),
    initialData: {
      email_deploy_success: true,
      email_deploy_failure: true,
      email_server_down: true,
      email_weekly_report: false,
    },
  });

  const notificationMutation = useMutation({
    mutationFn: (settings: Partial<NotificationSettings>) =>
      api.patch('/user/notifications', settings),
    // Optimistic update for instant feedback
    onMutate: async (newSettings) => {
      // Cancel any outgoing refetches
      await queryClient.cancelQueries({ queryKey: ['notificationSettings'] });

      // Snapshot the previous value
      const previousSettings = queryClient.getQueryData<NotificationSettings>(['notificationSettings']);

      // Optimistically update to the new value
      queryClient.setQueryData<NotificationSettings>(['notificationSettings'], (old) => ({
        ...old!,
        ...newSettings,
      }));

      // Return context with the previous value for rollback
      return { previousSettings };
    },
    onError: (_err, _newSettings, context) => {
      // Rollback to the previous value on error
      if (context?.previousSettings) {
        queryClient.setQueryData(['notificationSettings'], context.previousSettings);
      }
    },
    onSettled: () => {
      // Always refetch after error or success to ensure consistency
      queryClient.invalidateQueries({ queryKey: ['notificationSettings'] });
    },
  });

  const handleSaveProfile = () => {
    if (!profileName.trim()) return;
    profileMutation.mutate(profileName.trim());
  };

  const handleNotificationToggle = (key: keyof NotificationSettings) => {
    if (!notificationSettings) return;
    notificationMutation.mutate({
      [key]: !notificationSettings[key],
    });
  };

  const handleReconnectGithub = () => {
    window.location.href = `${process.env.NEXT_PUBLIC_API_URL || ''}/auth/github?reconnect=true`;
  };

  const handleDeleteAccount = () => {
    if (confirmText !== 'DELETE') return;
    deleteMutation.mutate();
  };

  const cancelDelete = () => {
    setShowDeleteConfirm(false);
    setConfirmText('');
    deleteMutation.reset();
  };

  return (
    <div className="max-w-2xl">
      {/* Profile Section */}
      <section className="mb-8 sm:mb-10">
        <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-3 sm:mb-4">{t('account.title')}</h2>

        <div className="p-4 sm:p-5 rounded-2xl bg-gradient-to-r from-gray-50 to-slate-50 border border-gray-100">
          <div className="flex items-center gap-4 sm:gap-5 mb-4">
            {user?.avatar_url ? (
              <img
                src={user.avatar_url}
                alt={user.name}
                className="w-12 h-12 sm:w-16 sm:h-16 rounded-2xl ring-4 ring-white shadow-lg flex-shrink-0"
              />
            ) : (
              <div className="w-12 h-12 sm:w-16 sm:h-16 rounded-2xl bg-gradient-to-br from-violet-400 to-purple-500 flex items-center justify-center ring-4 ring-white shadow-lg flex-shrink-0">
                <span className="text-white font-bold text-lg sm:text-xl">{user?.name?.charAt(0) || '?'}</span>
              </div>
            )}
            <div className="flex-1">
              {isEditingProfile ? (
                <div className="space-y-3">
                  <div>
                    <Label htmlFor="profileName" className="text-xs text-gray-500">{t('account.name')}</Label>
                    <Input
                      id="profileName"
                      value={profileName}
                      onChange={(e) => setProfileName(e.target.value)}
                      className="mt-1"
                      placeholder={t('account.namePlaceholder')}
                    />
                  </div>
                  {profileMutation.isError && (
                    <p className="text-sm text-red-600">{getApiErrorMessage(profileMutation.error)}</p>
                  )}
                  <div className="flex gap-2">
                    <Button size="sm" onClick={handleSaveProfile} disabled={profileMutation.isPending}>
                      {profileMutation.isPending ? tCommon('loading') : tCommon('save')}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => {
                      setIsEditingProfile(false);
                      setProfileName(user?.name || '');
                      profileMutation.reset();
                    }}>
                      {tCommon('cancel')}
                    </Button>
                  </div>
                </div>
              ) : (
                <>
                  <div className="font-semibold text-lg text-gray-900">{user?.name}</div>
                  <div className="text-gray-500">{user?.email}</div>
                </>
              )}
            </div>
            {!isEditingProfile && (
              <button
                onClick={() => setIsEditingProfile(true)}
                className="p-2 text-gray-400 hover:text-gray-600 hover:bg-white rounded-lg transition-colors"
              >
                <Edit className="w-4 h-4" />
              </button>
            )}
          </div>
        </div>
      </section>

      {/* GitHub Connection */}
      <section className="mb-8 sm:mb-10">
        <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-3 sm:mb-4">{t('account.githubConnection')}</h2>

        <Link
          href="/dashboard/settings/github"
          className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 sm:gap-4 p-4 rounded-xl bg-white border border-gray-200 hover:border-gray-300 transition-colors group"
        >
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-lg bg-gray-900 flex items-center justify-center">
              <Github className="w-5 h-5 text-white" />
            </div>
            <div>
              <div className="font-medium text-gray-900">{t('account.githubAccounts')}</div>
              <div className="text-sm text-gray-500">{t('account.githubAccountsDesc')}</div>
            </div>
          </div>
          <ChevronRight className="w-5 h-5 text-gray-400 group-hover:text-gray-600 transition-colors" />
        </Link>
      </section>

      {/* Email Notifications */}
      <section className="mb-8 sm:mb-10">
        <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-3 sm:mb-4">{t('notifications.title')}</h2>

        <div className="space-y-3">
          <NotificationToggle
            label={t('notifications.deploySuccess')}
            description={t('notifications.deploySuccessDesc')}
            checked={notificationSettings?.email_deploy_success ?? true}
            onChange={() => handleNotificationToggle('email_deploy_success')}
            disabled={notificationMutation.isPending}
          />
          <NotificationToggle
            label={t('notifications.deployFailure')}
            description={t('notifications.deployFailureDesc')}
            checked={notificationSettings?.email_deploy_failure ?? true}
            onChange={() => handleNotificationToggle('email_deploy_failure')}
            disabled={notificationMutation.isPending}
          />
          <NotificationToggle
            label={t('notifications.serverDown')}
            description={t('notifications.serverDownDesc')}
            checked={notificationSettings?.email_server_down ?? true}
            onChange={() => handleNotificationToggle('email_server_down')}
            disabled={notificationMutation.isPending}
          />
          <NotificationToggle
            label={t('notifications.weeklyReport')}
            description={t('notifications.weeklyReportDesc')}
            checked={notificationSettings?.email_weekly_report ?? false}
            onChange={() => handleNotificationToggle('email_weekly_report')}
            disabled={notificationMutation.isPending}
          />
        </div>
      </section>

      {/* Language Settings */}
      <section className="mb-8 sm:mb-10">
        <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-3 sm:mb-4">{t('language.title')}</h2>
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 p-4 rounded-xl bg-white border border-gray-200">
          <p className="text-gray-600 text-sm sm:text-base">{t('language.description')}</p>
          <LocaleSwitcher />
        </div>
      </section>

      {/* Sign Out */}
      <section className="mb-8 sm:mb-10">
        <h2 className="text-sm font-medium text-gray-500 uppercase tracking-wider mb-3 sm:mb-4">{t('account.signOut')}</h2>

        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 p-4 rounded-xl bg-white border border-gray-200">
          <p className="text-gray-600 text-sm sm:text-base">{t('account.signOutDesc')}</p>
          <Button variant="outline" onClick={() => logout()} className="gap-2 self-end sm:self-auto">
            <LogOut className="w-4 h-4" />
            {t('account.signOut')}
          </Button>
        </div>
      </section>

      {/* Danger Zone */}
      <section>
        <h2 className="text-sm font-medium text-red-500 uppercase tracking-wider mb-3 sm:mb-4 flex items-center gap-2">
          <AlertTriangle className="w-4 h-4" />
          {t('danger.title')}
        </h2>

        <div className="p-4 sm:p-5 rounded-xl border-2 border-dashed border-red-200 bg-red-50/50">
          <h3 className="font-medium text-gray-900 mb-2">{t('danger.deleteAccount')}</h3>
          <p className="text-sm text-gray-600 mb-3 sm:mb-4">{t('danger.deleteAccountDetail')}</p>

          {!showDeleteConfirm ? (
            <button
              onClick={() => setShowDeleteConfirm(true)}
              className="text-red-600 text-sm font-medium hover:text-red-700 transition-colors flex items-center gap-2"
            >
              <Trash2 className="w-4 h-4" />
              {t('danger.deleteAccount')}
            </button>
          ) : (
            <div className="mt-4 p-4 rounded-xl bg-white border border-red-200">
              <div className="flex items-start gap-3 mb-4">
                <div className="w-8 h-8 rounded-full bg-red-100 flex items-center justify-center flex-shrink-0">
                  <AlertCircle className="w-4 h-4 text-red-600" />
                </div>
                <div>
                  <p className="font-medium text-red-800">{t('danger.deleteConfirm')}</p>
                  <p className="text-sm text-gray-600 mt-1">{t('danger.typeDelete')}</p>
                </div>
              </div>

              <Input
                value={confirmText}
                onChange={(e) => setConfirmText(e.target.value)}
                placeholder="DELETE"
                className="mb-4 border-red-200 focus:border-red-400 focus:ring-red-400"
              />

              {deleteMutation.isError && (
                <p className="text-sm text-red-600 mb-4 flex items-center gap-2">
                  <XCircle className="w-4 h-4" />
                  {t('danger.deleteFailed')}
                </p>
              )}

              <div className="flex gap-3">
                <Button
                  variant="destructive"
                  onClick={handleDeleteAccount}
                  disabled={confirmText !== 'DELETE' || deleteMutation.isPending}
                  className="gap-2"
                >
                  {deleteMutation.isPending ? (
                    <div className="w-4 h-4 border-2 rounded-full border-white/30 border-t-white animate-spin" />
                  ) : (
                    <>
                      <Trash2 className="w-4 h-4" />
                      {t('danger.permanentlyDelete')}
                    </>
                  )}
                </Button>
                <Button variant="outline" onClick={cancelDelete}>
                  {tCommon('cancel')}
                </Button>
              </div>
            </div>
          )}
        </div>
      </section>
    </div>
  );
}

function NotificationToggle({
  label,
  description,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: () => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-3 p-4 rounded-xl bg-white border border-gray-200">
      <div className="min-w-0 flex-1">
        <div className="font-medium text-gray-900 text-sm sm:text-base">{label}</div>
        <div className="text-xs sm:text-sm text-gray-500">{description}</div>
      </div>
      <button
        onClick={onChange}
        disabled={disabled}
        className={`relative w-11 h-6 rounded-full transition-colors ${
          checked ? 'bg-violet-600' : 'bg-gray-300'
        } ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`}
      >
        <span
          className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow transition-transform ${
            checked ? 'translate-x-5' : 'translate-x-0'
          }`}
        />
      </button>
    </div>
  );
}


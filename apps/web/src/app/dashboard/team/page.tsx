'use client';

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import Link from 'next/link';
import { Users, User, Plus, AlertCircle, X } from 'lucide-react';
import { api } from '@/lib/api';
import { findPlanLimits } from '@/lib/plans';
import { TeamMember, AddMemberRequest, WorkspaceRole, getApiErrorCode, getApiErrorMessage } from '@/types';
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
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog';

interface PlanLimits {
  max_team_members: number;
}

interface Plan {
  plan: string;
  limits: PlanLimits;
}

export default function TeamPage() {
  const t = useTranslations('team');
  const tCommon = useTranslations('common');
  const [showCreate, setShowCreate] = useState(false);

  const { activeWorkspace: currentWorkspace, isLoading: isLoadingWorkspaces } = useWorkspace();
  const isErrorWorkspaces = false;
  const workspaceId = currentWorkspace?.id;

  const { data: members, isLoading: isLoadingMembers, isError: isErrorMembers } = useQuery<TeamMember[]>({
    queryKey: ['workspaces', workspaceId, 'members'],
    queryFn: () => api.get(`/workspaces/${workspaceId}/members`),
    enabled: !!workspaceId,
  });

  const { data: plans } = useQuery<Plan[]>({
    queryKey: ['billing-plans'],
    queryFn: () => api.get('/billing/plans'),
  });

  const currentPlanLimits = findPlanLimits(plans, currentWorkspace?.plan);
  const maxMembers = currentPlanLimits?.max_team_members || 1;
  const currentMemberCount = isErrorMembers ? 0 : (members?.length || 0);
  const isAtLimit = !isErrorMembers && currentMemberCount >= maxMembers;

  const isLoading = isLoadingWorkspaces || isLoadingMembers;

  return (
    <div>
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 mb-6 sm:mb-8">
        <div className="flex flex-col sm:flex-row sm:items-center gap-2 sm:gap-4" />
        <div className="flex items-center gap-2 sm:gap-3 self-start sm:self-auto">
          {/* Usage Badge */}
          <div className="flex items-center gap-1.5 sm:gap-2 px-2.5 sm:px-3 py-1 sm:py-1.5 rounded-lg bg-gray-100 border border-gray-200 text-xs sm:text-sm">
            <User className="w-3.5 h-3.5 sm:w-4 sm:h-4 text-gray-500" />
            <span className="text-gray-700">
              {t('usage', { current: currentMemberCount, max: maxMembers === 4294967295 ? '∞' : maxMembers })}
            </span>
          </div>
          <Button
            size="sm"
            onClick={() => setShowCreate(true)}
            disabled={!workspaceId || isAtLimit}
            className="h-7 text-xs px-2.5 bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white"
          >
            <Plus className="w-3.5 h-3.5 sm:mr-1" />
            <span className="hidden sm:inline">{t('addMember')}</span>
          </Button>
        </div>
      </div>

      {/* Upgrade Banner (when at limit and not on enterprise) */}
      {isAtLimit && currentWorkspace?.plan !== 'enterprise' && (
        <div className="mb-6 sm:mb-8 flex flex-wrap items-center gap-2 sm:gap-3 text-sm text-gray-500">
          <div className="flex items-center gap-2">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-amber-400" />
            <span>{t('upgrade.limitMessage')}</span>
          </div>
          <Link
            href="/dashboard/billing"
            className="text-violet-600 hover:text-violet-700 font-medium hover:underline"
          >
            {t('upgrade.cta')} →
          </Link>
        </div>
      )}

      {/* Create Form */}
      {showCreate && workspaceId && (
        <AddMemberForm
          workspaceId={workspaceId}
          onClose={() => setShowCreate(false)}
          t={t}
          tCommon={tCommon}
        />
      )}

      {/* Members List */}
      <div>
        {isLoading ? (
          <div className="space-y-3">
            {[...Array(3)].map((_, i) => (
              <div key={i} className="p-4 rounded-xl bg-white border border-gray-100 animate-pulse">
                <div className="flex flex-col sm:flex-row sm:items-center gap-3 sm:gap-4">
                  <div className="flex items-center gap-3 sm:gap-4">
                    <div className="w-10 h-10 rounded-full bg-gray-200 flex-shrink-0" />
                    <div className="flex-1">
                      <div className="h-4 w-32 bg-gray-200 rounded mb-1.5" />
                      <div className="h-3 w-48 bg-gray-100 rounded" />
                    </div>
                  </div>
                  <div className="flex items-center gap-2 ml-[52px] sm:ml-0">
                    <div className="h-8 w-24 bg-gray-100 rounded-lg" />
                    <div className="h-7 w-12 bg-gray-100 rounded-lg" />
                  </div>
                </div>
              </div>
            ))}
          </div>
        ) : isErrorWorkspaces || isErrorMembers ? (
          <div className="py-16 text-center">
            <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-red-100 flex items-center justify-center">
              <AlertCircle className="w-8 h-8 text-red-400" />
            </div>
            <p className="text-gray-500 mb-4">{t('loadError')}</p>
            <button
              onClick={() => window.location.reload()}
              className="text-sm text-violet-600 hover:text-violet-700"
            >
              {tCommon('retry')}
            </button>
          </div>
        ) : members?.length === 1 ? (
          <div className="flex flex-col items-center justify-center py-20">
            <Users className="w-12 h-12 text-gray-300 mb-4" />
            <p className="text-gray-400">{t('empty')}</p>
          </div>
        ) : (
          <div className="space-y-3">
            {members?.map((member, index) => (
              <MemberRow
                key={member.user_id}
                member={member}
                workspaceId={workspaceId!}
                t={t}
                tCommon={tCommon}
                index={index}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function AddMemberForm({
  workspaceId,
  onClose,
  t,
  tCommon,
}: {
  workspaceId: string;
  onClose: () => void;
  t: (key: string) => string;
  tCommon: (key: string) => string;
}) {
  const queryClient = useQueryClient();
  const [email, setEmail] = useState('');
  const [role, setRole] = useState<WorkspaceRole>('member');
  const [error, setError] = useState<string | null>(null);

  const createMutation = useMutation({
    mutationFn: (data: AddMemberRequest) =>
      api.post<TeamMember>(`/workspaces/${workspaceId}/members`, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'members'] });
      onClose();
    },
    onError: (err: unknown) => {
      const errorCode = getApiErrorCode(err);
      if (errorCode === 'USER_NOT_REGISTERED') {
        setError(t('errors.userNotFound'));
      } else if (errorCode === 'ALREADY_MEMBER') {
        setError(t('errors.alreadyMember'));
      } else if (errorCode === 'MEMBER_LIMIT_REACHED') {
        setError(t('errors.limitReached'));
      } else {
        setError(getApiErrorMessage(err));
      }
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    createMutation.mutate({ email, role });
  };

  const roles: WorkspaceRole[] = ['admin', 'member', 'viewer'];

  const roleDescriptions: Partial<Record<WorkspaceRole, string>> = {
    admin: t('roleDesc.admin'),
    member: t('roleDesc.member'),
    viewer: t('roleDesc.viewer'),
  };

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center">
      <div className="absolute inset-0 bg-black/30" onClick={onClose} />
      <div className="relative w-full max-w-lg mx-0 sm:mx-4 bg-white rounded-t-2xl sm:rounded-2xl border border-gray-200 shadow-2xl max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-gray-200">
          <h2 className="text-base font-semibold text-gray-500">{t('add.title')}</h2>
          <button
            onClick={onClose}
            className="p-1.5 -mr-1.5 text-gray-400 hover:text-gray-500 hover:bg-gray-100 rounded-lg transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body */}
        <form onSubmit={handleSubmit}>
          <div className="px-5 py-4">
            {/* Email Input */}
            <div className="mb-5">
              <label className="block text-sm font-medium text-gray-700 mb-2">
                {t('add.email')}
              </label>
              <input
                type="email"
                placeholder="user@example.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
                autoFocus
                className="w-full px-3 py-[9px] text-sm bg-white border border-gray-300 rounded-lg shadow-sm placeholder:text-gray-400 focus:outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
              />
            </div>

            {/* Role Selection */}
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-2">
                {t('add.role')}
              </label>
              <div className="border border-gray-300 rounded-lg divide-y divide-gray-200 overflow-hidden">
                {roles.map((r) => (
                  <label
                    key={r}
                    className="flex items-start gap-3 px-3 py-3 cursor-pointer hover:bg-gray-50 transition-colors"
                  >
                    <input
                      type="radio"
                      name="role"
                      value={r}
                      checked={role === r}
                      onChange={() => setRole(r)}
                      className="mt-0.5 w-4 h-4 text-violet-600 border-gray-300 focus:ring-violet-500 focus:ring-offset-0"
                    />
                    <div className="flex-1 min-w-0">
                      <div className="text-sm font-medium text-gray-900">{t(`roles.${r}`)}</div>
                      <div className="text-xs text-gray-500 mt-0.5 leading-relaxed">{roleDescriptions[r]}</div>
                    </div>
                  </label>
                ))}
              </div>
            </div>

            {error && (
              <div className="mt-4 px-3 py-2.5 text-sm text-red-700 bg-red-50 border border-red-200 rounded-lg">
                {error}
              </div>
            )}
          </div>

          {/* Footer */}
          <div className="flex justify-end gap-2 px-5 py-4 bg-gray-50 border-t border-gray-200 rounded-b-2xl">
            <button
              type="button"
              onClick={onClose}
              className="inline-flex items-center justify-center h-7 px-2.5 text-xs font-medium text-gray-700 bg-gray-100 border border-gray-300 rounded-[10px] hover:bg-gray-200 active:bg-gray-300 transition-colors"
            >
              {tCommon('cancel')}
            </button>
            <button
              type="submit"
              disabled={createMutation.isPending || !email}
              className="inline-flex items-center justify-center h-7 px-2.5 text-xs font-medium text-white bg-violet-600 border border-violet-700 rounded-[10px] hover:bg-violet-700 active:bg-violet-800 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              {createMutation.isPending ? tCommon('loading') : t('add.submit')}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function MemberRow({
  member,
  workspaceId,
  t,
  tCommon,
  index,
}: {
  member: TeamMember;
  workspaceId: string;
  t: (key: string, values?: Record<string, string | number>) => string;
  tCommon: (key: string) => string;
  index: number;
}) {
  const queryClient = useQueryClient();
  const [isHovered, setIsHovered] = useState(false);
  const [selectedRole, setSelectedRole] = useState<WorkspaceRole>(member.role);

  const updateMutation = useMutation({
    mutationFn: (role: WorkspaceRole) =>
      api.patch(`/workspaces/${workspaceId}/members/${member.user_id}`, { role }),
    // Optimistic update for role change
    onMutate: async (newRole) => {
      await queryClient.cancelQueries({ queryKey: ['workspaces', workspaceId, 'members'] });
      const previousMembers = queryClient.getQueryData<TeamMember[]>(['workspaces', workspaceId, 'members']);

      queryClient.setQueryData<TeamMember[]>(['workspaces', workspaceId, 'members'], (old) =>
        old?.map((m) => m.user_id === member.user_id ? { ...m, role: newRole } : m)
      );

      return { previousMembers };
    },
    onError: (_err, _newRole, context) => {
      if (context?.previousMembers) {
        queryClient.setQueryData(['workspaces', workspaceId, 'members'], context.previousMembers);
        setSelectedRole(member.role); // Reset local state
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'members'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => api.delete(`/workspaces/${workspaceId}/members/${member.user_id}`),
    // Optimistic update for member removal
    onMutate: async () => {
      await queryClient.cancelQueries({ queryKey: ['workspaces', workspaceId, 'members'] });
      const previousMembers = queryClient.getQueryData<TeamMember[]>(['workspaces', workspaceId, 'members']);

      queryClient.setQueryData<TeamMember[]>(['workspaces', workspaceId, 'members'], (old) =>
        old?.filter((m) => m.user_id !== member.user_id)
      );

      return { previousMembers };
    },
    onError: (_err, _vars, context) => {
      if (context?.previousMembers) {
        queryClient.setQueryData(['workspaces', workspaceId, 'members'], context.previousMembers);
      }
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ['workspaces', workspaceId, 'members'] });
    },
  });

  const handleRoleChange = (newRole: WorkspaceRole) => {
    setSelectedRole(newRole);
    updateMutation.mutate(newRole);
  };

  const colors = [
    'from-blue-400 to-cyan-500',
    'from-violet-400 to-purple-500',
    'from-emerald-400 to-teal-500',
    'from-amber-400 to-orange-500',
    'from-pink-400 to-rose-500',
  ];

  const isOwner = member.role === 'owner';

  return (
    <div
      className="group p-4 rounded-xl bg-white border border-gray-100 hover:border-gray-200 hover:shadow-md transition-all"
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
    >
      <div className="flex flex-col sm:flex-row sm:items-center gap-3 sm:gap-4">
        <div className="flex items-center gap-3 sm:gap-4">
          {member.avatar_url ? (
            <img
              src={member.avatar_url}
              alt={member.name}
              className="w-10 h-10 rounded-full flex-shrink-0"
            />
          ) : (
            <div className={`w-10 h-10 rounded-full bg-gradient-to-br ${colors[index % colors.length]} flex items-center justify-center flex-shrink-0`}>
              <span className="text-white font-bold text-sm">{member.name.charAt(0).toUpperCase()}</span>
            </div>
          )}

          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="font-medium text-gray-900 text-sm sm:text-base">{member.name}</span>
              {isOwner && (
                <span className="px-2 py-0.5 text-xs bg-amber-100 text-amber-700 rounded-full">
                  {t('roles.owner')}
                </span>
              )}
            </div>
            <p className="text-xs sm:text-sm text-gray-500 truncate">{member.email}</p>
          </div>
        </div>

        {!isOwner && (
          <div className="flex items-center gap-2 sm:gap-3 ml-[52px] sm:ml-0">
            <select
              value={selectedRole}
              onChange={(e) => handleRoleChange(e.target.value as WorkspaceRole)}
              disabled={updateMutation.isPending}
              className="px-2 sm:px-3 py-1.5 text-xs sm:text-sm border border-gray-200 rounded-lg bg-white focus:outline-none focus:ring-2 focus:ring-violet-500"
            >
              <option value="admin">{t('roles.admin')}</option>
              <option value="member">{t('roles.member')}</option>
              <option value="viewer">{t('roles.viewer')}</option>
            </select>

            <AlertDialog>
              <AlertDialogTrigger asChild>
                <button
                  className={`px-2 sm:px-3 py-1.5 text-xs sm:text-sm text-red-600 hover:bg-red-50 rounded-lg transition-all sm:opacity-0 sm:group-hover:opacity-100 ${
                    isHovered ? 'sm:opacity-100' : ''
                  }`}
                >
                  {tCommon('delete')}
                </button>
              </AlertDialogTrigger>
              <AlertDialogContent className="max-w-[calc(100%-2rem)] sm:max-w-md mx-4 sm:mx-auto">
                <AlertDialogHeader>
                  <AlertDialogTitle>{tCommon('confirm')}</AlertDialogTitle>
                  <AlertDialogDescription>
                    {t('remove.confirm', { name: member.name })}
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>{tCommon('cancel')}</AlertDialogCancel>
                  <AlertDialogAction
                    onClick={() => deleteMutation.mutate()}
                    className="bg-red-600 hover:bg-red-700"
                  >
                    {tCommon('delete')}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
        )}
      </div>
    </div>
  );
}

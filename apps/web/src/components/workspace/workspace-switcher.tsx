'use client';

import { useEffect, useRef, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import { Check, ChevronsUpDown, Plus, X } from 'lucide-react';
import { api } from '@/lib/api';
import { Workspace, getApiErrorCode, getApiErrorMessage } from '@/types';
import { useWorkspace } from '@/hooks/use-workspace';
import Avatar from 'boring-avatars';

const PLAN_BADGE_CLASSES: Record<string, string> = {
  free: 'bg-gray-100 text-gray-600',
  pro: 'bg-violet-100 text-violet-700',
  team: 'bg-blue-100 text-blue-700',
  enterprise: 'bg-amber-100 text-amber-700',
};

function PlanBadge({ plan, t }: { plan: string; t: (key: string) => string }) {
  const cls = PLAN_BADGE_CLASSES[plan] ?? PLAN_BADGE_CLASSES.free;
  return (
    <span className={`px-1.5 py-0.5 text-[10px] font-medium rounded ${cls}`}>
      {t(`plan.${plan}`)}
    </span>
  );
}

// Round, Boring Avatars identicon per workspace (generated from the id).
function WorkspaceAvatar({
  name,
  seed,
  size = 'md',
  active = false,
}: {
  name: string;
  seed: string;
  size?: 'md' | 'lg';
  active?: boolean;
}) {
  const px = size === 'lg' ? 32 : 24;
  return (
    <span
      className={`relative inline-flex shrink-0 overflow-hidden rounded-full shadow-sm ring-1 ring-black/5 ${
        active ? 'ring-2 ring-offset-1 ring-violet-400/70' : ''
      }`}
      style={{ width: px, height: px }}
    >
      <Avatar size={px} name={seed || name} variant="beam" />
    </span>
  );
}

// Derive a url-safe slug from a free-text name.
function slugify(value: string): string {
  return value
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 48);
}

export function WorkspaceSwitcher({ collapsed = false }: { collapsed?: boolean }) {
  const t = useTranslations('workspace');
  const { workspaces, activeWorkspace, activeWorkspaceId, setActiveWorkspaceId } = useWorkspace();
  const [open, setOpen] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Close the dropdown on outside click / Escape.
  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onClick);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onClick);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  if (!activeWorkspace) {
    return (
      <div className={`h-9 rounded-lg bg-gray-100 animate-pulse ${collapsed ? 'w-9 mx-auto' : 'w-full'}`} />
    );
  }

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={collapsed ? activeWorkspace.name : undefined}
        aria-label={t('label')}
        className={`group flex items-center transition-colors ${
          collapsed
            ? 'h-9 w-9 justify-center rounded-full p-0 mx-auto hover:bg-gray-100'
            : 'w-full gap-2 rounded-xl border border-gray-200 px-2 py-1.5 hover:bg-gray-50 hover:border-gray-300'
        }`}
      >
        <WorkspaceAvatar
          name={activeWorkspace.name}
          seed={activeWorkspace.id}
          size={collapsed ? 'lg' : 'md'}
          active
        />
        {!collapsed && (
          <>
            <span className="flex-1 min-w-0 text-left text-sm font-medium text-gray-800 truncate">
              {activeWorkspace.name}
            </span>
            <PlanBadge plan={activeWorkspace.plan} t={t} />
            <ChevronsUpDown className="w-3.5 h-3.5 text-gray-400 shrink-0" />
          </>
        )}
      </button>

      {open && (
        <div
          className={`absolute z-50 mt-1 w-60 rounded-lg border border-gray-200 bg-white shadow-lg py-1 ${
            collapsed ? 'left-0' : 'left-0 right-0'
          }`}
        >
          <div className="px-3 py-1.5 text-[11px] font-medium uppercase tracking-wide text-gray-400">
            {t('yourWorkspaces')}
          </div>
          <div className="max-h-64 overflow-y-auto">
            {workspaces?.map((ws) => (
              <button
                key={ws.id}
                type="button"
                onClick={() => {
                  if (ws.id !== activeWorkspaceId) setActiveWorkspaceId(ws.id);
                  setOpen(false);
                }}
                className="flex w-full items-center gap-2.5 px-3 py-2 text-left hover:bg-gray-50 transition-colors"
              >
                <WorkspaceAvatar name={ws.name} seed={ws.id} active={ws.id === activeWorkspaceId} />
                <span className="flex-1 min-w-0">
                  <span className="flex items-center gap-1.5">
                    <span className="text-sm font-medium text-gray-800 truncate">{ws.name}</span>
                    <PlanBadge plan={ws.plan} t={t} />
                  </span>
                  <span className="block text-[11px] text-gray-400 capitalize">{t(`roles.${ws.role}`)}</span>
                </span>
                {ws.id === activeWorkspaceId && (
                  <Check className="w-4 h-4 text-violet-600 shrink-0" />
                )}
              </button>
            ))}
          </div>
          <div className="my-1 h-px bg-gray-100" />
          <button
            type="button"
            onClick={() => {
              setOpen(false);
              setShowCreate(true);
            }}
            className="group flex w-full items-center gap-2.5 px-3 py-2 text-left text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors"
          >
            <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-dashed border-gray-300 text-gray-500 group-hover:border-violet-400 group-hover:text-violet-500 transition-colors">
              <Plus className="w-3.5 h-3.5" />
            </span>
            {t('createTeam')}
          </button>
        </div>
      )}

      {showCreate && (
        <CreateTeamDialog
          onClose={() => setShowCreate(false)}
          onCreated={(ws) => {
            setActiveWorkspaceId(ws.id);
            setShowCreate(false);
          }}
        />
      )}
    </div>
  );
}

function CreateTeamDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (ws: Workspace) => void;
}) {
  const t = useTranslations('workspace');
  const tCommon = useTranslations('common');
  const queryClient = useQueryClient();
  const [name, setName] = useState('');
  const [slug, setSlug] = useState('');
  const [slugEdited, setSlugEdited] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const effectiveSlug = slugEdited ? slug : slugify(name);

  const createMutation = useMutation({
    mutationFn: (data: { name: string; slug: string }) => api.post<Workspace>('/workspaces', data),
    onSuccess: (ws) => {
      queryClient.invalidateQueries({ queryKey: ['workspaces'] });
      onCreated(ws);
    },
    onError: (err: unknown) => {
      const code = getApiErrorCode(err);
      // Backend returns 409 (ApiError.status) when the slug is taken.
      if (code === 'SLUG_TAKEN' || code === 'CONFLICT' || (err as { status?: number })?.status === 409) {
        setError(t('slugTaken'));
      } else {
        setError(getApiErrorMessage(err));
      }
    },
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    createMutation.mutate({ name: name.trim(), slug: effectiveSlug });
  };

  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center">
      <div className="absolute inset-0 bg-black/30" onClick={onClose} />
      <div className="relative w-full max-w-md mx-0 sm:mx-4 bg-white rounded-t-2xl sm:rounded-2xl border border-gray-200 shadow-2xl">
        <div className="flex items-center justify-between px-5 py-4 border-b border-gray-200">
          <h2 className="text-base font-semibold text-gray-700">{t('createTeamTitle')}</h2>
          <button
            onClick={onClose}
            className="p-1.5 -mr-1.5 text-gray-400 hover:text-gray-500 hover:bg-gray-100 rounded-lg transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <form onSubmit={handleSubmit}>
          <div className="px-5 py-4 space-y-4">
            <p className="text-sm text-gray-500">{t('createTeamDescription')}</p>

            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1.5">{t('nameLabel')}</label>
              <input
                type="text"
                autoFocus
                required
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t('namePlaceholder')}
                className="w-full px-3 py-[9px] text-sm bg-white border border-gray-300 rounded-lg shadow-sm placeholder:text-gray-400 focus:outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
              />
            </div>

            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1.5">{t('slugLabel')}</label>
              <input
                type="text"
                required
                value={effectiveSlug}
                onChange={(e) => {
                  setSlugEdited(true);
                  setSlug(slugify(e.target.value));
                }}
                placeholder={t('slugPlaceholder')}
                className="w-full px-3 py-[9px] text-sm bg-white border border-gray-300 rounded-lg shadow-sm placeholder:text-gray-400 focus:outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
              />
            </div>

            {error && (
              <div className="px-3 py-2.5 text-sm text-red-700 bg-red-50 border border-red-200 rounded-lg">
                {error}
              </div>
            )}
          </div>

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
              disabled={createMutation.isPending || !name.trim() || !effectiveSlug}
              className="inline-flex items-center justify-center h-7 px-2.5 text-xs font-medium text-white bg-violet-600 border border-violet-700 rounded-[10px] hover:bg-violet-700 active:bg-violet-800 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              {createMutation.isPending ? t('creating') : t('create')}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

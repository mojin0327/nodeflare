'use client';

import { useAuth } from '@/hooks/use-auth';
import { useRouter, usePathname } from 'next/navigation';
import { useEffect, useState, useMemo, useCallback, useTransition } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import { api } from '@/lib/api';
import { McpServerMinimal } from '@/types';
import Link from 'next/link';
import Image from 'next/image';
import { Button } from '@/components/ui/button';
import { LayoutDashboard, Server, Lock, Shield, Users, FileText, CreditCard, Settings, ChevronsLeft, ChevronsRight, X, LogOut, Menu } from 'lucide-react';
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  DragEndEvent,
} from '@dnd-kit/core';
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import { PageHeaderProvider, DashboardHeaderTitle } from './page-header';
import { WorkspaceProvider, useWorkspace } from '@/hooks/use-workspace';
import { WorkspaceSwitcher } from '@/components/workspace/workspace-switcher';

interface NavItem {
  id: string;
  href: string;
  icon: React.ReactNode;
  exact?: boolean;
  prefetchKeys?: string[][];
}

// プリフェッチ対象のクエリキー定義（データ量が小さいページのみ）
const PREFETCH_QUERIES: Record<string, string[][]> = {
  settings: [['notificationSettings']],
  auth: [['workspaces']],
  team: [['workspaces'], ['billing-plans']],
  vpn: [['workspaces']],
};

const DEFAULT_SIDEBAR_ORDER = ['overview', 'servers', 'auth', 'vpn', 'team', 'logs', 'billing', 'settings'];

export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <PageHeaderProvider>
      <WorkspaceProvider>
        <DashboardShell>{children}</DashboardShell>
      </WorkspaceProvider>
    </PageHeaderProvider>
  );
}

function DashboardShell({
  children,
}: {
  children: React.ReactNode;
}) {
  const t = useTranslations('nav');
  const { user, isLoading, logout } = useAuth();
  const { activeWorkspace } = useWorkspace();
  const router = useRouter();
  const pathname = usePathname();
  const queryClient = useQueryClient();
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const [, startTransition] = useTransition();
  const [pendingHref, setPendingHref] = useState<string | null>(null);

  // ナビゲーション完了時にpendingHrefをクリア
  useEffect(() => {
    setPendingHref(null);
  }, [pathname]);

  const { data: servers, isLoading: serversLoading } = useQuery<McpServerMinimal[]>({
    queryKey: ['servers-minimal', activeWorkspace?.id],
    queryFn: () => api.get(`/workspaces/${activeWorkspace!.id}/servers/minimal`),
    enabled: !!user && !!activeWorkspace,
  });

  const { data: preferences } = useQuery<{ sidebar_order: string[] }>({
    queryKey: ['userPreferences'],
    queryFn: () => api.get('/user/preferences'),
    enabled: !!user,
  });

  const updatePreferencesMutation = useMutation({
    mutationFn: (sidebarOrder: string[]) =>
      api.patch('/user/preferences', { sidebar_order: sidebarOrder }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['userPreferences'] });
    },
  });

  const [sidebarOrder, setSidebarOrder] = useState<string[]>(DEFAULT_SIDEBAR_ORDER);

  useEffect(() => {
    if (preferences?.sidebar_order) {
      // 保存された設定に含まれていない新しいナビアイテムを追加
      const savedOrder = preferences.sidebar_order;
      const newItems = DEFAULT_SIDEBAR_ORDER.filter(id => !savedOrder.includes(id));
      if (newItems.length > 0) {
        setSidebarOrder([...savedOrder, ...newItems]);
      } else {
        setSidebarOrder(savedOrder);
      }
    }
  }, [preferences]);

  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: {
        distance: 8,
      },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    })
  );

  // クリック時に即座にUIを更新してから遷移
  const handleNavigation = useCallback((href: string) => {
    if (href === pathname) return;
    setPendingHref(href);
    setMobileMenuOpen(false); // Close mobile menu on navigation
    startTransition(() => {
      router.push(href);
    });
  }, [router, pathname]);

  // ホバー時にプリフェッチ（推奨ページのみ：Settings, API-Keys, Team, VPN）
  const handlePrefetch = useCallback((navId: string) => {
    const queries = PREFETCH_QUERIES[navId];
    if (!queries) return;

    queries.forEach((queryKey) => {
      // キャッシュにデータがない場合のみプリフェッチ
      const cached = queryClient.getQueryData(queryKey);
      if (!cached) {
        if (queryKey[0] === 'notificationSettings') {
          queryClient.prefetchQuery({
            queryKey,
            queryFn: () => api.get('/user/notifications'),
          });
        } else if (queryKey[0] === 'workspaces') {
          queryClient.prefetchQuery({
            queryKey,
            queryFn: () => api.get('/workspaces'),
          });
        } else if (queryKey[0] === 'billing-plans') {
          queryClient.prefetchQuery({
            queryKey,
            queryFn: () => api.get('/billing/plans'),
          });
        }
      }
    });
  }, [queryClient]);

  // Memoize navItemsMap to prevent recreation on every render
  const navItemsMap = useMemo<Record<string, NavItem>>(() => ({
    overview: {
      id: 'overview',
      href: '/dashboard',
      exact: true,
      icon: <LayoutDashboard className="w-4 h-4" />,
    },
    servers: {
      id: 'servers',
      href: '/dashboard/servers',
      icon: <Server className="w-4 h-4" />,
    },
    auth: {
      id: 'auth',
      href: '/dashboard/auth',
      icon: <Lock className="w-4 h-4" />,
    },
    vpn: {
      id: 'vpn',
      href: '/dashboard/vpn',
      icon: <Shield className="w-4 h-4" />,
    },
    team: {
      id: 'team',
      href: '/dashboard/team',
      icon: <Users className="w-4 h-4" />,
    },
    logs: {
      id: 'logs',
      href: '/dashboard/logs',
      icon: <FileText className="w-4 h-4" />,
    },
    billing: {
      id: 'billing',
      href: '/dashboard/billing',
      icon: <CreditCard className="w-4 h-4" />,
    },
    settings: {
      id: 'settings',
      href: '/dashboard/settings',
      icon: <Settings className="w-4 h-4" />,
    },
  }), []);

  const sortedNavItems = useMemo(() => {
    return sidebarOrder
      .filter(id => navItemsMap[id])
      .map(id => navItemsMap[id]);
  }, [sidebarOrder, navItemsMap]);

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;

    if (over && active.id !== over.id) {
      const oldIndex = sidebarOrder.indexOf(active.id as string);
      const newIndex = sidebarOrder.indexOf(over.id as string);
      const newOrder = arrayMove(sidebarOrder, oldIndex, newIndex);
      setSidebarOrder(newOrder);
      updatePreferencesMutation.mutate(newOrder);
    }
  };

  useEffect(() => {
    if (!isLoading && !user) {
      router.push('/login');
    }
  }, [user, isLoading, router]);


  if (isLoading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="w-8 h-8 border-4 rounded-full border-gray-200 border-t-violet-600 animate-spin" />
      </div>
    );
  }

  if (!user) {
    return null;
  }

  return (
    <div className="h-screen flex overflow-hidden">
      {/* Mobile menu backdrop */}
      {mobileMenuOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-40 md:hidden"
          onClick={() => setMobileMenuOpen(false)}
        />
      )}

      {/* Sidebar - Desktop */}
      <aside className={`hidden md:flex ${sidebarOpen ? 'w-56' : 'w-12'} bg-card transition-all duration-300 flex-shrink-0 flex-col`}>
        <div className="h-14 px-2 border-b border-gray-200 flex items-center justify-between relative">
          <div className="absolute right-0 top-4 bottom-4 w-px bg-gray-200" />
          {sidebarOpen && (
            <Link href="/dashboard" className="flex items-center gap-2 min-w-0">
              <Image src="/logo2.png" alt="Nodeflare" width={153} height={32} className="h-8 w-auto shrink-0" />
            </Link>
          )}
          <button
            onClick={() => setSidebarOpen(!sidebarOpen)}
            className="p-1.5 rounded-md hover:bg-accent transition-colors shrink-0"
          >
            {sidebarOpen ? (
              <ChevronsLeft className="w-4 h-4" />
            ) : (
              <ChevronsRight className="w-4 h-4" />
            )}
          </button>
        </div>
        <div className={`border-r border-gray-200 ${sidebarOpen ? 'px-2 py-2' : 'px-1.5 py-2'}`}>
          <WorkspaceSwitcher collapsed={!sidebarOpen} />
        </div>
        <nav className="py-2 px-2 space-y-0.5 border-r border-gray-200 flex-1">
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragEnd={handleDragEnd}
          >
            <SortableContext items={sidebarOrder} strategy={verticalListSortingStrategy}>
              {sortedNavItems.map((item) => (
                <SortableNavLink
                  key={item.id}
                  id={item.id}
                  href={item.href}
                  pathname={pathname}
                  pendingHref={pendingHref}
                  exact={item.exact}
                  icon={item.icon}
                  collapsed={!sidebarOpen}
                  onPrefetch={handlePrefetch}
                  onNavigate={handleNavigation}
                >
                  {t(item.id)}
                </SortableNavLink>
              ))}
            </SortableContext>
          </DndContext>
        </nav>
      </aside>

      {/* Sidebar - Mobile (drawer) */}
      <aside className={`fixed inset-y-0 left-0 z-50 w-64 bg-card transform transition-transform duration-300 ease-in-out md:hidden ${mobileMenuOpen ? 'translate-x-0' : '-translate-x-full'}`}>
        <div className="h-14 px-4 border-b border-gray-200 flex items-center justify-between">
          <Link href="/dashboard" className="flex items-center gap-2" onClick={() => setMobileMenuOpen(false)}>
            <Image src="/logo2.png" alt="Nodeflare" width={96} height={20} className="h-5 w-auto" />
          </Link>
          <button
            onClick={() => setMobileMenuOpen(false)}
            className="p-1.5 rounded-md hover:bg-accent transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>
        <div className="px-2 py-2">
          <WorkspaceSwitcher />
        </div>
        <nav className="py-2 px-2 space-y-0.5 flex-1 overflow-y-auto">
          {sortedNavItems.map((item) => (
            <a
              key={item.id}
              href={item.href}
              className={`flex items-center gap-3 px-3 py-2.5 rounded-md text-sm font-medium transition-colors ${
                (item.exact ? pathname === item.href : pathname === item.href || pathname.startsWith(item.href + '/'))
                  ? 'bg-gray-100 text-foreground'
                  : 'hover:bg-gray-50 text-gray-500 hover:text-foreground'
              }`}
              onClick={(e) => {
                e.preventDefault();
                handleNavigation(item.href);
              }}
            >
              <span className="flex-shrink-0">{item.icon}</span>
              <span>{t(item.id)}</span>
            </a>
          ))}
        </nav>
        {/* Mobile user info */}
        <div className="border-t border-gray-200 p-4">
          <div className="flex items-center gap-3 mb-3">
            {user.avatar_url && (
              <Image
                src={user.avatar_url}
                alt={user.name}
                width={40}
                height={40}
                className="w-10 h-10 rounded-full"
              />
            )}
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium truncate">{user.name}</p>
              <p className="text-xs text-muted-foreground truncate">{user.email}</p>
            </div>
          </div>
          <Button variant="outline" size="sm" className="w-full" onClick={() => { setMobileMenuOpen(false); logout(); }}>
            <LogOut className="w-4 h-4 mr-2" />
            {t('logout')}
          </Button>
        </div>
      </aside>

      {/* Main content */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Top bar */}
        <header className="h-14 border-b border-gray-200 flex items-center justify-between px-4 md:px-6 bg-card">
          {/* Left: mobile menu button + current page title/icon */}
          <div className="flex items-center gap-2 sm:gap-3 min-w-0">
            <button
              onClick={() => setMobileMenuOpen(true)}
              className="p-2 -ml-2 rounded-md hover:bg-accent transition-colors md:hidden"
            >
              <Menu className="w-5 h-5" />
            </button>
            <DashboardHeaderTitle />
          </div>
          <div className="flex items-center space-x-2 md:space-x-4">
            <span className="text-sm text-muted-foreground hidden sm:block">{user.name}</span>
            {user.avatar_url && (
              <Image
                src={user.avatar_url}
                alt={user.name}
                width={32}
                height={32}
                className="w-8 h-8 rounded-full"
              />
            )}
            <Button variant="ghost" size="icon" onClick={() => logout()} title={t('logout')} className="hidden md:flex">
              <LogOut className="w-4 h-4" />
            </Button>
          </div>
        </header>

        {/* Page content */}
        <main className="flex-1 p-4 md:p-6 bg-card overflow-y-auto overflow-x-hidden">
          {children}
        </main>
      </div>
    </div>
  );
}

function SortableNavLink({
  id,
  href,
  icon,
  children,
  collapsed,
  pathname,
  pendingHref,
  exact = false,
  onPrefetch,
  onNavigate
}: {
  id: string;
  href: string;
  icon: React.ReactNode;
  children: React.ReactNode;
  collapsed: boolean;
  pathname: string;
  pendingHref: string | null;
  exact?: boolean;
  onPrefetch?: (id: string) => void;
  onNavigate?: (href: string) => void;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  // 楽観的更新: pending中は遷移先のみアクティブ、他は全て非アクティブ
  const isActiveByPath = exact
    ? pathname === href
    : pathname === href || pathname.startsWith(href + '/');
  const isPendingThis = pendingHref === href;
  const isActive = pendingHref ? isPendingThis : isActiveByPath;

  return (
    <div ref={setNodeRef} style={style} {...attributes} {...listeners}>
      <a
        href={href}
        className={`flex items-center gap-4 px-2.5 py-1.5 rounded-lg text-sm font-medium transition-colors ${
          collapsed ? 'justify-center' : ''
        } ${
          isActive
            ? 'bg-gray-200 text-foreground'
            : 'hover:bg-gray-50 text-gray-500 hover:text-foreground'
        } ${isDragging ? 'cursor-grabbing' : 'cursor-grab'}`}
        title={collapsed ? String(children) : undefined}
        onClick={(e) => {
          e.preventDefault();
          if (isDragging) return;
          onNavigate?.(href);
        }}
        onMouseEnter={() => onPrefetch?.(id)}
      >
        <span className="flex-shrink-0">{icon}</span>
        {!collapsed && <span>{children}</span>}
      </a>
    </div>
  );
}

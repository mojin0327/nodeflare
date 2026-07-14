'use client';

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslations } from 'next-intl';
import { AlertCircle, CreditCard, X, ArrowLeft, ChevronLeft, ChevronRight, Clock, Check } from 'lucide-react';
import { api } from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from '@/components/ui/card';
import { UsageCard } from '@/components/billing/usage-card';
import { useWorkspace } from '@/hooks/use-workspace';
import { formatDate, formatYearMonth, formatShortDate } from '@/lib/date-utils';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog';

interface Plan {
  plan: string;
  name: string;
  description: string;
  price_monthly_jpy: number | null;
  price_yearly_jpy: number | null;
  features: string[];
  limits: {
    max_servers: number;
    max_deployments_per_month: number;
    max_requests_per_month: number;
    max_team_members: number;
    log_retention_days: number;
    custom_domains: boolean;
    priority_support: boolean;
    sso_enabled: boolean;
  };
}

interface Subscription {
  plan: string;
  status: string;
  stripe_customer_id: string | null;
  stripe_subscription_id: string | null;
  current_period_start: number | null;
  current_period_end: number | null;
  cancel_at_period_end: boolean;
}

interface PaymentMethod {
  brand: string;
  last4: string;
  exp_month: number;
  exp_year: number;
}

interface BillingSettings {
  auto_email_invoices: boolean;
}

interface Invoice {
  id: string;
  number: string | null;
  status: string | null;
  amount_due: number;
  amount_paid: number;
  currency: string;
  created: number;
  hosted_invoice_url: string | null;
  invoice_pdf: string | null;
}

interface SubscriptionHistory {
  id: string;
  plan: string;
  status: string;
  current_period_start: number;
  current_period_end: number;
  canceled_at: number | null;
  ended_at: number | null;
  cancel_at_period_end: boolean;
}

export default function BillingPage() {
  const t = useTranslations('billing');
  const tCommon = useTranslations('common');
  const tApiErrors = useTranslations('apiErrors');
  const [selectedInterval, setSelectedInterval] = useState<'monthly' | 'yearly'>('monthly');
  const [showPlans, setShowPlans] = useState(false);
  const [selectedInvoice, setSelectedInvoice] = useState<Invoice | null>(null);
  const [checkoutPlan, setCheckoutPlan] = useState<string | null>(null);
  const [calendarDate, setCalendarDate] = useState(new Date());
  const [exportFrom, setExportFrom] = useState('');
  const [exportTo, setExportTo] = useState('');
  const [billingError, setBillingError] = useState<string | null>(null);
  const queryClient = useQueryClient();

  const { activeWorkspace: currentWorkspace, isLoading: workspacesLoading } = useWorkspace();
  const workspacesError = false;

  const { data: plans, isLoading: plansLoading, isError: plansError } = useQuery<Plan[]>({
    queryKey: ['billing-plans'],
    queryFn: () => api.get('/billing/plans'),
  });

  const { data: subscription, isLoading: subscriptionLoading, isError: subscriptionError } = useQuery<Subscription>({
    queryKey: ['subscription', currentWorkspace?.id],
    queryFn: () => api.get(`/workspaces/${currentWorkspace?.id}/billing/subscription`),
    enabled: !!currentWorkspace?.id,
  });

  const { data: invoices = [], isLoading: invoicesLoading, isError: invoicesError } = useQuery<Invoice[]>({
    queryKey: ['invoices', currentWorkspace?.id],
    queryFn: () => api.get(`/workspaces/${currentWorkspace?.id}/billing/invoices`),
    enabled: !!currentWorkspace?.id,
  });

  const { data: subscriptionHistory = [] } = useQuery<SubscriptionHistory[]>({
    queryKey: ['subscription-history', currentWorkspace?.id],
    queryFn: () => api.get(`/workspaces/${currentWorkspace?.id}/billing/subscriptions`),
    enabled: !!currentWorkspace?.id,
  });

  const { data: paymentMethodData, isLoading: paymentMethodLoading } = useQuery<{ payment_method: PaymentMethod | null }>({
    queryKey: ['payment-method', currentWorkspace?.id],
    queryFn: () => api.get(`/workspaces/${currentWorkspace?.id}/billing/payment-method`),
    enabled: !!currentWorkspace?.id && !!subscription?.stripe_customer_id,
  });

  const { data: billingSettings } = useQuery<BillingSettings>({
    queryKey: ['billing-settings', currentWorkspace?.id],
    queryFn: () => api.get(`/workspaces/${currentWorkspace?.id}/billing/settings`),
    enabled: !!currentWorkspace?.id,
  });

  const updateBillingSettingsMutation = useMutation({
    mutationFn: async (autoEmailInvoices: boolean) => {
      return api.patch(`/workspaces/${currentWorkspace?.id}/billing/settings`, {
        auto_email_invoices: autoEmailInvoices,
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['billing-settings', currentWorkspace?.id] });
    },
  });

  const handleBillingError = (error: any) => {
    const errorCode = error?.code;
    if (errorCode) {
      try {
        const translated = tApiErrors(errorCode);
        if (translated && translated !== errorCode) {
          setBillingError(translated);
          return;
        }
      } catch {
        // Translation not found
      }
    }
    setBillingError(error?.message || tCommon('error'));
  };

  const checkoutMutation = useMutation({
    mutationFn: async ({ plan, yearly }: { plan: string; yearly: boolean }) => {
      // If already has subscription, change plan instead of new checkout
      if (subscription?.stripe_subscription_id) {
        const response = await api.post<{ plan: string; status: string; portal_url?: string }>(`/workspaces/${currentWorkspace?.id}/billing/change-plan`, {
          plan,
          yearly,
        });
        return { type: 'change' as const, ...response };
      } else {
        const response = await api.post<{ checkout_url: string }>(`/workspaces/${currentWorkspace?.id}/billing/checkout`, {
          plan,
          yearly,
        });
        return { type: 'checkout' as const, ...response };
      }
    },
    onSuccess: (data) => {
      setBillingError(null);
      if (data.type === 'checkout') {
        window.location.href = data.checkout_url;
      } else if (data.portal_url) {
        // Redirect to Stripe Portal for plan change
        window.location.href = data.portal_url;
      } else {
        // Plan reactivated successfully, refresh data
        queryClient.invalidateQueries({ queryKey: ['subscription', currentWorkspace?.id] });
        queryClient.invalidateQueries({ queryKey: ['workspaces'] });
      }
    },
    onError: handleBillingError,
    onSettled: () => {
      setCheckoutPlan(null);
    },
  });

  const portalMutation = useMutation({
    mutationFn: async () => {
      const response = await api.post<{ portal_url: string }>(`/workspaces/${currentWorkspace?.id}/billing/portal`);
      return response;
    },
    onSuccess: (data) => {
      setBillingError(null);
      window.open(data.portal_url, '_blank');
    },
    onError: handleBillingError,
  });

  const cancelMutation = useMutation({
    mutationFn: async () => {
      const response = await api.post<{ status: string; cancel_at_period_end: boolean; current_period_end: number | null }>(
        `/workspaces/${currentWorkspace?.id}/billing/cancel`
      );
      return response;
    },
    onSuccess: () => {
      setBillingError(null);
      queryClient.invalidateQueries({ queryKey: ['subscription', currentWorkspace?.id] });
      queryClient.invalidateQueries({ queryKey: ['workspaces'] });
    },
    onError: handleBillingError,
  });

  const isLoading = workspacesLoading || plansLoading || subscriptionLoading || invoicesLoading;
  const hasError = workspacesError || plansError || subscriptionError || invoicesError;

  if (isLoading) {
    return (
      <div className="flex items-center justify-center min-h-[400px]">
        <div className="w-8 h-8 border-4 rounded-full border-gray-200 border-t-violet-600 animate-spin" />
      </div>
    );
  }

  if (hasError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-[400px]">
        <AlertCircle className="w-12 h-12 text-red-400 mb-4" />
        <p className="text-muted-foreground mb-4">{t('loadError')}</p>
        <button
          onClick={() => window.location.reload()}
          className="text-sm text-violet-600 hover:text-violet-700"
        >
          {tCommon('retry')}
        </button>
      </div>
    );
  }

  const currentPlan = subscription?.plan || 'free';
  const currentPlanData = plans?.find(p => p.plan === currentPlan);
  const isActive = subscription?.status === 'active';
  const isFree = currentPlan === 'free';

  // Calculate billing amounts
  const planPrice = currentPlanData?.price_monthly_jpy || 0;
  const totalMonthly = planPrice;
  const paymentMethod = paymentMethodData?.payment_method;
  const autoEmailEnabled = billingSettings?.auto_email_invoices ?? true;

  return (
    <div className="space-y-4 sm:space-y-6">
      {/* Billing Error */}
      {billingError && (
        <div className="p-4 rounded-xl bg-red-50 border border-red-200">
          <div className="flex items-center gap-3">
            <AlertCircle className="w-5 h-5 text-red-600 flex-shrink-0" />
            <p className="text-sm text-red-700">{billingError}</p>
            <button onClick={() => setBillingError(null)} className="ml-auto text-red-400 hover:text-red-600">
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>
      )}

      {/* Current Plan Header */}
      <div className="flex flex-col sm:flex-row sm:items-center gap-3 sm:gap-6 text-sm">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-muted-foreground">{t('currentPlan')}</span>
          <span className="font-semibold capitalize">{t(`plans.${currentPlan}.name` as any)}</span>
          <span className={`px-2 py-0.5 text-xs rounded ${isActive ? 'bg-green-100 text-green-700' : 'bg-gray-100 text-gray-600'}`}>
            {subscription?.status || 'free'}
          </span>
          {subscription?.cancel_at_period_end && (
            <span className="px-2 py-0.5 text-xs rounded bg-amber-100 text-amber-700">
              {t('scheduledCancel')}
            </span>
          )}
        </div>
        {subscription?.stripe_subscription_id && (
          <div className="flex flex-wrap gap-2 sm:gap-2.5 sm:ml-auto">
            <Button
              variant="outline"
              onClick={() => portalMutation.mutate()}
              disabled={portalMutation.isPending}
              className="h-9 px-4 rounded-lg border-[#d1d5db] text-[#374151] text-sm font-medium hover:bg-[#f3f4f6] transition-colors duration-200"
            >
              {portalMutation.isPending ? tCommon('loading') : t('manageSubscription')}
            </Button>
            {subscription?.cancel_at_period_end ? (
              <Button
                variant="outline"
                onClick={() => {
                  // Reactivate by changing to the same plan (clears cancel_at_period_end)
                  checkoutMutation.mutate({ plan: currentPlan, yearly: false });
                }}
                disabled={checkoutMutation.isPending}
                className="h-9 px-4 rounded-lg border-[#d1d5db] text-[#374151] text-sm font-medium hover:bg-[#f3f4f6] transition-colors duration-200"
              >
                {checkoutMutation.isPending ? t('undoCancelling') : t('undoCancel')}
              </Button>
            ) : (
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button
                  disabled={cancelMutation.isPending}
                  className="h-9 px-4 rounded-lg bg-red-500 hover:bg-red-600 border border-red-600 text-white text-sm font-medium transition-colors duration-200"
                >
                  {cancelMutation.isPending ? t('cancelling') : t('cancelSubscription')}
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent className="max-w-[calc(100%-2rem)] sm:max-w-sm mx-4 sm:mx-auto p-0 overflow-hidden rounded-2xl">
                {/* Header */}
                <div className="px-6 pt-6 pb-4">
                  <div className="flex items-center gap-3">
                    <AlertCircle className="w-8 h-8 text-red-500" />
                    <div>
                      <AlertDialogTitle className="text-[#1f2937] text-base font-semibold">
                        {t('cancelTitle')}
                      </AlertDialogTitle>
                      <AlertDialogDescription className="text-xs text-[#6b7280] mt-0.5">
                        {t(`plans.${currentPlan}.name` as any)}プラン
                      </AlertDialogDescription>
                    </div>
                  </div>
                </div>

                {/* Content */}
                <div className="px-6 pb-4">
                  <p className="text-sm text-[#6b7280]">
                    {t('cancelDesc')}
                  </p>
                </div>

                {/* Footer */}
                <div className="px-6 py-4 bg-[#f9fafb] border-t border-[#f3f4f6] flex gap-2.5">
                  <AlertDialogCancel className="flex-1 h-10 rounded-lg border-[#d1d5db] text-[#374151] text-sm font-medium hover:bg-[#f3f4f6] transition-colors duration-200">
                    {t('keepSubscription')}
                  </AlertDialogCancel>
                  <AlertDialogAction
                    onClick={() => cancelMutation.mutate()}
                    className="flex-1 h-10 rounded-lg bg-red-500 hover:bg-red-600 border border-red-600 text-white text-sm font-medium transition-colors duration-200"
                  >
                    {t('confirmCancel')}
                  </AlertDialogAction>
                </div>
              </AlertDialogContent>
            </AlertDialog>
            )}
          </div>
        )}
      </div>

      {/* Memory-time usage this month */}
      <div className="mb-4 sm:mb-6">
        <UsageCard workspaceId={currentWorkspace?.id} />
      </div>

      {/* Two Column Layout: Current Plan | Invoice Calendar */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 sm:gap-6">
        {/* Left: Current Plan Receipt or Selected Invoice */}
        <div>
          <div className="flex items-center justify-between mb-2">
            <h2 className="text-sm font-medium text-gray-500">
              {selectedInvoice ? t('invoiceHistory') : t('currentPlan')}
            </h2>
            {selectedInvoice && (
              <button
                onClick={() => setSelectedInvoice(null)}
                className="text-xs text-gray-500 hover:text-gray-700 flex items-center gap-1"
              >
                <ArrowLeft className="w-3 h-3" />
                現在の請求書に戻る
              </button>
            )}
          </div>
          {selectedInvoice ? (
            <div className="bg-white border border-gray-300 rounded-lg overflow-hidden font-mono text-xs">
              <div className="px-4 py-3 text-center border-b border-dashed border-gray-300">
                <div className="font-bold text-gray-900">Nodeflare</div>
                <div className="text-xs text-gray-400">領収書</div>
              </div>
              <div className="px-4 py-2 border-b border-dashed border-gray-300">
                <div className="flex justify-between text-gray-600">
                  <span>発行日</span>
                  <span>{formatDate(selectedInvoice.created * 1000)}</span>
                </div>
                <div className="flex justify-between text-gray-600 mt-1">
                  <span>請求番号</span>
                  <span>{selectedInvoice.number || selectedInvoice.id.slice(0, 14)}</span>
                </div>
              </div>
              <div className="px-4 py-2 border-b border-dashed border-gray-300">
                <div className="flex justify-between text-gray-900">
                  <span>ご利用料金</span>
                  <span>¥{selectedInvoice.amount_paid.toLocaleString()}</span>
                </div>
              </div>
              <div className="px-4 py-2 bg-gray-50">
                <div className="flex justify-between text-gray-900 font-bold">
                  <span>合計（税込）</span>
                  <span>¥{selectedInvoice.amount_paid.toLocaleString()}</span>
                </div>
                <div className="flex justify-between mt-2 text-gray-500">
                  <span className={selectedInvoice.status === 'paid' ? 'text-green-600' : 'text-amber-600'}>
                    {selectedInvoice.status === 'paid' ? '支払済' : '未払い'}
                  </span>
                  {selectedInvoice.invoice_pdf && (
                    <a href={selectedInvoice.invoice_pdf} target="_blank" rel="noopener noreferrer" className="hover:text-gray-700 underline">
                      PDFをダウンロード
                    </a>
                  )}
                </div>
              </div>
            </div>
          ) : (
            <div className="bg-white border border-gray-300 rounded-lg overflow-hidden font-mono text-xs">
              <div className="px-4 py-3 text-center border-b border-dashed border-gray-300">
                <div className="font-bold text-gray-900">Nodeflare</div>
                <div className="text-xs text-gray-400">請求書</div>
              </div>
              <div className="px-4 py-2 border-b border-dashed border-gray-300">
                <div className="flex justify-between text-gray-600">
                  <span>発行日</span>
                  <span>{formatDate(new Date())}</span>
                </div>
                {subscription?.current_period_end && (
                  <div className="flex justify-between text-gray-600 mt-1">
                    <span>次回請求日</span>
                    <span>{formatDate(subscription.current_period_end * 1000)}</span>
                  </div>
                )}
              </div>
              <div className="px-4 py-2 border-b border-dashed border-gray-300">
                <div className="flex justify-between text-gray-900">
                  <span>{t(`plans.${currentPlan}.name` as any)}プラン</span>
                  <span>¥{planPrice.toLocaleString()}</span>
                </div>
              </div>
              <div className="px-4 py-2 bg-gray-50">
                <div className="flex justify-between text-gray-900 font-bold">
                  <span>合計（税込）</span>
                  <span>¥{totalMonthly.toLocaleString()}</span>
                </div>
              </div>
            </div>
          )}
        </div>

        {/* Right: Invoice Calendar */}
        <div>
          <h2 className="text-sm font-medium text-gray-500 mb-2">{t('invoiceHistory')}</h2>
          <div className="bg-white border border-gray-200 rounded-lg overflow-hidden">
            <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200">
              <button onClick={() => setCalendarDate(new Date(calendarDate.getFullYear(), calendarDate.getMonth() - 1, 1))} className="p-1 hover:bg-gray-100 rounded">
                <ChevronLeft className="w-4 h-4 text-gray-600" />
              </button>
              <span className="text-sm font-medium text-gray-900">{formatYearMonth(calendarDate)}</span>
              <button onClick={() => setCalendarDate(new Date(calendarDate.getFullYear(), calendarDate.getMonth() + 1, 1))} className="p-1 hover:bg-gray-100 rounded">
                <ChevronRight className="w-4 h-4 text-gray-600" />
              </button>
            </div>
            <div className="p-3">
              <div className="grid grid-cols-7 gap-1 mb-1">
                {['日', '月', '火', '水', '木', '金', '土'].map((day) => (
                  <div key={day} className="text-center text-xs text-gray-400 py-1">{day}</div>
                ))}
              </div>
              <div className="grid grid-cols-7 gap-1">
                {(() => {
                  const year = calendarDate.getFullYear();
                  const month = calendarDate.getMonth();
                  const firstDay = new Date(year, month, 1).getDay();
                  const daysInMonth = new Date(year, month + 1, 0).getDate();
                  const days = [];
                  for (let i = 0; i < firstDay; i++) days.push(<div key={`empty-${i}`} className="h-8" />);
                  for (let day = 1; day <= daysInMonth; day++) {
                    const invoiceOnDay = invoices.find((inv) => {
                      const d = new Date(inv.created * 1000);
                      return d.getFullYear() === year && d.getMonth() === month && d.getDate() === day;
                    });
                    const isSelected = selectedInvoice && (() => {
                      const d = new Date(selectedInvoice.created * 1000);
                      return d.getFullYear() === year && d.getMonth() === month && d.getDate() === day;
                    })();
                    days.push(
                      <button
                        key={day}
                        onClick={() => invoiceOnDay && setSelectedInvoice(isSelected ? null : invoiceOnDay)}
                        className={`h-8 w-full rounded text-xs transition-colors ${invoiceOnDay ? isSelected ? 'bg-violet-600 text-white font-medium' : 'bg-violet-100 text-violet-700 hover:bg-violet-200 font-medium' : 'text-gray-600 hover:bg-gray-50'}`}
                        disabled={!invoiceOnDay}
                      >
                        {day}
                      </button>
                    );
                  }
                  return days;
                })()}
              </div>
            </div>
            <div className="px-3 py-2 border-t border-gray-200 flex items-center gap-3 text-xs text-gray-500">
              <div className="flex items-center gap-1"><div className="w-2 h-2 rounded bg-violet-100" /><span>引き落とし</span></div>
            </div>
            {/* Bulk Export by Date Range */}
            <div className="px-3 py-3 border-t border-gray-200 bg-gray-50">
              <div className="flex items-center gap-2 text-xs">
                <input
                  type="month"
                  value={exportFrom}
                  onChange={(e) => setExportFrom(e.target.value)}
                  placeholder="開始月"
                  className="border border-gray-300 rounded px-2 py-1 text-xs w-28 bg-white"
                />
                <span className="text-gray-400">〜</span>
                <input
                  type="month"
                  value={exportTo}
                  onChange={(e) => setExportTo(e.target.value)}
                  placeholder="終了月"
                  className="border border-gray-300 rounded px-2 py-1 text-xs w-28 bg-white"
                />
                <button
                  onClick={() => {
                    if (!exportFrom || !exportTo) return;
                    const from = new Date(exportFrom + '-01');
                    const to = new Date(exportTo + '-01');
                    to.setMonth(to.getMonth() + 1);
                    const filtered = invoices.filter((inv) => {
                      const d = new Date(inv.created * 1000);
                      return d >= from && d < to;
                    });
                    if (filtered.length > 0) {
                      filtered.forEach((inv) => {
                        if (inv.invoice_pdf) window.open(inv.invoice_pdf, '_blank');
                      });
                    } else {
                      alert('該当期間の請求書がありません');
                    }
                  }}
                  disabled={!exportFrom || !exportTo}
                  className={`px-3 py-1 rounded text-xs transition-colors ${exportFrom && exportTo ? 'bg-violet-600 text-white hover:bg-violet-700' : 'bg-gray-200 text-gray-400'}`}
                >
                  一括DL
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Payment Method & Invoice Email Settings */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 sm:gap-6">
        {/* Payment Method */}
        <div>
          <h2 className="text-sm font-medium text-gray-500 mb-2">{t('paymentMethod.title')}</h2>
          <div className="bg-white border border-gray-200 rounded-lg p-4 h-[72px] flex items-center">
            {paymentMethod ? (
              <div className="flex items-center justify-between w-full">
                <div className="flex items-center gap-3">
                  <div className="w-10 h-6 bg-gray-100 rounded flex items-center justify-center text-xs font-bold text-gray-600 uppercase">
                    {paymentMethod.brand.slice(0, 4)}
                  </div>
                  <div className="text-sm">
                    <div className="text-gray-900">**** {paymentMethod.last4}</div>
                    <div className="text-gray-500 text-xs">
                      {String(paymentMethod.exp_month).padStart(2, '0')}/{String(paymentMethod.exp_year).slice(-2)}
                    </div>
                  </div>
                </div>
                <button onClick={() => portalMutation.mutate()} disabled={portalMutation.isPending} className="text-sm text-gray-600 hover:text-gray-900 underline">変更</button>
              </div>
            ) : subscription?.stripe_customer_id ? (
              <div className="flex items-center justify-between w-full">
                {paymentMethodLoading ? (
                  <div className="flex items-center gap-2 text-sm text-gray-500">
                    <div className="w-4 h-4 border-2 rounded-full border-gray-300 border-t-violet-600 animate-spin" />
                    <span>カード情報を取得中...</span>
                  </div>
                ) : (
                  <div className="text-sm text-gray-500">カード情報がありません</div>
                )}
                <button onClick={() => portalMutation.mutate()} disabled={portalMutation.isPending} className="text-sm text-gray-600 hover:text-gray-900 underline">
                  {paymentMethod ? '変更' : '追加'}
                </button>
              </div>
            ) : (
              <div className="text-sm text-gray-500">{t('paymentMethod.noMethod')}</div>
            )}
          </div>
        </div>

        {/* Invoice Email Toggle */}
        <div>
          <h2 className="text-sm font-medium text-gray-500 mb-2">請求書メール</h2>
          <div className="bg-white border border-gray-200 rounded-lg p-4 h-[72px] flex items-center">
            <label className="flex items-center justify-between w-full text-sm text-[#6b7280] cursor-pointer">
              <span>請求書を毎月メールで受け取る</span>
              <button
                onClick={() => updateBillingSettingsMutation.mutate(!autoEmailEnabled)}
                disabled={updateBillingSettingsMutation.isPending}
                className={`relative w-10 h-5 rounded-full transition-colors duration-200 ${autoEmailEnabled ? 'bg-violet-500' : 'bg-[#d1d5db]'} ${updateBillingSettingsMutation.isPending ? 'opacity-50' : ''}`}
              >
                <span className={`absolute top-0.5 w-4 h-4 bg-white rounded-full shadow-sm transition-transform duration-200 ${autoEmailEnabled ? 'left-[22px]' : 'left-0.5'}`} />
              </button>
            </label>
          </div>
        </div>
      </div>

      {/* Subscription History */}
      {subscriptionHistory.length > 0 && (
        <div>
          <h2 className="text-sm font-medium text-gray-500 mb-2">サブスクリプション履歴</h2>
          <div className="bg-white border border-gray-200 rounded-lg overflow-x-auto scrollbar-hide -mx-4 sm:mx-0">
            <table className="w-full text-sm min-w-[480px]">
              <thead className="bg-gray-50 border-b border-gray-200">
                <tr>
                  <th className="text-left px-4 py-2 font-medium text-gray-600 whitespace-nowrap">プラン</th>
                  <th className="text-left px-4 py-2 font-medium text-gray-600 whitespace-nowrap">ステータス</th>
                  <th className="text-left px-4 py-2 font-medium text-gray-600 whitespace-nowrap">期間</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100">
                {subscriptionHistory.map((sub) => {
                  const isCanceled = sub.status === 'canceled';
                  const isCanceling = sub.cancel_at_period_end && sub.status === 'active';
                  const periodEnd = new Date(sub.current_period_end * 1000);
                  const endedAt = sub.ended_at ? new Date(sub.ended_at * 1000) : null;

                  return (
                    <tr key={sub.id} className={isCanceled ? 'bg-gray-50' : ''}>
                      <td className="px-4 py-3">
                        <span className={`font-medium capitalize ${isCanceled ? 'text-gray-400' : 'text-gray-900'}`}>
                          {t(`plans.${sub.plan}.name` as any)}
                        </span>
                      </td>
                      <td className="px-4 py-3">
                        {isCanceled ? (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs bg-gray-100 text-gray-600">
                            <X className="w-3 h-3" />
                            解約済み
                          </span>
                        ) : isCanceling ? (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs bg-amber-100 text-amber-700">
                            <Clock className="w-3 h-3" />
                            {formatShortDate(periodEnd)}まで
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs bg-green-100 text-green-700">
                            <Check className="w-3 h-3" />
                            有効
                          </span>
                        )}
                      </td>
                      <td className="px-4 py-3 text-gray-500 whitespace-nowrap">
                        {isCanceled && endedAt ? (
                          <span>{formatDate(endedAt)}に終了</span>
                        ) : (
                          <span>
                            {formatShortDate(sub.current_period_start * 1000)}
                            {' 〜 '}
                            {formatShortDate(periodEnd)}
                          </span>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* Upgrade Link */}
      {(isFree || subscription?.cancel_at_period_end) && !showPlans && (
        <button onClick={() => setShowPlans(true)} className="text-sm text-violet-600 hover:text-violet-700">
          {subscription?.cancel_at_period_end ? 'プランを選択 →' : 'プランをアップグレード →'}
        </button>
      )}

      {/* Plans Grid */}
      {showPlans && (
        <>
          {/* Plan Selector */}
          <div className="flex justify-center">
            <div className="inline-flex items-center bg-muted rounded-lg p-1">
              <button
                className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
                  selectedInterval === 'monthly' ? 'bg-background shadow' : 'text-muted-foreground'
                }`}
                onClick={() => setSelectedInterval('monthly')}
              >
                {t('interval.monthly')}
              </button>
              <button
                className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
                  selectedInterval === 'yearly' ? 'bg-background shadow' : 'text-muted-foreground'
                }`}
                onClick={() => setSelectedInterval('yearly')}
              >
                {t('interval.yearly')} <span className="text-green-600 text-xs ml-1">{t('interval.save')}</span>
              </button>
            </div>
          </div>

          {/* Plans Grid */}
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 sm:gap-6">
            {plans?.map((plan) => {
              const isCurrent = currentPlan === plan.plan;
              const isEnterprise = plan.plan === 'enterprise';
              const price = selectedInterval === 'yearly' ? plan.price_yearly_jpy : plan.price_monthly_jpy;
              const monthlyPrice = price !== null ? (selectedInterval === 'yearly' ? Math.round(price / 12) : price) : null;
              const planKey = plan.plan as 'free' | 'pro' | 'team' | 'enterprise';
              const features = t.raw(`plans.${planKey}.features`) as string[];

              return (
                <Card key={plan.plan} className={`relative ${isCurrent ? 'border-primary ring-2 ring-primary' : ''}`}>
                  {isCurrent && (
                    <div className="absolute -top-3 left-1/2 -translate-x-1/2 px-3 py-1 bg-primary text-primary-foreground text-xs font-medium rounded-full">
                      {t('currentPlan')}
                    </div>
                  )}
                  <CardHeader>
                    <CardTitle>{t(`plans.${planKey}.name`)}</CardTitle>
                    <CardDescription>{t(`plans.${planKey}.description`)}</CardDescription>
                  </CardHeader>
                  <CardContent>
                    <div className="mb-6">
                      {isEnterprise ? (
                        <>
                          <span className="text-2xl font-bold">{t('contactSales')}</span>
                          <p className="text-sm text-muted-foreground mt-1">
                            {t('enterpriseCustomPricing')}
                          </p>
                        </>
                      ) : (
                        <>
                          <span className="text-4xl font-bold">¥{monthlyPrice?.toLocaleString()}</span>
                          <span className="text-muted-foreground">{t('perMonth')}</span>
                          {selectedInterval === 'yearly' && price !== null && price > 0 && (
                            <p className="text-sm text-muted-foreground">
                              {t('billedYearly', { price: price.toLocaleString() })}
                            </p>
                          )}
                        </>
                      )}
                    </div>

                    <ul className="space-y-3">
                      {features.map((feature, index) => (
                        <li key={index} className="flex items-start gap-2 text-sm">
                          <Check className="w-5 h-5 text-green-500 flex-shrink-0 mt-0.5" />
                          {feature}
                        </li>
                      ))}
                    </ul>
                  </CardContent>
                  <CardFooter>
                    {plan.plan === 'free' ? (
                      <Button variant="outline" className="w-full" disabled={isCurrent && !subscription?.cancel_at_period_end}>
                        {isCurrent && !subscription?.cancel_at_period_end ? t('currentPlan') : t('downgrade')}
                      </Button>
                    ) : isEnterprise ? (
                      <Button
                        className="w-full"
                        variant="default"
                        asChild
                      >
                        <a href="/contact">{t('contactSales')}</a>
                      </Button>
                    ) : (
                      <Button
                        className="w-full"
                        variant={isCurrent && !subscription?.cancel_at_period_end ? 'outline' : 'default'}
                        disabled={(isCurrent && !subscription?.cancel_at_period_end) || checkoutMutation.isPending}
                        onClick={() => {
                          setCheckoutPlan(plan.plan);
                          checkoutMutation.mutate({ plan: plan.plan, yearly: selectedInterval === 'yearly' });
                        }}
                      >
                        {isCurrent && !subscription?.cancel_at_period_end
                          ? t('currentPlan')
                          : isCurrent && subscription?.cancel_at_period_end
                            ? t('reactivate')
                            : checkoutPlan === plan.plan
                              ? tCommon('loading')
                              : t('upgrade')}
                      </Button>
                    )}
                  </CardFooter>
                </Card>
              );
            })}
          </div>
        </>
      )}

    </div>
  );
}

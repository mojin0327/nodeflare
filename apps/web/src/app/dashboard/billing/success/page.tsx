'use client';

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useTranslations } from 'next-intl';
import { useQuery } from '@tanstack/react-query';
import { Button } from '@/components/ui/button';
import { CheckCircle } from 'lucide-react';
import Link from 'next/link';
import { api } from '@/lib/api';
import { BillingSubscription } from '@/types';
import { useWorkspace } from '@/hooks/use-workspace';

export default function BillingSuccessPage() {
  const t = useTranslations('billing.success');
  const router = useRouter();
  const [isComplete, setIsComplete] = useState(false);
  const [elapsedTime, setElapsedTime] = useState(0);

  const { activeWorkspace: currentWorkspace } = useWorkspace();

  // Poll subscription status
  const { data: subscription } = useQuery<BillingSubscription>({
    queryKey: ['subscription-check', currentWorkspace?.id],
    queryFn: () => api.get(`/workspaces/${currentWorkspace?.id}/billing/subscription`),
    enabled: !!currentWorkspace?.id && !isComplete,
    refetchInterval: 5000,
  });

  // Check if subscription is active (not free)
  useEffect(() => {
    if (subscription && subscription.plan !== 'free' && subscription.status === 'active') {
      setIsComplete(true);
    }
  }, [subscription]);

  // Track elapsed time
  useEffect(() => {
    const timer = setInterval(() => {
      setElapsedTime((prev) => prev + 1);
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  // Redirect after completion
  useEffect(() => {
    if (isComplete) {
      const timer = setTimeout(() => {
        router.push('/dashboard/billing');
      }, 2000);
      return () => clearTimeout(timer);
    }
  }, [isComplete, router]);

  // Timeout after 30 seconds - stop polling and show manual check message
  const isTimeout = elapsedTime >= 30;

  if (isComplete) {
    return (
      <div className="flex items-center justify-center min-h-[60vh] px-4">
        <div className="max-w-sm w-full text-center">
          <CheckCircle className="w-10 h-10 mx-auto text-green-500 mb-6" strokeWidth={1.5} />

          <h1 className="text-xl font-bold mb-2 text-[#323232]">{t('completeTitle')}</h1>
          <p className="text-sm text-muted-foreground leading-relaxed">
            {t('completeDescription')}
          </p>

          <div className="my-8 h-px bg-border" />

          <p className="text-xs text-muted-foreground/80 mb-6">
            {t('redirecting')}
          </p>

          <div className="flex gap-3 justify-center">
            <Button size="sm" asChild>
              <Link href="/dashboard/billing">{t('goToBilling')}</Link>
            </Button>
          </div>
        </div>
      </div>
    );
  }

  if (isTimeout) {
    return (
      <div className="flex items-center justify-center min-h-[60vh] px-4">
        <div className="max-w-sm w-full text-center">
          <div className="w-10 h-10 mx-auto border-4 rounded-full border-gray-200 border-t-violet-600 animate-spin mb-6" />

          <h1 className="text-xl font-bold mb-2 text-[#323232]">{t('timeoutTitle')}</h1>
          <p className="text-sm text-muted-foreground leading-relaxed">
            {t('timeoutDescription')}
          </p>

          <div className="my-8 h-px bg-border" />

          <div className="flex gap-3 justify-center">
            <Button variant="ghost" size="sm" asChild>
              <Link href="/dashboard">{t('goToDashboard')}</Link>
            </Button>
            <Button size="sm" asChild>
              <Link href="/dashboard/billing">{t('goToBilling')}</Link>
            </Button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex items-center justify-center min-h-[60vh] px-4">
      <div className="max-w-sm w-full text-center">
        <div className="w-10 h-10 mx-auto border-4 rounded-full border-gray-200 border-t-violet-600 animate-spin mb-6" />

        <h1 className="text-xl font-bold mb-2 text-[#323232]">{t('title')}</h1>
        <p className="text-sm text-muted-foreground leading-relaxed">
          {t('description')}
        </p>

        <div className="my-8 h-px bg-border" />

        <p className="text-xs text-muted-foreground/80 mb-6">
          {t('checkBilling')}
        </p>

        <div className="flex gap-3 justify-center">
          <Button variant="ghost" size="sm" asChild>
            <Link href="/dashboard">{t('goToDashboard')}</Link>
          </Button>
          <Button size="sm" asChild>
            <Link href="/dashboard/billing">{t('goToBilling')}</Link>
          </Button>
        </div>
      </div>
    </div>
  );
}

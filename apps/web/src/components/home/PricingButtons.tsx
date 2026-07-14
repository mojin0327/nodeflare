'use client';

import { useSearchParams } from 'next/navigation';
import { useTranslations } from 'next-intl';
import { Button } from '@/components/ui/button';
import { authUrl } from '@/lib/auth-urls';

interface PricingButtonsProps {
  variant: 'free' | 'pro';
}

export function PricingButtons({ variant }: PricingButtonsProps) {
  const t = useTranslations('home');
  const searchParams = useSearchParams();

  const returnTo = searchParams.get('return_to');
  const githubLoginUrl = authUrl('github', returnTo);

  if (variant === 'free') {
    return (
      <a href={githubLoginUrl} className="block">
        <Button variant="outline" className="w-full h-12 border-gray-300 hover:bg-gray-50">
          {t('pricing.free.cta')}
        </Button>
      </a>
    );
  }

  return (
    <a href={githubLoginUrl} className="block">
      <Button className="w-full h-12 bg-violet-500 hover:bg-violet-400 text-white">
        {t('pricing.pro.cta')}
      </Button>
    </a>
  );
}

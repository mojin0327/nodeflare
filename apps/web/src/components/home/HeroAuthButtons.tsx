'use client';

import { useTranslations } from 'next-intl';
import { Button } from '@/components/ui/button';
import Link from 'next/link';
import { LogIn } from 'lucide-react';

export function HeroAuthButtons() {
  const t = useTranslations('home');

  return (
    <div className="mt-8 flex flex-wrap justify-center gap-3">
      <Link href="/signup">
        <Button size="sm" className="h-7 text-xs px-2.5 bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white">
          <LogIn className="w-3.5 h-3.5 mr-1" />
          {t('getStarted')}
        </Button>
      </Link>
    </div>
  );
}

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
        <Button size="default" className="h-10 text-sm px-8 bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white">
          <LogIn className="w-4 h-4 mr-1.5" />
          {t('getStarted')}
        </Button>
      </Link>
    </div>
  );
}

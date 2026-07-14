'use client';

import { useState } from 'react';
import Image from 'next/image';
import Link from 'next/link';
import { useTranslations } from 'next-intl';
import { useAuth } from '@/hooks/use-auth';
import { Button } from '@/components/ui/button';
import { LocaleSwitcher } from '@/components/locale-switcher';
import { FaDiscord, FaXTwitter, FaGithub } from 'react-icons/fa6';

export function Header() {
  const { user, isLoading } = useAuth();
  const t = useTranslations('nav');
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);

  return (
    <header className="relative z-20">
      <div className="max-w-6xl mx-auto px-4 sm:px-6 h-16 flex items-center justify-between">
        <Link href="/" className="flex items-center gap-2">
          <Image src="/logo2.png" alt="Nodeflare" height={32} width={160} className="h-7 sm:h-8 w-auto" style={{ width: 'auto' }} />
        </Link>

        <nav className="hidden md:flex items-center gap-8">
          <Link href="/explore" className="text-sm font-medium text-[#323232] hover:text-gray-900">{t('explore')}</Link>
          <Link href="/docs" className="text-sm font-medium text-[#323232] hover:text-gray-900">{t('docs')}</Link>
          <Link href="/pricing" className="text-sm font-medium text-[#323232] hover:text-gray-900">Pricing</Link>
          <Link href="/blog" className="text-sm font-medium text-[#323232] hover:text-gray-900">{t('blog')}</Link>
        </nav>

        <div className="flex items-center gap-2 sm:gap-3">
          <div className="hidden md:flex items-center gap-3 mr-2">
            <a href="https://discord.gg/ZqHemHHmzd" target="_blank" rel="noopener noreferrer" className="text-gray-500 hover:text-gray-900 transition-colors">
              <FaDiscord className="w-5 h-5" />
            </a>
            <a href="https://x.com/3vvqu2dhUn36840" target="_blank" rel="noopener noreferrer" className="text-gray-500 hover:text-gray-900 transition-colors">
              <FaXTwitter className="w-5 h-5" />
            </a>
            <a href="https://github.com/nodeflare-dev/nodeflare" target="_blank" rel="noopener noreferrer" className="text-gray-500 hover:text-gray-900 transition-colors">
              <FaGithub className="w-5 h-5" />
            </a>
          </div>
          <LocaleSwitcher />
          {isLoading ? (
            <div className="w-16 sm:w-20 h-9 bg-gray-100 rounded-lg animate-pulse" />
          ) : user ? (
            <Link href="/dashboard">
              <Button className="h-9 px-3 sm:px-4 text-sm rounded-lg bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white">{t('dashboard')}</Button>
            </Link>
          ) : (
            <>
              <Link href="/login" className="hidden sm:block">
                <Button className="h-9 px-3 sm:px-4 text-sm rounded-lg bg-gray-200 hover:bg-gray-300 text-gray-700">{t('login')}</Button>
              </Link>
              <Link href="/signup" className="hidden sm:block">
                <Button className="h-9 px-3 sm:px-4 text-sm rounded-lg bg-violet-600 hover:bg-violet-700 border border-violet-900 text-white">{t('signup')}</Button>
              </Link>
            </>
          )}
          {/* Mobile menu button */}
          <button
            onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
            className="p-2 -mr-2 rounded-md hover:bg-gray-100 transition-colors md:hidden"
          >
            <svg className="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              {mobileMenuOpen ? (
                <>
                  <line x1="18" y1="6" x2="6" y2="18" />
                  <line x1="6" y1="6" x2="18" y2="18" />
                </>
              ) : (
                <>
                  <line x1="3" y1="12" x2="21" y2="12" />
                  <line x1="3" y1="6" x2="21" y2="6" />
                  <line x1="3" y1="18" x2="21" y2="18" />
                </>
              )}
            </svg>
          </button>
        </div>
      </div>

      {/* Mobile menu */}
      {mobileMenuOpen && (
        <div className="md:hidden border-t border-gray-100 bg-white">
          <nav className="max-w-6xl mx-auto px-4 py-4 space-y-1">
            <Link
              href="/explore"
              className="block px-3 py-2.5 text-sm font-medium text-[#323232] hover:text-gray-900 hover:bg-gray-50 rounded-lg"
              onClick={() => setMobileMenuOpen(false)}
            >
              {t('explore')}
            </Link>
            <Link
              href="/docs"
              className="block px-3 py-2.5 text-sm font-medium text-[#323232] hover:text-gray-900 hover:bg-gray-50 rounded-lg"
              onClick={() => setMobileMenuOpen(false)}
            >
              {t('docs')}
            </Link>
            <Link
              href="/pricing"
              className="block px-3 py-2.5 text-sm font-medium text-[#323232] hover:text-gray-900 hover:bg-gray-50 rounded-lg"
              onClick={() => setMobileMenuOpen(false)}
            >
              Pricing
            </Link>
            <Link
              href="/blog"
              className="block px-3 py-2.5 text-sm font-medium text-[#323232] hover:text-gray-900 hover:bg-gray-50 rounded-lg"
              onClick={() => setMobileMenuOpen(false)}
            >
              {t('blog')}
            </Link>
            <div className="flex items-center gap-4 px-3 py-2.5">
              <a href="https://discord.gg/ZqHemHHmzd" target="_blank" rel="noopener noreferrer" className="text-gray-500 hover:text-gray-900 transition-colors">
                <FaDiscord className="w-5 h-5" />
              </a>
              <a href="https://x.com/3vvqu2dhUn36840" target="_blank" rel="noopener noreferrer" className="text-gray-500 hover:text-gray-900 transition-colors">
                <FaXTwitter className="w-5 h-5" />
              </a>
              <a href="https://github.com/nodeflare-dev/nodeflare" target="_blank" rel="noopener noreferrer" className="text-gray-500 hover:text-gray-900 transition-colors">
                <FaGithub className="w-5 h-5" />
              </a>
            </div>
            {!isLoading && !user && (
              <div className="pt-3 border-t border-gray-100 mt-3 space-y-2">
                <Link
                  href="/login"
                  className="block w-full"
                  onClick={() => setMobileMenuOpen(false)}
                >
                  <Button variant="outline" className="w-full h-10 text-sm rounded-lg border-gray-300 text-gray-700">
                    {t('login')}
                  </Button>
                </Link>
                <Link
                  href="/signup"
                  className="block w-full"
                  onClick={() => setMobileMenuOpen(false)}
                >
                  <Button className="w-full h-10 text-sm rounded-lg bg-violet-600 hover:bg-violet-700 text-white">
                    {t('signup')}
                  </Button>
                </Link>
              </div>
            )}
          </nav>
        </div>
      )}
    </header>
  );
}

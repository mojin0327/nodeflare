'use client';

import Image from 'next/image';
import Link from 'next/link';
import { useSearchParams, useRouter } from 'next/navigation';
import { useState } from 'react';
import { useTranslations } from 'next-intl';
import { Button } from '@/components/ui/button';
import { FaGithub } from 'react-icons/fa6';
import { HiOutlineMail } from 'react-icons/hi';
import { login } from '@/lib/auth-api';
import { isSafeReturnTo } from '@/lib/safe-redirect';
import { authUrl } from '@/lib/auth-urls';
import { LoadingType } from '@/types';

const GoogleIcon = ({ className = "w-4 h-4" }: { className?: string }) => (
  <svg className={className} viewBox="0 0 24 24">
    <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z" />
    <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" />
    <path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" />
    <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" />
  </svg>
);

export default function LoginPage() {
  const t = useTranslations('auth.login');
  const tErrors = useTranslations('errors');
  const router = useRouter();
  const searchParams = useSearchParams();
  const returnTo = searchParams.get('return_to');

  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [isEmailNotVerified, setIsEmailNotVerified] = useState(false);
  const [loadingType, setLoadingType] = useState<LoadingType>(null);

  const dest = isSafeReturnTo(returnTo) ? returnTo! : '/dashboard';
  const githubLoginUrl = authUrl('github', dest);
  const googleLoginUrl = authUrl('google', dest);

  const handleEmailLogin = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    setLoadingType('email');

    try {
      await login(email, password);
      // Redirect to dashboard or return_to URL
      // Use window.location for full page reload to ensure cookies are applied.
      // SECURITY: only follow return_to if it is a safe same-origin relative path,
      // otherwise an attacker could set ?return_to=https://evil.com (open redirect) or
      // a javascript: URL (XSS). Falls back to /dashboard.
      const redirectUrl = isSafeReturnTo(returnTo) ? returnTo! : '/dashboard';
      window.location.href = redirectUrl;
    } catch (err) {
      const message = err instanceof Error ? err.message : tErrors('serverError');
      // Check if this is an email not verified error
      if (message.toLowerCase().includes('verify your email')) {
        setIsEmailNotVerified(true);
        setError(t('emailNotVerified'));
      } else {
        setIsEmailNotVerified(false);
        setError(message);
      }
      setLoadingType(null);
    }
  };

  const handleOAuthClick = (type: 'github' | 'google') => {
    setLoadingType(type);
  };

  return (
    <div className="min-h-screen flex">
      {/* Left side - Login form */}
      <div className="flex-1 flex flex-col px-4 py-6 sm:px-8 sm:py-8 lg:px-16">
        {/* Logo */}
        <Link href="/" className="flex items-center gap-2 mb-8 sm:mb-16">
          <Image src="/logo2.png" alt="Nodeflare" height={32} width={160} className="h-7 sm:h-8 w-auto" style={{ width: 'auto' }} />
        </Link>

        {/* Form container */}
        <div className="flex-1 flex items-center justify-center">
          <div className="w-full max-w-sm">
            <h1 className="text-xl sm:text-2xl font-normal text-gray-900 mb-6 sm:mb-8">{t('title')}</h1>

            {/* OAuth buttons */}
            <div className="flex flex-col sm:flex-row gap-3 mb-6">
              <a
                href={githubLoginUrl}
                className="flex-1"
                onClick={() => handleOAuthClick('github')}
              >
                <Button
                  variant="outline"
                  className="w-full h-11 text-sm font-medium rounded-lg border-gray-300 hover:bg-gray-50 gap-2"
                  disabled={loadingType !== null}
                >
                  <FaGithub className={`w-5 h-5 ${loadingType === 'github' ? 'animate-spin' : ''}`} />
                  {t('withGithub')}
                </Button>
              </a>
              <a
                href={googleLoginUrl}
                className="flex-1"
                onClick={() => handleOAuthClick('google')}
              >
                <Button
                  variant="outline"
                  className="w-full h-11 text-sm font-medium rounded-lg border-gray-300 hover:bg-gray-50 gap-2"
                  disabled={loadingType !== null}
                >
                  <GoogleIcon className={`w-4 h-4 ${loadingType === 'google' ? 'animate-spin' : ''}`} />
                  {t('withGoogle')}
                </Button>
              </a>
            </div>

            {/* Divider */}
            <div className="relative my-6">
              <div className="absolute inset-0 flex items-center">
                <div className="w-full border-t border-gray-200" />
              </div>
              <div className="relative flex justify-center text-sm">
                <span className="px-4 bg-white text-gray-500">{t('or')}</span>
              </div>
            </div>

            {/* Email form */}
            <form onSubmit={handleEmailLogin} className="space-y-4">
              {error && (
                <div className="p-3 text-sm text-red-600 bg-red-50 border border-red-200 rounded-lg">
                  <p>{error}</p>
                  {isEmailNotVerified && (
                    <Link
                      href="/verify-email"
                      className="block mt-2 text-violet-600 hover:text-violet-700 font-medium underline"
                    >
                      {t('resendVerification')}
                    </Link>
                  )}
                </div>
              )}

              <div>
                <label htmlFor="email" className="block text-sm font-medium text-gray-700 mb-1">
                  {t('email')}
                </label>
                <input
                  id="email"
                  type="email"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  className="w-full h-11 px-4 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
                  placeholder={t('emailPlaceholder')}
                  required
                  disabled={loadingType !== null}
                />
              </div>

              <div>
                <label htmlFor="password" className="block text-sm font-medium text-gray-700 mb-1">
                  {t('password')}
                </label>
                <input
                  id="password"
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full h-11 px-4 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
                  placeholder={t('passwordPlaceholder')}
                  required
                  disabled={loadingType !== null}
                />
              </div>

              <div className="flex justify-end">
                <Link href="/forgot-password" className="text-sm text-violet-600 hover:text-violet-700">
                  {t('forgotPassword')}
                </Link>
              </div>

              <Button
                type="submit"
                className="w-full h-11 text-sm font-medium rounded-lg bg-violet-600 hover:bg-violet-700 text-white gap-2"
                disabled={loadingType !== null}
              >
                {loadingType === 'email' ? (
                  <>
                    <HiOutlineMail className="w-5 h-5 animate-spin" />
                    {t('submit')}
                  </>
                ) : (
                  t('submit')
                )}
              </Button>
            </form>

            {/* Sign up link */}
            <p className="mt-8 text-center text-sm text-gray-600">
              <Link href="/signup" className="text-violet-600 hover:text-violet-700 font-medium">
                {t('needAccount')}
              </Link>
            </p>
          </div>
        </div>
      </div>

      {/* Right side - Decorative image */}
      <div className="hidden lg:block lg:flex-1 relative">
        <Image src="/sign.png" alt="" fill className="object-cover" />
      </div>
    </div>
  );
}

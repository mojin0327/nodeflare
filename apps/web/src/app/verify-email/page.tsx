'use client';

import Image from 'next/image';
import Link from 'next/link';
import { useSearchParams, useRouter } from 'next/navigation';
import { useState, useEffect } from 'react';
import { useTranslations } from 'next-intl';
import { Button } from '@/components/ui/button';
import { verifyEmail, resendVerification } from '@/lib/auth-api';

export default function VerifyEmailPage() {
  const t = useTranslations('auth.verifyEmail');
  const tErrors = useTranslations('errors');
  const router = useRouter();
  const searchParams = useSearchParams();
  const token = searchParams.get('token');

  const [status, setStatus] = useState<'loading' | 'success' | 'error' | 'resend'>('loading');
  const [error, setError] = useState('');
  const [email, setEmail] = useState('');
  const [resendSuccess, setResendSuccess] = useState(false);
  const [isResending, setIsResending] = useState(false);

  useEffect(() => {
    if (!token) {
      setStatus('resend');
      return;
    }

    const verify = async () => {
      try {
        await verifyEmail(token);
        // Auto-login successful, redirect to dashboard
        setStatus('success');
        // Small delay to show success message, then redirect
        setTimeout(() => {
          router.push('/dashboard');
        }, 1500);
      } catch (err) {
        const message = err instanceof Error ? err.message : tErrors('serverError');
        setError(message);
        setStatus('error');
      }
    };

    verify();
  }, [token, tErrors, router]);

  const handleResend = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsResending(true);
    setError('');

    try {
      await resendVerification(email);
      setResendSuccess(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : tErrors('serverError');
      setError(message);
    } finally {
      setIsResending(false);
    }
  };

  return (
    <div className="min-h-screen flex">
      {/* Left side - Content */}
      <div className="flex-1 flex flex-col px-4 py-6 sm:px-8 sm:py-8 lg:px-16">
        {/* Logo */}
        <Link href="/" className="flex items-center gap-2 mb-8 sm:mb-16">
          <Image src="/logo2.png" alt="Nodeflare" height={32} width={160} className="h-7 sm:h-8 w-auto" style={{ width: 'auto' }} />
        </Link>

        {/* Content container */}
        <div className="flex-1 flex items-center justify-center">
          <div className="w-full max-w-sm">
            {status === 'loading' && (
              <div className="text-center">
                <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-violet-600 mx-auto"></div>
                <p className="mt-4 text-gray-600">{t('verifying')}</p>
              </div>
            )}

            {status === 'success' && (
              <div className="text-center">
                <div className="flex items-center justify-center gap-2 mb-2">
                  <svg className="w-6 h-6 text-green-600" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M20 6L9 17l-5-5" />
                  </svg>
                  <h3 className="text-lg font-semibold text-gray-900">{t('successTitle')}</h3>
                </div>
                <p className="text-gray-600 mb-4">{t('redirecting')}</p>
                <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-violet-600 mx-auto"></div>
              </div>
            )}

            {status === 'error' && (
              <div className="bg-red-50 border border-red-200 rounded-xl p-8 text-center">
                <div className="w-12 h-12 bg-red-100 rounded-full flex items-center justify-center mx-auto mb-4">
                  <svg className="w-6 h-6 text-red-600" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </div>
                <h3 className="text-lg font-semibold text-gray-900 mb-1">{t('errorTitle')}</h3>
                <p className="text-red-600 mb-6">{error}</p>
                <Button
                  onClick={() => setStatus('resend')}
                  className="w-full h-11 text-sm font-medium rounded-lg bg-violet-600 hover:bg-violet-700 text-white"
                >
                  {t('resendEmail')}
                </Button>
              </div>
            )}

            {status === 'resend' && (
              <div>
                <h1 className="text-xl sm:text-2xl font-normal text-gray-900 mb-2">{t('resendTitle')}</h1>
                <p className="text-sm text-gray-600 mb-6 sm:mb-8">{t('resendDescription')}</p>

                {resendSuccess ? (
                  <div className="p-4 text-sm text-green-700 bg-green-50 border border-green-200 rounded-lg">
                    <p className="font-medium">{t('resendSuccessTitle')}</p>
                    <p className="mt-1">{t('resendSuccessMessage')}</p>
                  </div>
                ) : (
                  <form onSubmit={handleResend} className="space-y-4">
                    {error && (
                      <div className="p-3 text-sm text-red-600 bg-red-50 border border-red-200 rounded-lg">
                        {error}
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
                        disabled={isResending}
                      />
                    </div>

                    <Button
                      type="submit"
                      className="w-full h-11 text-sm font-medium rounded-lg bg-violet-600 hover:bg-violet-700 text-white"
                      disabled={isResending}
                    >
                      {isResending ? '...' : t('resendSubmit')}
                    </Button>
                  </form>
                )}
              </div>
            )}

            {/* Back to login link */}
            <p className="mt-8 text-center text-sm text-gray-600">
              <Link href="/login" className="text-violet-600 hover:text-violet-700 font-medium">
                {t('backToLogin')}
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

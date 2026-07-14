'use client';

import Image from 'next/image';
import Link from 'next/link';
import { useState } from 'react';
import { useTranslations } from 'next-intl';
import { Button } from '@/components/ui/button';
import { forgotPassword } from '@/lib/auth-api';

export default function ForgotPasswordPage() {
  const t = useTranslations('auth.forgotPassword');
  const tErrors = useTranslations('errors');

  const [email, setEmail] = useState('');
  const [error, setError] = useState('');
  const [success, setSuccess] = useState(false);
  const [isLoading, setIsLoading] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    setIsLoading(true);

    try {
      await forgotPassword(email);
      setSuccess(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : tErrors('serverError');
      setError(message);
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="min-h-screen flex">
      {/* Left side - Form */}
      <div className="flex-1 flex flex-col px-4 py-6 sm:px-8 sm:py-8 lg:px-16">
        {/* Logo */}
        <Link href="/" className="flex items-center gap-2 mb-8 sm:mb-16">
          <Image src="/logo2.png" alt="Nodeflare" height={32} width={160} className="h-7 sm:h-8 w-auto" style={{ width: 'auto' }} />
        </Link>

        {/* Form container */}
        <div className="flex-1 flex items-center justify-center">
          <div className="w-full max-w-sm">
            <h1 className="text-xl sm:text-2xl font-normal text-gray-900 mb-2">{t('title')}</h1>
            <p className="text-sm text-gray-600 mb-6 sm:mb-8">{t('description')}</p>

            {success ? (
              <div className="p-4 text-sm text-green-700 bg-green-50 border border-green-200 rounded-lg">
                <p className="font-medium">{t('successTitle')}</p>
                <p className="mt-1">{t('successMessage')}</p>
              </div>
            ) : (
              <form onSubmit={handleSubmit} className="space-y-4">
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
                    disabled={isLoading}
                  />
                </div>

                <Button
                  type="submit"
                  className="w-full h-11 text-sm font-medium rounded-lg bg-violet-600 hover:bg-violet-700 text-white"
                  disabled={isLoading}
                >
                  {isLoading ? '...' : t('submit')}
                </Button>
              </form>
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

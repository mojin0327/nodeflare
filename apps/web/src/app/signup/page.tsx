'use client';

import Image from 'next/image';
import Link from 'next/link';
import { useSearchParams } from 'next/navigation';
import { useState } from 'react';
import { useTranslations } from 'next-intl';
import { Button } from '@/components/ui/button';
import { FaGithub } from 'react-icons/fa6';
import { HiOutlineMail } from 'react-icons/hi';
import { register } from '@/lib/auth-api';
import { authUrl } from '@/lib/auth-urls';
import { LoadingType } from '@/types';
import { GoogleIcon } from '@/components/icons/google-icon';

export default function SignupPage() {
  const t = useTranslations('auth.signup');
  const tErrors = useTranslations('errors');
  const searchParams = useSearchParams();
  const returnTo = searchParams.get('return_to');

  const [name, setName] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [error, setError] = useState('');
  const [success, setSuccess] = useState(false);
  const [loadingType, setLoadingType] = useState<LoadingType>(null);

  const githubLoginUrl = authUrl('github', returnTo);
  const googleLoginUrl = authUrl('google', returnTo);

  const handleEmailSignup = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');

    // Validate password match
    if (password !== confirmPassword) {
      setError(tErrors('passwordMatch'));
      return;
    }

    // Validate password length
    if (password.length < 8) {
      setError(tErrors('passwordMin'));
      return;
    }

    setLoadingType('email');

    try {
      await register(email, password, name || email.split('@')[0]);
      setSuccess(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : tErrors('serverError');
      setError(message);
    } finally {
      setLoadingType(null);
    }
  };

  const handleOAuthClick = (type: 'github' | 'google') => {
    setLoadingType(type);
  };

  return (
    <div className="min-h-screen flex">
      {/* Left side - Signup form */}
      <div className="flex-1 flex flex-col px-4 py-6 sm:px-8 sm:py-8 lg:px-16">
        {/* Logo */}
        <Link href="/" className="flex items-center gap-2 mb-6 sm:mb-12">
          <Image src="/logo2.png" alt="Nodeflare" height={32} width={160} className="h-7 sm:h-8 w-auto" style={{ width: 'auto' }} />
        </Link>

        {/* Form container */}
        <div className="flex-1 flex items-center justify-center">
          <div className="w-full max-w-sm">
            <h1 className="text-xl sm:text-2xl font-normal text-gray-900 mb-5 sm:mb-8">{t('title')}</h1>

            {/* OAuth buttons */}
            <div className="flex flex-col sm:flex-row gap-3 mb-5 sm:mb-6">
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
            {success ? (
              <div className="p-4 text-sm text-green-700 bg-green-50 border border-green-200 rounded-lg">
                <p className="font-medium">{t('successTitle')}</p>
                <p className="mt-1">{t('successMessage')}</p>
              </div>
            ) : (
              <form onSubmit={handleEmailSignup} className="space-y-4">
                {error && (
                  <div className="p-3 text-sm text-red-600 bg-red-50 border border-red-200 rounded-lg">
                    {error}
                  </div>
                )}

                <div>
                  <label htmlFor="name" className="block text-sm font-medium text-gray-700 mb-1">
                    {t('name')}
                  </label>
                  <input
                    id="name"
                    type="text"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    className="w-full h-11 px-4 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
                    placeholder={t('namePlaceholder')}
                    disabled={loadingType !== null}
                  />
                </div>

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
                    minLength={8}
                    disabled={loadingType !== null}
                  />
                </div>

                <div>
                  <label htmlFor="confirmPassword" className="block text-sm font-medium text-gray-700 mb-1">
                    {t('confirmPassword')}
                  </label>
                  <input
                    id="confirmPassword"
                    type="password"
                    value={confirmPassword}
                    onChange={(e) => setConfirmPassword(e.target.value)}
                    className="w-full h-11 px-4 text-sm border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-violet-500 focus:border-transparent"
                    placeholder={t('confirmPasswordPlaceholder')}
                    required
                    disabled={loadingType !== null}
                  />
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
            )}

            {!success && (
              <>
                {/* Terms */}
                <p className="mt-4 text-center text-xs text-gray-500">
                  {t('terms')}{' '}
                  <Link href="/legal/terms" className="text-violet-600 hover:text-violet-700">
                    {t('termsOfService')}
                  </Link>{' '}
                  {t('and')}{' '}
                  <Link href="/legal/privacy" className="text-violet-600 hover:text-violet-700">
                    {t('privacyPolicy')}
                  </Link>
                </p>
              </>
            )}

            {/* Sign in link */}
            <p className="mt-6 text-center text-sm text-gray-600">
              {success ? t('verificationSent') : t('hasAccount')}{' '}
              <Link href="/login" className="text-violet-600 hover:text-violet-700 font-medium">
                {t('signIn')}
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

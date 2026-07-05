'use client';

import { Suspense, useEffect, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import Image from 'next/image';
import { api } from '@/lib/api';
import { useAuth } from '@/hooks/use-auth';
import { SquareLoader } from '@/components/ui/square-loader';
import { isSafeRedirectUri } from '@/lib/safe-redirect';

interface ClientInfo {
  client_id: string;
  client_name: string;
  scopes: string[];
}

function scopeLabel(scope: string): string {
  if (scope === '*') return 'Full access to your account and MCP servers';
  return scope;
}

function ConsentInner() {
  const sp = useSearchParams();
  const { refreshUser } = useAuth();
  const [phase, setPhase] = useState<'loading' | 'consent' | 'submitting' | 'error'>('loading');
  const [client, setClient] = useState<ClientInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const clientId = sp.get('client_id') || '';
  const redirectUri = sp.get('redirect_uri') || '';
  const scope = sp.get('scope') || '';
  const stateParam = sp.get('state') || '';
  const codeChallenge = sp.get('code_challenge') || '';
  const codeChallengeMethod = sp.get('code_challenge_method') || 'S256';
  const responseType = sp.get('response_type') || 'code';

  const fullPath = `/oauth/consent?${sp.toString()}`;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      // Require an authenticated session before showing the consent prompt.
      let user = null;
      try {
        user = (await refreshUser()).data ?? null;
      } catch {
        user = null;
      }
      if (cancelled) return;
      if (!user) {
        window.location.href = `/login?return_to=${encodeURIComponent(fullPath)}`;
        return;
      }
      if (!clientId || !redirectUri) {
        setError('Invalid authorization request.');
        setPhase('error');
        return;
      }
      try {
        const info = await api.get<ClientInfo>(
          `/oauth/client-info?client_id=${encodeURIComponent(clientId)}`,
        );
        if (cancelled) return;
        setClient(info);
        setPhase('consent');
      } catch {
        if (cancelled) return;
        setError('This application could not be verified.');
        setPhase('error');
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [refreshUser, clientId, redirectUri, fullPath]);

  // Navigate back to the client's redirect_uri with a query appended. SECURITY: the
  // redirect_uri comes from the URL, so validate it (absolute http(s) only) before
  // navigating — otherwise `redirect_uri=javascript:...` would run as XSS on click and a
  // non-registered URL would be an open redirect. Falls back to /dashboard when unsafe.
  const goToClient = (queryPart: string) => {
    if (!isSafeRedirectUri(redirectUri)) {
      window.location.href = '/dashboard';
      return;
    }
    const sep = redirectUri.includes('?') ? '&' : '?';
    window.location.href = `${redirectUri}${sep}${queryPart}`;
  };

  const approve = async () => {
    setPhase('submitting');
    try {
      // The backend validates redirect_uri/scope against the registered client and only
      // then issues a code, so we redirect to the (now-validated) redirect_uri.
      const res = await api.post<{ code: string }>('/oauth/authorize-code', {
        response_type: responseType,
        client_id: clientId,
        redirect_uri: redirectUri,
        code_challenge: codeChallenge,
        code_challenge_method: codeChallengeMethod,
        state: stateParam,
        scope,
      });
      goToClient(
        `code=${encodeURIComponent(res.code)}&state=${encodeURIComponent(stateParam)}`,
      );
    } catch {
      setError('Authorization failed. Please try again.');
      setPhase('error');
    }
  };

  const deny = () => {
    if (redirectUri) {
      goToClient(`error=access_denied&state=${encodeURIComponent(stateParam)}`);
    } else {
      window.location.href = '/dashboard';
    }
  };

  // The effective scopes the client is allowed (what will actually be granted).
  const grantedScopes = client?.scopes?.length ? client.scopes : scope ? scope.split(/\s+/) : ['*'];

  return (
    <div className="min-h-screen flex items-center justify-center bg-white px-4">
      <div className="w-full max-w-sm rounded-2xl border border-gray-200 shadow-sm p-6">
        <div className="flex flex-col items-center text-center">
          <Image src="/logo2.png" alt="Nodeflare" width={153} height={32} priority className="h-8 w-auto" />
        </div>

        {phase === 'loading' || phase === 'submitting' ? (
          <div className="flex flex-col items-center py-10">
            <SquareLoader />
            <p className="mt-5 text-sm text-gray-500">
              {phase === 'submitting' ? 'Authorizing…' : 'Loading…'}
            </p>
          </div>
        ) : phase === 'error' ? (
          <div className="py-8 text-center">
            <p className="text-red-600 font-medium">{error}</p>
            <button onClick={deny} className="mt-4 text-sm text-violet-600 hover:underline">
              Go back
            </button>
          </div>
        ) : (
          <>
            <h1 className="mt-6 text-center text-lg font-semibold text-gray-900">
              Authorize {client?.client_name || 'this application'}
            </h1>
            <p className="mt-2 text-center text-sm text-gray-500">
              <span className="font-medium text-gray-700">{client?.client_name || 'This app'}</span> is
              requesting access to your Nodeflare account:
            </p>

            <ul className="mt-4 space-y-2">
              {grantedScopes.map((s) => (
                <li key={s} className="flex items-start gap-2 text-sm text-gray-700">
                  <span className="mt-0.5 text-violet-600">✓</span>
                  <span>{scopeLabel(s)}</span>
                </li>
              ))}
            </ul>

            <div className="mt-6 flex flex-col gap-2">
              <button
                onClick={approve}
                className="w-full rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-700 active:bg-violet-800 transition-colors"
              >
                Authorize
              </button>
              <button
                onClick={deny}
                className="w-full rounded-lg border border-gray-300 bg-white px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors"
              >
                Cancel
              </button>
            </div>

            <p className="mt-4 text-center text-xs text-gray-400">
              You can revoke access anytime in your dashboard.
            </p>
          </>
        )}
      </div>
    </div>
  );
}

export default function OAuthConsentPage() {
  return (
    <Suspense
      fallback={
        <div className="min-h-screen flex items-center justify-center bg-white">
          <SquareLoader />
        </div>
      }
    >
      <ConsentInner />
    </Suspense>
  );
}

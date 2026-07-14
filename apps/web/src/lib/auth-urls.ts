export function authUrl(provider: 'github' | 'google', returnTo?: string | null): string {
  const base = `/api/v1/auth/${provider}`;
  return returnTo ? `${base}?return_to=${encodeURIComponent(returnTo)}` : base;
}

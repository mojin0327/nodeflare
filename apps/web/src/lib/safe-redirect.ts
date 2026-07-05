// SECURITY: shared guards against open-redirect and `javascript:`/`data:` XSS via
// user-controlled redirect targets. Always validate a redirect destination that came
// from the URL (query params) before assigning it to `window.location`.

/**
 * True only for a same-origin relative path (e.g. `/dashboard`). Rejects absolute URLs,
 * protocol-relative URLs (`//host`), dangerous schemes, and control characters.
 * Use for internal `return_to` navigation.
 */
export function isSafeReturnTo(url: string | null | undefined): boolean {
  if (!url || typeof url !== 'string') return false;
  const trimmed = url.trim();
  if (trimmed.length === 0) return false;
  // Must be a single-slash relative path; block `//` (protocol-relative) and absolutes.
  if (!trimmed.startsWith('/') || trimmed.startsWith('//')) return false;
  const lower = trimmed.toLowerCase();
  if (lower.includes('javascript:') || lower.includes('data:') || lower.includes('vbscript:')) {
    return false;
  }
  // Block control characters that could smuggle a scheme past the checks above.
  if (/[\x00-\x1f\x7f]/.test(trimmed)) return false;
  return true;
}

/**
 * True only for an absolute `http(s)` URL. Use for OAuth `redirect_uri` client callbacks,
 * where an absolute external URL is expected. Blocks `javascript:`/`data:`/custom schemes.
 * Note: this does not confirm the URL is a *registered* client redirect — the backend does
 * that when issuing a code — it only prevents script-scheme XSS and non-web redirects.
 */
export function isSafeRedirectUri(url: string | null | undefined): boolean {
  if (!url || typeof url !== 'string') return false;
  const trimmed = url.trim();
  if (trimmed.length === 0) return false;
  if (/[\x00-\x1f\x7f]/.test(trimmed)) return false;
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return false;
  }
  return parsed.protocol === 'https:' || parsed.protocol === 'http:';
}

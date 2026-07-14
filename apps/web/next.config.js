const createNextIntlPlugin = require('next-intl/plugin');
const withBundleAnalyzer = require('@next/bundle-analyzer')({
  enabled: process.env.ANALYZE === 'true',
});

const withNextIntl = createNextIntlPlugin('./src/i18n/request.ts');

// SECURITY: Content Security Policy configuration
// NOTE: 'unsafe-inline' and 'unsafe-eval' are used for Next.js compatibility:
// - unsafe-eval: Required for Next.js development and some production features
// - unsafe-inline: Required for Next.js inline styles and CSS-in-JS
// For stricter CSP, implement nonce-based CSP using Next.js middleware
// See: https://nextjs.org/docs/app/building-your-application/configuring/content-security-policy
const isDev = process.env.NODE_ENV === 'development';
// In development, we need unsafe-eval for hot module replacement
// In production, we use stricter settings but still need unsafe-inline for Next.js styling
const scriptSrc = isDev
  ? `'self' 'unsafe-eval' 'unsafe-inline' https://va.vercel-scripts.com`
  : `'self' 'unsafe-inline' https://va.vercel-scripts.com`; // Production: no unsafe-eval, reducing XSS attack surface
const cspHeader = `
  default-src 'self';
  script-src ${scriptSrc};
  style-src 'self' 'unsafe-inline';
  img-src 'self' data: https: blob:;
  font-src 'self';
  object-src 'none';
  connect-src 'self' ${process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8080'} ws://localhost:* wss://*.fly.dev wss://*.nodeflare.tech https://api-us-west-2.hygraph.com https://cdn.jsdelivr.net https://va.vercel-scripts.com https://*.vercel-insights.com;
  frame-ancestors 'none';
  base-uri 'self';
  form-action 'self';
  upgrade-insecure-requests;
`.replace(/\n/g, '');

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  output: 'standalone',
  images: {
    remotePatterns: [
      {
        protocol: 'https',
        hostname: 'avatars.githubusercontent.com',
        pathname: '/**',
      },
      {
        protocol: 'https',
        hostname: 'lh3.googleusercontent.com',
        pathname: '/**',
      },
      {
        protocol: 'https',
        hostname: 'img.youtube.com',
        pathname: '/**',
      },
      {
        protocol: 'https',
        hostname: 'i.pravatar.cc',
        pathname: '/**',
      },
    ],
  },
  experimental: {
    serverActions: {
      bodySizeLimit: '2mb',
    },
  },
  async headers() {
    return [
      {
        source: '/(.*)',
        headers: [
          {
            key: 'Content-Security-Policy',
            value: cspHeader,
          },
          {
            key: 'X-Content-Type-Options',
            value: 'nosniff',
          },
          {
            key: 'X-Frame-Options',
            value: 'DENY',
          },
          {
            key: 'X-XSS-Protection',
            value: '1; mode=block',
          },
          {
            key: 'Referrer-Policy',
            value: 'strict-origin-when-cross-origin',
          },
          {
            key: 'Permissions-Policy',
            value: 'camera=(), microphone=(), geolocation=()',
          },
        ],
      },
    ];
  },
  async rewrites() {
    // Use API_URL for server-side proxy (runtime env var)
    // Falls back to NEXT_PUBLIC_API_URL (build-time) or localhost
    const apiUrl = process.env.API_URL || process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8080';
    return [
      {
        source: '/api/v1/:path*',
        destination: `${apiUrl}/api/v1/:path*`,
      },
    ];
  },
};

module.exports = withBundleAnalyzer(withNextIntl(nextConfig));

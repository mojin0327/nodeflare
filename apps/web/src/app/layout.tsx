import type { Metadata } from 'next';
import { Inter, Kaushan_Script, Klee_One } from 'next/font/google';
import './globals.css';
import { Providers } from '@/components/providers';
import { NextIntlClientProvider } from 'next-intl';
import { getLocale, getMessages } from 'next-intl/server';
// TODO: Vercel Analytics disabled due to CSP eval conflict
// import { Analytics } from '@vercel/analytics/next';

const inter = Inter({ subsets: ['latin'] });
const dancingScript = Kaushan_Script({
  subsets: ['latin'],
  variable: '--font-dancing',
  weight: ['400'],
});
const kleeOne = Klee_One({
  subsets: ['latin', 'latin-ext', 'japanese'],
  variable: '--font-klee',
  weight: ['400', '600'],
});

export const metadata: Metadata = {
  metadataBase: new URL('https://nodeflare.tech'),
  title: {
    default: 'Nodeflare - Deploy MCP Servers in Seconds',
    template: '%s | Nodeflare',
  },
  description: 'Deploy and manage MCP servers with zero configuration. TypeScript & Python support, built-in secrets management, and global edge deployment.',
  keywords: ['MCP', 'Model Context Protocol', 'AI', 'server deployment', 'Claude', 'TypeScript', 'Python'],
  authors: [{ name: 'Nodeflare' }],
  creator: 'Nodeflare',
  openGraph: {
    type: 'website',
    locale: 'ja_JP',
    url: 'https://nodeflare.tech',
    siteName: 'Nodeflare',
    title: 'Nodeflare - Deploy MCP Servers in Seconds',
    description: 'Deploy and manage MCP servers with zero configuration. TypeScript & Python support, built-in secrets management, and global edge deployment.',
    images: [{ url: '/nodeflare_perfect_visual.png', width: 2560, height: 1440, alt: 'Nodeflare' }],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Nodeflare - Deploy MCP Servers in Seconds',
    description: 'Deploy and manage MCP servers with zero configuration.',
    images: ['/nodeflare_perfect_visual.png'],
  },
  robots: {
    index: true,
    follow: true,
    googleBot: {
      index: true,
      follow: true,
      'max-video-preview': -1,
      'max-image-preview': 'large',
      'max-snippet': -1,
    },
  },
  icons: {
    icon: '/favicon.png',
    apple: '/favicon.png',
  },
};

export default async function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const locale = await getLocale();
  const messages = await getMessages();

  return (
    <html lang={locale} suppressHydrationWarning>
      <body className={`${inter.className} ${dancingScript.variable} ${kleeOne.variable}`}>
        <NextIntlClientProvider messages={messages}>
          <Providers>{children}</Providers>
        </NextIntlClientProvider>
        {/* <Analytics /> */}
      </body>
    </html>
  );
}

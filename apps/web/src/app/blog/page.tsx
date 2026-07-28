import type { Metadata } from 'next';
import { Header, Footer } from '@/components/layout';
import Link from 'next/link';
import { getTranslations, getLocale } from 'next-intl/server';
import { formatLocalizedDate } from '@/lib/date-utils';
import { BlogPost } from '@/lib/hygraph';

// Map next-intl locale to Hygraph locales with fallback
function toHygraphLocales(locale: string): string[] {
  // Always include 'en' as fallback since content may only exist in English
  return locale === 'ja' ? ['ja_JP', 'en'] : ['en'];
}

// Use ISR for better SEO - revalidate every 60 seconds
export const revalidate = 60;

export async function generateMetadata(): Promise<Metadata> {
  const t = await getTranslations('blog');
  return {
    title: t('title'),
    alternates: {
      canonical: '/blog',
      languages: {
        'ja': '/blog',
        'en': '/blog',
      },
    },
  };
}

async function fetchBlogPosts(locales: string[]): Promise<BlogPost[]> {
  const token = process.env.HYGRAPH_TOKEN;
  if (!token) {
    return [];
  }

  try {
    const endpoint = process.env.HYGRAPH_ENDPOINT || 'https://api-us-west-2.hygraph.com/v2/cmmky48hh00h006w5q885vkcf/master';
    const res = await fetch(endpoint, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${token}`,
      },
      body: JSON.stringify({
        query: `
          query GetBlogPosts($locales: [Locale!]!) {
            blogPosts(locales: $locales, orderBy: publishDate_DESC, stage: PUBLISHED) {
              id
              title
              slug
              excerpt
              publishDate
              categories { id name slug }
            }
          }
        `,
        variables: { locales },
      }),
      cache: 'no-store',
    });

    const json = await res.json();
    if (json.errors) {
      return [];
    }
    return json.data?.blogPosts || [];
  } catch {
    return [];
  }
}

export default async function BlogPage() {
  const t = await getTranslations('blog');
  const locale = await getLocale();
  const posts = await fetchBlogPosts(toHygraphLocales(locale));

  return (
    <div className="min-h-screen bg-gray-50">
      <Header />

      <main className="max-w-6xl mx-auto px-6 py-16">
        <div className="mb-12">
          <h1 className="text-xl sm:text-2xl font-extrabold" style={{ color: '#323232' }}>{t('title')}</h1>
        </div>

        <div className="grid sm:grid-cols-2 lg:grid-cols-3 gap-3">
          {posts.map((post) => (
            <Link
              key={post.id}
              href={`/blog/${post.slug}`}
              className="group bg-white rounded-lg border border-gray-200 px-4 py-3 hover:border-violet-300 hover:bg-violet-50/30 transition-all"
            >
              <div className="flex items-center gap-2 mb-1">
                {post.categories[0] && (
                  <span className="text-xs font-semibold text-violet-600">
                    {post.categories[0].name}
                  </span>
                )}
                <span className="text-xs text-gray-400">
                  {post.publishDate && formatLocalizedDate(post.publishDate, locale, 'short')}
                </span>
              </div>
              <h2 className="text-base font-bold text-gray-900 group-hover:text-violet-600 transition-colors line-clamp-2">
                {post.title}
              </h2>
            </Link>
          ))}
        </div>

        {posts.length === 0 && (
          <div className="text-center py-20">
            <p className="text-gray-500">{t('empty')}</p>
          </div>
        )}
      </main>

      <Footer />
    </div>
  );
}

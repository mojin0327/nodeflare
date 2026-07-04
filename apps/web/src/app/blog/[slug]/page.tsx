import { Header, Footer } from '@/components/layout';
import Link from 'next/link';
import { getBlogPost, getBlogPosts, sanitizeHtml } from '@/lib/hygraph';
import { notFound } from 'next/navigation';
import { getLocale, getTranslations } from 'next-intl/server';

function formatDate(dateString: string | undefined, locale: string): string {
  if (!dateString) return '';
  const date = new Date(dateString);
  return date.toLocaleDateString(locale === 'ja' ? 'ja-JP' : 'en-US', {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
  });
}

// Use ISR for better SEO - revalidate every 60 seconds
export const revalidate = 60;

export async function generateStaticParams() {
  // Return empty array if Hygraph is not configured (build will use dynamic rendering)
  if (!process.env.HYGRAPH_TOKEN) {
    return [];
  }

  try {
    const posts = await getBlogPosts();
    return posts.map((post) => ({
      slug: post.slug,
    }));
  } catch {
    return [];
  }
}

export async function generateMetadata({ params }: { params: Promise<{ slug: string }> }) {
  const locale = await getLocale();
  const t = await getTranslations('blog');

  if (!process.env.HYGRAPH_TOKEN) {
    return { title: t('title') };
  }

  const { slug } = await params;
  try {
    const post = await getBlogPost(slug, locale);
    if (!post) {
      return { title: t('notFound') };
    }
    return {
      title: `${post.title} | ${t('title')}`,
      description: post.excerpt,
      alternates: {
        canonical: `/blog/${slug}`,
        languages: {
          'ja': `/blog/${slug}`,
          'en': `/blog/${slug}`,
        },
      },
    };
  } catch {
    return { title: t('title') };
  }
}

export default async function BlogPostPage({ params }: { params: Promise<{ slug: string }> }) {
  const locale = await getLocale();
  const t = await getTranslations('blog');

  if (!process.env.HYGRAPH_TOKEN) {
    notFound();
  }

  const { slug } = await params;
  let post;
  let otherPosts;
  try {
    [post, otherPosts] = await Promise.all([
      getBlogPost(slug, locale),
      getBlogPosts(locale),
    ]);
  } catch {
    notFound();
  }

  if (!post) {
    notFound();
  }

  // Get 3 other posts excluding current
  const relatedPosts = otherPosts
    .filter((p) => p.slug !== slug)
    .slice(0, 3);

  return (
    <div className="min-h-screen bg-white">
      <Header />

      <main className="max-w-3xl mx-auto px-6 py-20">
        {/* Back Link */}
        <Link
          href="/blog"
          className="inline-flex items-center gap-2 text-sm text-gray-500 hover:text-gray-900 transition-colors mb-12"
        >
          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
          </svg>
          {t('backToList')}
        </Link>

        <article>
          {/* Header */}
          <header className="mb-12">
            {/* Categories */}
            <div className="flex items-center gap-2 mb-6">
              {post.categories.map((category) => (
                <span
                  key={category.id}
                  className="text-xs font-medium text-violet-600 bg-violet-100 px-3 py-1 rounded-full"
                >
                  {category.name}
                </span>
              ))}
            </div>

            {/* Title */}
            <h1 className="text-3xl sm:text-4xl font-bold text-gray-900 tracking-tight leading-tight mb-6">
              {post.title}
            </h1>

            {/* Meta */}
            {post.publishDate && (
              <div className="flex items-center gap-4">
                <time className="text-sm text-gray-500">
                  {formatDate(post.publishDate, locale)}
                </time>
              </div>
            )}
          </header>

          {/* Excerpt */}
          {post.excerpt && (
            <div className="mb-10 pb-10 border-b border-gray-100">
              <p className="text-xl text-gray-600 leading-relaxed">
                {post.excerpt}
              </p>
            </div>
          )}

          {/* Content */}
          {post.content?.html && (
            <div
              className="prose prose-lg max-w-none prose-table:block prose-table:overflow-x-auto prose-table:w-full prose-th:whitespace-nowrap prose-td:whitespace-nowrap sm:prose-th:whitespace-normal sm:prose-td:whitespace-normal"
              dangerouslySetInnerHTML={{ __html: sanitizeHtml(post.content.html) }}
            />
          )}

        </article>

        {/* nodeflare Banner */}
        <a href="/api/v1/auth/github" className="block mt-16 group">
          {/* Mobile */}
          <div className="sm:hidden rounded-xl overflow-hidden border border-gray-300 bg-slate-100">
            <div className="p-4">
              <div className="mb-1">
                <img src="/logo2.png" alt="NodeFlare" className="h-7 w-auto" />
              </div>
              <p className="text-sm font-extrabold text-gray-800 !m-0 !leading-normal">
                {locale === 'ja' ? 'MCP専用ホスティングサービス' : 'MCP Hosting Service'}
              </p>
              <div className="text-xs font-semibold text-gray-400">
                {locale === 'ja' ? 'GitHubにあるMCPを簡単デプロイ・簡単運用で最適化' : 'Easily deploy and optimize MCP from GitHub'}
              </div>
            </div>
            <div className="bg-[#6d28d9] px-4 py-2.5 flex items-center justify-center gap-2">
              <span className="text-white text-sm font-bold">
                {locale === 'ja' ? '無料で始める' : 'Get Started'}
              </span>
              <svg className="w-4 h-4 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
              </svg>
            </div>
          </div>
          {/* Desktop */}
          <div
            className="hidden sm:block relative rounded-xl overflow-hidden py-3 px-6 border border-gray-300"
            style={{ background: 'linear-gradient(115deg, #f1f5f9 50%, #6d28d9 50%)' }}
          >
            {/* Security illustration in the purple region */}
            <img
              src="/secret_management.png"
              alt=""
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 left-[52%] -translate-y-1/2 h-[150%] w-auto object-contain mix-blend-screen"
            />

            <div className="relative flex items-center justify-between">
              <div>
                <div className="mb-0.5">
                  <img src="/logo2.png" alt="NodeFlare" className="h-8 w-auto" />
                </div>
                <p className="text-[17px] font-extrabold text-gray-800 !m-0 !leading-normal">
                  {locale === 'ja' ? 'MCP専用ホスティングサービス' : 'MCP Hosting Service'}
                </p>
                <div className="text-xs font-semibold text-gray-400">
                  {locale === 'ja' ? 'GitHubにあるMCPを簡単デプロイ・簡単運用で最適化' : 'Easily deploy and optimize MCP from GitHub'}
                </div>
              </div>

              <div className="flex items-center gap-2 text-[#323232] text-sm font-bold bg-white px-4 py-2 rounded-lg group-hover:bg-gray-100 transition-colors">
                {locale === 'ja' ? '無料で始める' : 'Get Started'}
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                </svg>
              </div>
            </div>
          </div>
        </a>

        {/* Related Posts */}
        {relatedPosts.length > 0 && (
          <div className="mt-16 pt-10 border-t border-gray-100">
            <h2 className="text-xl font-bold text-gray-900 mb-6">
              {locale === 'ja' ? '他の記事' : 'More Articles'}
            </h2>
            <div className="grid gap-6">
              {relatedPosts.map((relatedPost) => (
                <Link
                  key={relatedPost.id}
                  href={`/blog/${relatedPost.slug}`}
                  className="group block p-5 bg-gray-50 rounded-xl hover:bg-gray-100 transition-colors"
                >
                  <div className="flex items-center gap-2 mb-2">
                    {relatedPost.categories.slice(0, 1).map((cat) => (
                      <span
                        key={cat.id}
                        className="text-xs font-medium text-violet-600"
                      >
                        {cat.name}
                      </span>
                    ))}
                    {relatedPost.publishDate && (
                      <span className="text-xs text-gray-400">
                        {formatDate(relatedPost.publishDate, locale)}
                      </span>
                    )}
                  </div>
                  <h3 className="font-semibold text-gray-900 group-hover:text-violet-600 transition-colors mb-1">
                    {relatedPost.title}
                  </h3>
                  {relatedPost.excerpt && (
                    <p className="text-sm text-gray-500 line-clamp-2">
                      {relatedPost.excerpt}
                    </p>
                  )}
                </Link>
              ))}
            </div>
          </div>
        )}

        {/* Navigation */}
        <div className="mt-16 pt-10 border-t border-gray-100">
          <Link
            href="/blog"
            className="inline-flex items-center gap-2 text-sm font-medium text-violet-600 hover:text-violet-700 transition-colors"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
            </svg>
            {t('readMore')}
          </Link>
        </div>
      </main>

      <Footer />
    </div>
  );
}

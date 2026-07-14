'use client';

import { useState, useEffect } from 'react';
import { useTranslations } from 'next-intl';
import Link from 'next/link';
import { Button } from '@/components/ui/button';
import { formatLocalizedDate } from '@/lib/date-utils';

interface BlogPost {
  id: string;
  title: string;
  slug: string;
  excerpt?: string;
  publishDate?: string;
  categories: { id: string; name: string }[];
}

export function BlogSection() {
  const t = useTranslations('home');
  const [blogPosts, setBlogPosts] = useState<BlogPost[]>([]);

  useEffect(() => {
    const fetchBlogPosts = async () => {
      try {
        const res = await fetch('/api/blog');
        if (res.ok) {
          const posts = await res.json();
          setBlogPosts(posts.slice(0, 3));
        }
      } catch {
        // Silently fail - blog posts are not critical
      }
    };
    fetchBlogPosts();
  }, []);

  if (blogPosts.length === 0) {
    return null;
  }

  return (
    <section className="py-20">
      <div className="max-w-4xl mx-auto px-4 sm:px-6">
        <div className="flex items-end justify-between mb-8">
          <div>
            <span className="inline-block text-violet-600 text-sm font-medium mb-4">
              {t('blog.badge')}
            </span>
            <h2 className="text-2xl sm:text-3xl font-semibold tracking-tight" style={{ color: '#323232' }}>{t('blog.title')}</h2>
          </div>
          <Link href="/blog" className="hidden sm:flex items-center gap-2 text-violet-600 hover:text-violet-700 font-medium group">
            {t('blog.viewAll')}
            <svg className="w-4 h-4 group-hover:translate-x-1 transition-transform" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M5 12h14M12 5l7 7-7 7" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </Link>
        </div>

        <div className="grid sm:grid-cols-3 gap-3">
          {blogPosts.map((post) => (
            <Link key={post.id} href={`/blog/${post.slug}`} className="group bg-white rounded-lg border border-gray-200 px-4 py-3 hover:border-violet-300 hover:bg-violet-50/30 transition-all">
              <div className="flex items-center gap-2 mb-1">
                {post.categories[0] && (
                  <span className="text-xs font-semibold text-violet-600">
                    {post.categories[0].name}
                  </span>
                )}
                <span className="text-xs text-gray-400">{formatLocalizedDate(post.publishDate, 'ja', 'short')}</span>
              </div>
              <h3 className="text-base font-bold text-gray-900 group-hover:text-violet-600 transition-colors line-clamp-2">
                {post.title}
              </h3>
            </Link>
          ))}
        </div>

        <div className="mt-8 text-center sm:hidden">
          <Link href="/blog">
            <Button variant="outline" className="border-gray-300">{t('blog.viewAllArticles')}</Button>
          </Link>
        </div>
      </div>
    </section>
  );
}

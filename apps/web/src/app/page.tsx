import type { Metadata } from 'next';
import { getTranslations } from 'next-intl/server';
import { Suspense } from 'react';

// Use ISR for better SEO - revalidate every 60 seconds
export const revalidate = 60;

export const metadata: Metadata = {
  alternates: {
    canonical: '/',
    languages: {
      'ja': '/',
      'en': '/',
    },
  },
};
import Image from 'next/image';
import { FaDiscord } from 'react-icons/fa6';
import { Header, Footer } from '@/components/layout';
import {
  OAuthHandler,
  HeroAuthButtons,
  DashboardSlideshow,
  FAQAccordion,
  PricingButtons,
  BlogSection,
} from '@/components/home';

export default async function HomePage() {
  const t = await getTranslations('home');

  const features = [
    { titleKey: 'features.zeroConfig.title', descKey: 'features.zeroConfig.desc', icon: <><path d="M12 2a10 10 0 1 0 10 10H12V2z" /><path d="M21.18 8.02A10 10 0 0 0 12 2v10h10a10 10 0 0 0-0.82-3.98z" /></>, align: 'left', image: '/stdio.png' },
    { titleKey: 'features.acl.title', descKey: 'features.acl.desc', icon: <><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" /><path d="M9 12l2 2 4-4" /></>, align: 'right', image: '/fine_grained_permissions.png' },
    { titleKey: 'features.secrets.title', descKey: 'features.secrets.desc', icon: <><rect x="3" y="11" width="18" height="11" rx="2" /><path d="M7 11V7a5 5 0 0 1 10 0v4" /></>, align: 'left', image: '/secret_management.png' },
    { titleKey: 'features.protocol.title', descKey: 'features.protocol.desc', icon: <><path d="M22 12h-4l-3 9L9 3l-3 9H2" /></>, align: 'right', image: '/audit_access_logs.png' },
    { titleKey: 'features.alwaysOn.title', descKey: 'features.alwaysOn.desc', icon: <><circle cx="12" cy="12" r="10" /><path d="M2 12h20M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" /></>, align: 'left', image: '/mcp_leakage_prevention.png' },
  ];

  const freeFeatures = ['pricing.free.feature1', 'pricing.free.feature2', 'pricing.free.feature3', 'pricing.free.feature4'];
  const proFeatures = ['pricing.pro.feature1', 'pricing.pro.feature2', 'pricing.pro.feature3', 'pricing.pro.feature4', 'pricing.pro.feature5'];

  return (
    <div className="min-h-screen bg-white">
      {/* OAuth Handler - invisible component for OAuth redirect processing */}
      <Suspense fallback={null}>
        <OAuthHandler />
      </Suspense>

      <main>
        {/* Hero - クリーンな中央寄せ + デプロイカード */}
        <section className="relative isolate overflow-hidden">
          {/* 上部の淡いグラデーション */}
          <div className="absolute inset-x-0 top-0 -z-10 h-[420px] bg-gradient-to-b from-violet-50/70 via-white/40 to-transparent pointer-events-none" />
          {/* 中央のぼやっとしたテーマカラーの光 */}
          <div
            className="absolute left-1/2 top-1/2 -z-10 h-[560px] w-[860px] max-w-[95vw] -translate-x-1/2 -translate-y-1/2 pointer-events-none"
            style={{
              background: 'radial-gradient(ellipse at center, rgba(124,58,237,0.38) 0%, rgba(124,58,237,0.18) 35%, rgba(124,58,237,0) 70%)',
            }}
          />

          {/* ヘッダーをヒーロー背景の上に重ねて、bgを共有する */}
          <Header />

          <div className="relative max-w-5xl mx-auto px-6 pt-20 pb-24 sm:pt-24 sm:pb-32 text-center">
            <h1 className="mx-auto max-w-3xl text-5xl font-medium leading-[1.05] tracking-tight text-[#333333] sm:text-6xl lg:text-7xl">
              {t('heroTitle1')} <span className="font-semibold text-violet-600">{t('heroTitle2')}</span>{t('heroTitle3')}
            </h1>

            <p className="mx-auto mt-6 max-w-2xl text-base font-medium leading-loose text-[#333333] sm:text-lg">
              {t('heroDescription1')} {t('heroDescription2')}
            </p>

            <Suspense fallback={
              <div className="mt-8 flex flex-wrap justify-center gap-3">
                <div className="h-10 w-40 animate-pulse rounded-lg bg-violet-100" />
                <div className="h-10 w-24 animate-pulse rounded-lg bg-gray-100" />
              </div>
            }>
              <HeroAuthButtons />
            </Suspense>
          </div>
        </section>

        {/* Dashboard Preview - カード重ねデザイン */}
        <DashboardSlideshow />

        {/* Features - ベントーグリッド */}
        <section className="py-24">
          <div className="max-w-6xl mx-auto px-6">
            <div className="max-w-4xl mb-6">
              <span className="text-sm font-semibold text-violet-600">
                {t('features.badge')}
              </span>
              <h2 className="mt-3 text-2xl sm:text-3xl font-medium tracking-tight text-[#323232]">{t('features.title')}</h2>
            </div>

            <div className="grid gap-px overflow-hidden rounded-2xl border-2 border-gray-200/80 bg-gray-200/70 shadow-xl shadow-gray-200/60 sm:grid-cols-2 lg:grid-cols-6">
              {features.map((item, idx) => (
                <div
                  key={idx}
                  className={`group flex flex-col bg-white p-6 transition-colors hover:bg-gray-50/70 sm:p-8 ${idx < 2 ? 'lg:col-span-3' : 'lg:col-span-2'}`}
                >
                  {/* アイコン＋タイトル（横並び・アイコン色は文字と統一） */}
                  <div className="mb-2 flex items-center gap-2.5">
                    <svg className="h-5 w-5 shrink-0 text-[#323232]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      {item.icon}
                    </svg>
                    <h3 className="text-base font-semibold text-[#323232]">{t(item.titleKey)}</h3>
                  </div>
                  <p className="mb-8 max-w-md text-sm leading-relaxed text-gray-500">{t(item.descKey)}</p>

                  {/* 画像スペース（全カード共通の固定高さ・比率維持で枠内に収め、画像の有無/元サイズに左右されない） */}
                  <div className={`-mx-6 -mb-6 sm:-mx-8 sm:-mb-8 relative ${idx < 2 ? 'h-64' : 'h-40'}`}>
                    {item.image ? (
                      <Image src={item.image} alt={t(item.titleKey)} fill className="object-contain" />
                    ) : null}
                  </div>
                </div>
              ))}
            </div>
          </div>
        </section>

        {/* Code Example - サイドバイサイド + シンタックスハイライト */}
        <section className="py-20">
          <div className="max-w-4xl mx-auto px-4 sm:px-6">
            <div className="grid lg:grid-cols-2 gap-12 items-center">
              <div>
                <span className="text-sm font-semibold text-violet-600">
                  {t('devExperience.badge')}
                </span>
                <h2 className="mt-3 mb-5 text-3xl font-semibold tracking-tight text-[#323232]">
                  {t('devExperience.title')}
                </h2>
                <p className="text-lg text-gray-600 mb-5 leading-relaxed">
                  {t('devExperience.description')}
                </p>
                <p className="text-base text-gray-600 leading-relaxed">
                  {t('devExperience.descriptionExtra')}
                </p>
              </div>

              <div className="relative">
                <img
                  src="/c1.png"
                  alt="Access Control Settings"
                  className="w-full rounded-2xl"
                />
              </div>
            </div>
          </div>
        </section>

        {/* Bare Metal - 画像左・テキスト右 */}
        <section className="py-20">
          <div className="max-w-4xl mx-auto px-4 sm:px-6">
            <div className="grid lg:grid-cols-2 gap-12 items-center">
              <div className="relative order-2 lg:order-1">
                <img
                  src="/c2.png"
                  alt="Dedicated bare metal servers"
                  className="w-full rounded-2xl"
                />
              </div>

              <div className="order-1 lg:order-2">
                <span className="text-sm font-semibold text-violet-600">
                  {t('bareMetal.badge')}
                </span>
                <h2 className="mt-3 mb-5 text-3xl font-semibold tracking-tight text-[#323232]">
                  {t('bareMetal.title')}
                </h2>
                <p className="text-lg text-gray-600 mb-5 leading-relaxed">
                  {t('bareMetal.description')}
                </p>
                <p className="text-base text-gray-600 leading-relaxed">
                  {t('bareMetal.descriptionExtra')}
                </p>
              </div>
            </div>
          </div>
        </section>

        {/* Discord - コミュニティ紹介（チャットプレビュー） */}
        <section
          className="py-20"
          style={{ background: 'linear-gradient(135deg, #6b78f5 0%, #5865F2 50%, #4954c9 100%)' }}
        >
          <div className="max-w-5xl mx-auto px-4 sm:px-6">
            <div className="grid items-center gap-10 lg:grid-cols-2">
              {/* 左: 見出し + 説明 + CTA */}
              <div>
                <div className="inline-flex items-center gap-2 rounded-full bg-white/15 px-3 py-1 text-xs font-medium text-white">
                  <FaDiscord className="h-4 w-4" />
                  Community
                </div>
                <h2 className="mt-4 text-3xl font-semibold tracking-tight text-white">
                  {t('discord.title')}
                </h2>
                <p className="mt-3 max-w-md text-lg leading-loose text-white/80">
                  {t('discord.description')}
                </p>
                <a
                  href="https://discord.gg/ZqHemHHmzd"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="mt-6 inline-flex items-center gap-2 rounded-lg bg-white px-6 py-3 text-sm font-semibold text-[#5865F2] transition-colors hover:bg-white/90"
                >
                  <FaDiscord className="h-5 w-5" />
                  {t('discord.cta')}
                </a>
              </div>

              {/* 右: Discord風チャットモック */}
              <div className="flex h-[420px] overflow-hidden rounded-xl bg-[#313338] shadow-2xl ring-1 ring-black/30">
                {/* チャンネルサイドバー */}
                <div className="hidden w-44 shrink-0 flex-col bg-[#2b2d31] sm:flex">
                  <div className="flex h-12 shrink-0 items-center justify-between border-b border-black/30 px-3 text-[15px] font-semibold text-white shadow-sm">
                    NodeFlare
                    <svg className="h-4 w-4 text-gray-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"><path d="M6 9l6 6 6-6" /></svg>
                  </div>
                  <div className="space-y-0.5 px-2 py-3 text-[14px] font-medium">
                    <div className="flex items-center gap-1.5 rounded bg-[#404249] px-2 py-1 text-white">
                      <span className="text-gray-400">#</span> mcp-updates
                    </div>
                    <div className="flex items-center gap-1.5 px-2 py-1 text-gray-400">
                      <span className="text-gray-500">#</span> showcase
                    </div>
                    <div className="flex items-center gap-1.5 px-2 py-1 text-gray-400">
                      <span className="text-gray-500">#</span> automation
                    </div>
                    <div className="flex items-center gap-1.5 px-2 py-1 text-gray-400">
                      <span className="text-gray-500">#</span> support
                    </div>
                    <div className="flex items-center gap-1.5 px-2 py-1 text-gray-400">
                      <svg className="h-3.5 w-3.5 text-gray-500" viewBox="0 0 24 24" fill="currentColor"><path d="M11 5L6 9H2v6h4l5 4V5zm4.54.46a1 1 0 0 0-1.41 1.42 5 5 0 0 1 0 7.07 1 1 0 1 0 1.41 1.41 7 7 0 0 0 0-9.9z" /></svg>
                      community
                    </div>
                  </div>
                </div>

                {/* メイン: ヘッダー + メッセージ + 入力 */}
                <div className="flex min-w-0 flex-1 flex-col">
                  <div className="flex h-12 shrink-0 items-center gap-2 border-b border-black/30 px-4 shadow-sm">
                    <svg className="h-5 w-5 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M4 9h16M4 15h16M10 3L8 21M16 3l-2 18" /></svg>
                    <span className="shrink-0 whitespace-nowrap text-[15px] font-semibold text-white">mcp-updates</span>
                    <span className="ml-2 hidden truncate border-l border-white/10 pl-2 text-xs text-gray-400 lg:inline">Latest MCP releases</span>
                    <div className="ml-auto flex shrink-0 items-center gap-1 text-xs text-gray-400">
                      <span className="h-2 w-2 rounded-full bg-green-500" />
                      86
                    </div>
                  </div>

                  <div className="flex flex-1 flex-col justify-end gap-3 overflow-hidden px-3 py-3">
                    {[
                      { name: 'alex', avatarUrl: 'https://i.pravatar.cc/80?img=12', nameColor: 'text-[#f2f3f5]', time: '14:14', textKey: 'discord.chat.msg1', reaction: '🎉', count: 4 },
                      { name: 'mei', avatarUrl: 'https://i.pravatar.cc/80?img=47', nameColor: 'text-[#f2f3f5]', time: '14:15', textKey: 'discord.chat.msg2' },
                      { name: 'nodeflare', icon: true, avatar: 'bg-[#5865f2]', nameColor: 'text-[#9b8cff]', tag: 'TEAM', time: '14:16', textKey: 'discord.chat.msg3', reaction: '🚀', count: 7 },
                    ].map((m) => (
                      <div key={m.name} className="group flex gap-3 rounded px-1 py-0.5 hover:bg-black/10">
                        <div className="mt-0.5 shrink-0">
                          {m.icon ? (
                            <div className={`flex h-10 w-10 items-center justify-center rounded-full ${m.avatar}`}>
                              <FaDiscord className="h-5 w-5 text-white" />
                            </div>
                          ) : (
                            <Image src={m.avatarUrl ?? ''} alt={m.name} width={40} height={40} className="h-10 w-10 rounded-full object-cover" />
                          )}
                        </div>
                        <div className="min-w-0">
                          <div className="flex items-baseline gap-2">
                            <span className={`text-[15px] font-medium ${m.nameColor}`}>{m.name}</span>
                            {m.tag && (
                              <span className="rounded bg-[#5865f2] px-1.5 py-px text-[10px] font-bold uppercase leading-4 text-white">{m.tag}</span>
                            )}
                            <span className="text-[11px] text-gray-500">{m.time}</span>
                          </div>
                          <p className="text-[15px] leading-snug text-[#dbdee1]">{t(m.textKey)}</p>
                          {m.reaction && (
                            <div className="mt-1 inline-flex items-center gap-1 rounded-md border border-[#5865f2]/50 bg-[#5865f2]/15 px-1.5 py-0.5 text-xs font-medium text-gray-200">
                              <span>{m.reaction}</span>
                              <span>{m.count}</span>
                            </div>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>

                  <div className="px-3 pb-3">
                    <div className="flex items-center gap-3 rounded-lg bg-[#383a40] px-4 py-2.5 text-sm text-gray-400">
                      <svg className="h-5 w-5 text-gray-500" viewBox="0 0 24 24" fill="currentColor"><path d="M12 2a10 10 0 100 20 10 10 0 000-20zm5 11h-4v4h-2v-4H7v-2h4V7h2v4h4v2z" /></svg>
                      {t('discord.chat.input')}
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* Pricing - 非対称カード */}
        <section className="py-20 bg-gray-50">
          <div className="max-w-4xl mx-auto px-4 sm:px-6">
            <div className="text-center mb-10">
              <span className="text-sm font-semibold text-violet-600">
                {t('pricing.badge')}
              </span>
              <h2 className="mt-3 mb-3 text-2xl sm:text-3xl font-semibold tracking-tight text-[#323232]">{t('pricing.title')}</h2>
              <p className="text-lg text-gray-600">{t('pricing.subtitle')}</p>
            </div>

            <div className="grid md:grid-cols-2 gap-6">
              {/* Free */}
              <div className="relative group">
                <div className="relative bg-white rounded-2xl p-8 border border-gray-200 hover:border-gray-300 hover:shadow-lg transition-all h-full">
                  <div className="text-sm font-medium text-gray-500 mb-2">{t('pricing.free.name')}</div>
                  <div className="flex items-baseline gap-1 mb-6">
                    <span className="text-5xl font-bold text-[#323232]">{t('pricing.free.price')}</span>
                    <span className="text-gray-500">{t('pricing.perMonth')}</span>
                  </div>
                  <p className="text-gray-600 mb-8">{t('pricing.free.description')}</p>
                  <ul className="space-y-4 mb-8">
                    {freeFeatures.map((featureKey) => (
                      <li key={featureKey} className="flex items-center gap-3 text-gray-700">
                        <svg className="w-5 h-5 text-gray-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                          <path d="M20 6L9 17l-5-5" strokeLinecap="round" strokeLinejoin="round" />
                        </svg>
                        {t(featureKey)}
                      </li>
                    ))}
                  </ul>
                  <Suspense fallback={<div className="h-12 bg-gray-100 rounded-lg animate-pulse" />}>
                    <PricingButtons variant="free" />
                  </Suspense>
                </div>
              </div>

              {/* Pro */}
              <div className="relative group">
                <div className="absolute -inset-[1.5px] bg-violet-500 rounded-2xl" />
                <div className="relative bg-white rounded-2xl p-8 h-full">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-sm font-medium text-violet-600">{t('pricing.pro.name')}</span>
                    <span className="px-2.5 py-0.5 rounded-full bg-violet-100 text-violet-700 text-xs font-medium">{t('pricing.pro.badge')}</span>
                  </div>
                  <div className="flex items-baseline gap-1 mb-6">
                    <span className="text-5xl font-bold text-[#323232]">{t('pricing.pro.price')}</span>
                    <span className="text-gray-500">{t('pricing.perMonth')}</span>
                  </div>
                  <p className="text-gray-600 mb-8">{t('pricing.pro.description')}</p>
                  <ul className="space-y-4 mb-8">
                    {proFeatures.map((featureKey) => (
                      <li key={featureKey} className="flex items-center gap-3 text-gray-700">
                        <svg className="w-5 h-5 text-violet-600" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                          <path d="M20 6L9 17l-5-5" strokeLinecap="round" strokeLinejoin="round" />
                        </svg>
                        {t(featureKey)}
                      </li>
                    ))}
                  </ul>
                  <Suspense fallback={<div className="h-12 bg-violet-500/50 rounded-lg animate-pulse" />}>
                    <PricingButtons variant="pro" />
                  </Suspense>
                </div>
              </div>
            </div>
          </div>
        </section>

        {/* FAQ - インタラクティブアコーディオン */}
        <FAQAccordion />

        {/* Blog - マガジンレイアウト */}
        <BlogSection />
      </main>

      <Footer />
    </div>
  );
}

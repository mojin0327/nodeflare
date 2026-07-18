'use client';

import { useState, useRef, useEffect } from 'react';
import { useLocale } from 'next-intl';
import { locales, localeNames, Locale } from '@/i18n/config';

const localeCountry: Record<Locale, string> = {
  ja: 'jp',
  en: 'us',
};

function Flag({ locale, className = '' }: { locale: Locale; className?: string }) {
  // flag-icons: `.fi` sizes off font-size (width 1.333em, height 1em)
  return (
    <span
      className={`fi fi-${localeCountry[locale]} shrink-0 rounded-[3px] leading-none ${className}`}
      aria-hidden="true"
    />
  );
}

export function LocaleSwitcher() {
  const locale = useLocale() as Locale;
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    }
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const handleChange = (newLocale: Locale) => {
    document.cookie = `locale=${newLocale};path=/;max-age=31536000`;
    setIsOpen(false);
    window.location.reload();
  };

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="flex items-center gap-1.5 px-2.5 py-2 rounded-lg text-gray-600 hover:text-gray-900 hover:bg-gray-100 transition-colors"
        aria-label="言語を選択"
      >
        <Flag locale={locale} className="text-[19px]" />
        <svg
          className={`w-4 h-4 transition-transform ${isOpen ? 'rotate-180' : ''}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>

      {isOpen && (
        <div className="absolute left-0 mt-2 w-40 bg-white rounded-xl shadow-lg border border-gray-200 overflow-hidden z-50 animate-in fade-in slide-in-from-top-2 duration-200">
          {locales.map((loc) => (
            <button
              key={loc}
              onClick={() => handleChange(loc)}
              className={`w-full flex items-center gap-3 px-4 py-2.5 text-sm transition-colors ${
                locale === loc
                  ? 'bg-violet-50 text-violet-700 font-medium'
                  : 'text-gray-700 hover:bg-gray-50'
              }`}
            >
              <Flag locale={loc} className="text-lg" />
              <span>{localeNames[loc]}</span>
              {locale === loc && (
                <svg
                  className="w-4 h-4 ml-auto text-violet-600"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M20 6L9 17l-5-5" />
                </svg>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function formatDate(date: Date | string | number): string {
  const d = date instanceof Date ? date : new Date(date);
  return d.toLocaleDateString('ja-JP', { year: 'numeric', month: 'long', day: 'numeric' });
}

export function formatYearMonth(date: Date | string | number): string {
  const d = date instanceof Date ? date : new Date(date);
  return d.toLocaleDateString('ja-JP', { year: 'numeric', month: 'long' });
}

export function formatShortDate(date: Date | string | number): string {
  const d = date instanceof Date ? date : new Date(date);
  return d.toLocaleDateString('ja-JP', { month: 'short', day: 'numeric' });
}

export function formatSlashDate(date: Date | string | number): string {
  const d = date instanceof Date ? date : new Date(date);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}/${m}/${day}`;
}

export function formatLocalizedDate(
  date: Date | string | undefined,
  locale: string,
  monthFormat: 'short' | 'long' = 'long',
): string {
  if (!date) return '';
  const d = date instanceof Date ? date : new Date(date);
  return d.toLocaleDateString(locale === 'ja' ? 'ja-JP' : 'en-US', {
    year: 'numeric',
    month: monthFormat,
    day: 'numeric',
  });
}

export function formatDateTime(date: Date | string | number): string {
  const d = date instanceof Date ? date : new Date(date);
  return d.toLocaleString('ja-JP', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}

export function getRelativeTime(dateStr: string | null | undefined, t: (key: string) => string): string {
  if (!dateStr) return '-';
  const date = new Date(dateStr);
  if (isNaN(date.getTime())) return '-';

  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60000);
  const diffHours = Math.floor(diffMins / 60);
  const diffDays = Math.floor(diffHours / 24);
  const diffWeeks = Math.floor(diffDays / 7);

  if (diffMins < 1) return t('detail.justNow');
  if (diffMins < 60) return `${diffMins}${t('detail.minutesAgo')}`;
  if (diffHours < 24) return `${diffHours}${t('detail.hoursAgo')}`;
  if (diffDays < 7) return `${diffDays}${t('detail.daysAgo')}`;
  return `${diffWeeks}${t('detail.weeksAgo')}`;
}

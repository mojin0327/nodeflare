'use client';
import { useState, useRef, useEffect } from 'react';
import { ChevronDown, Settings2 } from 'lucide-react';
import { Icon, addCollection } from '@iconify/react';
import logosData from '@iconify-json/logos/icons.json';
import { cn } from '@/lib/utils';
import { UpstreamOAuthProvider } from '@/types';

addCollection(logosData);

const PROVIDER_LOGO_ICONS: Record<string, string> = {
  google: 'logos:google-icon',
  github: 'logos:github-icon',
  microsoft: 'logos:microsoft-icon',
  slack: 'logos:slack-icon',
  notion: 'logos:notion-icon',
  linear: 'logos:linear-icon',
};

interface ProviderSelectProps {
  value: string;
  onChange: (value: string) => void;
  providers: UpstreamOAuthProvider[];
  placeholder: string;
  className?: string;
}

function ProviderIcon({ name }: { name: string }) {
  const icon = PROVIDER_LOGO_ICONS[name];
  if (!icon) return <Settings2 className="w-4 h-4 flex-shrink-0 text-gray-400" />;
  return <Icon icon={icon} className="w-4 h-4 flex-shrink-0" />;
}

export function ProviderSelect({ value, onChange, providers, placeholder, className }: ProviderSelectProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, []);

  const selected = providers.find((p) => p.name === value) ?? null;

  return (
    <div ref={ref} className={cn('relative w-48', className)}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={cn(
          'flex h-10 w-full items-center gap-2 appearance-none rounded-[10px] border border-input bg-background px-3 pr-9 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2',
          open && 'ring-2 ring-ring ring-offset-2'
        )}
      >
        {selected ? (
          <>
            <ProviderIcon name={selected.name} />
            <span className="truncate text-left">{selected.display_name}</span>
          </>
        ) : (
          <span className="text-muted-foreground truncate">{placeholder}</span>
        )}
      </button>
      <ChevronDown className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 text-gray-400 transition-colors" />

      {open && (
        <div className="absolute z-50 mt-1 w-full overflow-hidden rounded-[10px] border border-input bg-background shadow-md">
          <button
            type="button"
            onClick={() => { onChange(''); setOpen(false); }}
            className={cn(
              'flex w-full items-center px-3 py-2 text-sm text-muted-foreground hover:bg-gray-50',
              !value && 'bg-violet-50 text-violet-700'
            )}
          >
            {placeholder}
          </button>
          {providers.map((p) => (
            <button
              key={p.name}
              type="button"
              onClick={() => { onChange(p.name); setOpen(false); }}
              className={cn(
                'flex w-full items-center gap-2 px-3 py-2 text-sm hover:bg-gray-50',
                value === p.name && 'bg-violet-50 text-violet-700'
              )}
            >
              <ProviderIcon name={p.name} />
              <span className="flex-1 text-left">{p.display_name}</span>
              {!p.is_managed && <span className="text-xs text-gray-400">(Custom)</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

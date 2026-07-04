'use client';

import Link from 'next/link';
import { useTranslations } from 'next-intl';
import { ChevronDown } from 'lucide-react';
import { Label } from '@/components/ui/label';
import { Select } from '@/components/ui/select';
import { MEMORY_LADDER_MB } from '@/lib/plans';

/** Human label for a memory size, e.g. 256 -> "256 MB", 2048 -> "2 GB". */
function formatSize(mb: number): string {
  return mb >= 1024 ? `${mb / 1024} GB` : `${mb} MB`;
}

interface MemorySelectProps {
  /** Currently selected size in MB. */
  value: number;
  onChange: (mb: number) => void;
  /** The plan's per-server memory ceiling (MB). Sizes above this are locked. */
  maxMemoryMb: number;
  id?: string;
}

/**
 * Per-server machine memory picker. Sizes above the workspace plan's ceiling are
 * disabled and labelled as upgrade-only, mirroring the backend's plan enforcement
 * (a too-large choice is rejected with MEMORY_LIMIT_REACHED). When the plan caps
 * below the largest rung, an inline upgrade link is shown.
 */
export function MemorySelect({ value, onChange, maxMemoryMb, id = 'memory_mb' }: MemorySelectProps) {
  const t = useTranslations('servers');
  const capped = maxMemoryMb < MEMORY_LADDER_MB[MEMORY_LADDER_MB.length - 1];

  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{t('create.machineMemory')}</Label>
      <p className="text-xs text-muted-foreground">{t('create.machineMemoryHelp')}</p>
      <div className="relative w-full sm:w-48">
        <Select
          id={id}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
          className="peer w-full appearance-none rounded-[10px] pr-9"
        >
          {MEMORY_LADDER_MB.map((mb) => {
            const locked = mb > maxMemoryMb;
            return (
              <option key={mb} value={mb} disabled={locked}>
                {formatSize(mb)}
                {locked ? ` — ${t('create.machineMemoryLocked')}` : ''}
              </option>
            );
          })}
        </Select>
        {/* Custom caret: inset from the edge and light when idle, darker on focus. */}
        <ChevronDown className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 text-gray-400 transition-colors peer-focus:text-gray-600" />
      </div>
      {capped && (
        <p className="text-xs text-muted-foreground">
          {t('upgrade.memoryLimit')}{' '}
          <Link href="/dashboard/billing" className="text-primary hover:underline">
            {t('upgrade.cta')} →
          </Link>
        </p>
      )}
    </div>
  );
}

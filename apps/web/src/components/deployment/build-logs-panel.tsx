'use client';

import { useEffect, useRef, useState, useMemo, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { useTranslations } from 'next-intl';
import { useBuildLogsWebSocket } from '@/hooks/use-websocket';
import { api } from '@/lib/api';
import { cn } from '@/lib/utils';

const LOG_LINE_STYLES: Record<string, string> = {
  error: 'text-red-400 bg-red-500/10',
  warning: 'text-yellow-400 bg-yellow-500/5',
  success: 'text-green-400',
  step: 'text-cyan-400 font-medium',
  default: 'text-gray-300',
};

interface BuildLogsPanelProps {
  deploymentId: string;
  workspaceId?: string;
  serverId?: string;
  className?: string;
  maxHeight?: string;
  allowFullscreen?: boolean;
}

interface LogsResponse {
  logs: string | null;
}

interface ParsedLogLine {
  lineNumber: number;
  content: string;
  type: 'info' | 'error' | 'warning' | 'success' | 'step';
}

// Extended BuildLogLine with pre-computed type
interface RealtimeLogLine {
  deployment_id: string;
  line: string;
  stream: 'stdout' | 'stderr';
  timestamp: string;
  type: ParsedLogLine['type'];
}

// Detect log line type based on content
function detectLogType(line: string): ParsedLogLine['type'] {
  const lowerLine = line.toLowerCase();

  // Error patterns
  if (
    lowerLine.includes('error') ||
    lowerLine.includes('failed') ||
    lowerLine.includes('fatal') ||
    lowerLine.includes('exception') ||
    line.startsWith('ERR') ||
    line.startsWith('E ')
  ) {
    return 'error';
  }

  // Warning patterns
  if (
    lowerLine.includes('warning') ||
    lowerLine.includes('warn') ||
    line.startsWith('WARN') ||
    line.startsWith('W ')
  ) {
    return 'warning';
  }

  // Success patterns
  if (
    lowerLine.includes('success') ||
    lowerLine.includes('completed') ||
    lowerLine.includes('done') ||
    lowerLine.includes('finished') ||
    line.includes('✓') ||
    line.includes('✔')
  ) {
    return 'success';
  }

  // Step patterns (build steps)
  if (
    line.startsWith('Step ') ||
    line.startsWith('---') ||
    line.startsWith('==>') ||
    line.startsWith('>>>')
  ) {
    return 'step';
  }

  return 'info';
}

// Parse historical logs into structured format
function parseHistoricalLogs(logs: string): ParsedLogLine[] {
  return logs.split('\n').map((line, index) => ({
    lineNumber: index + 1,
    content: line,
    type: detectLogType(line),
  }));
}

export function BuildLogsPanel({
  deploymentId,
  workspaceId,
  serverId,
  className,
  maxHeight = '400px',
  allowFullscreen = true,
}: BuildLogsPanelProps) {
  const t = useTranslations('servers.detail');
  const [realtimeLogs, setRealtimeLogs] = useState<RealtimeLogLine[]>([]);
  const [historicalLogs, setHistoricalLogs] = useState<string | null>(null);
  // Auto-scroll is disabled by default - only enable when real-time logs arrive
  const [autoScroll, setAutoScroll] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const fullscreenContainerRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  // Handle ESC key to close fullscreen
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isFullscreen) {
        setIsFullscreen(false);
      }
    };
    if (isFullscreen) {
      document.addEventListener('keydown', handleKeyDown);
      document.body.style.overflow = 'hidden';
    }
    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      document.body.style.overflow = '';
    };
  }, [isFullscreen]);

  const toggleFullscreen = useCallback(() => {
    setIsFullscreen((prev) => !prev);
  }, []);

  // Parse historical logs
  const parsedHistoricalLogs = useMemo(() => {
    if (!historicalLogs) return [];
    return parseHistoricalLogs(historicalLogs);
  }, [historicalLogs]);

  // Fetch historical logs from API
  useEffect(() => {
    if (!workspaceId || !serverId || !deploymentId) {
      setIsLoading(false);
      return;
    }

    const fetchLogs = async () => {
      try {
        const response = await api.get<LogsResponse>(
          `/workspaces/${workspaceId}/servers/${serverId}/deployments/${deploymentId}/logs`
        );
        setHistoricalLogs(response.logs);
      } catch {
        // Silently fail - historical logs are not critical
      } finally {
        setIsLoading(false);
      }
    };

    fetchLogs();
  }, [workspaceId, serverId, deploymentId]);

  const { isConnected } = useBuildLogsWebSocket(deploymentId, {
    onLog: (log) => {
      // Pre-compute log type on receive to avoid re-computation on render
      const type = log.stream === 'stderr' ? 'error' : detectLogType(log.line);
      setRealtimeLogs((prev) => [...prev, { ...log, type }]);
      // Enable auto-scroll when real-time logs start arriving
      setAutoScroll(true);
    },
  });

  // Polling fallback: poll until WS actually delivers a message.
  // isConnected=true only means Ping/Pong worked — log messages may still be lost.
  // Stop polling only when realtimeLogs has entries (WS is genuinely delivering).
  useEffect(() => {
    if (!workspaceId || !serverId || !deploymentId) return;
    if (realtimeLogs.length > 0) return;

    const poll = async () => {
      try {
        const response = await api.get<LogsResponse>(
          `/workspaces/${workspaceId}/servers/${serverId}/deployments/${deploymentId}/logs`
        );
        if (response.logs != null) {
          setHistoricalLogs(response.logs);
          setIsLoading(false);
        }
      } catch {
        // Silently ignore
      }
    };

    const intervalId = setInterval(poll, 3000);
    return () => clearInterval(intervalId);
  }, [workspaceId, serverId, deploymentId, realtimeLogs.length]);

  // Auto-scroll to bottom — debounced to avoid excessive calls on high-frequency logs
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => {
    if (!autoScroll || !bottomRef.current) return;
    clearTimeout(scrollTimerRef.current);
    scrollTimerRef.current = setTimeout(() => {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    }, 80);
    return () => clearTimeout(scrollTimerRef.current);
  }, [realtimeLogs, autoScroll]);

  // Detect manual scroll to disable auto-scroll
  const handleScroll = () => {
    if (!containerRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = containerRef.current;
    const isAtBottom = scrollHeight - scrollTop - clientHeight < 50;
    setAutoScroll(isAtBottom);
  };


  const totalLines = parsedHistoricalLogs.length + realtimeLogs.length;
  const hasLogs = totalLines > 0;

  // Shared header component
  const renderHeader = (inFullscreen: boolean) => (
    <div className="flex items-center justify-between px-4 py-2.5 bg-[#161b22] border-b border-gray-700">
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-1.5">
          <div className="w-3 h-3 rounded-full bg-red-500" />
          <div className="w-3 h-3 rounded-full bg-yellow-500" />
          <div className="w-3 h-3 rounded-full bg-green-500" />
        </div>
        <span className="text-sm font-medium text-gray-300">{t('buildOutput')}</span>
        {isConnected && (
          <span className="flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-green-500/20 text-xs text-green-400">
            <span className="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse" />
            {t('live')}
          </span>
        )}
        {hasLogs && (
          <span className="text-xs text-gray-500">
            {t('linesCount', { count: totalLines })}
          </span>
        )}
        {inFullscreen && (
          <span className="text-xs text-gray-500 ml-2">
            {t('pressEscToExit')}
          </span>
        )}
      </div>
      <div className="flex items-center gap-3">
        {!autoScroll && (
          <button
            onClick={() => {
              setAutoScroll(true);
              const ref = inFullscreen ? fullscreenContainerRef : containerRef;
              ref.current?.scrollTo({ top: ref.current.scrollHeight, behavior: 'smooth' });
            }}
            className="flex items-center gap-1 text-xs text-gray-400 hover:text-white transition-colors"
          >
            <svg className="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M12 5v14M5 12l7 7 7-7" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
            {t('follow')}
          </button>
        )}
        {allowFullscreen && (
          <button
            onClick={toggleFullscreen}
            className="flex items-center gap-1 text-xs text-gray-400 hover:text-white transition-colors"
            title={inFullscreen ? t('exitFullscreen') : t('fullscreen')}
          >
            {inFullscreen ? (
              <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M8 3v3a2 2 0 0 1-2 2H3m18 0h-3a2 2 0 0 1-2-2V3m0 18v-3a2 2 0 0 1 2-2h3M3 16h3a2 2 0 0 1 2 2v3" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            ) : (
              <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M8 3H5a2 2 0 0 0-2 2v3m18 0V5a2 2 0 0 0-2-2h-3m0 18h3a2 2 0 0 0 2-2v-3M3 16v3a2 2 0 0 0 2 2h3" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            )}
          </button>
        )}
      </div>
    </div>
  );

  // Shared logs content component
  const renderLogsContent = () => {
    if (isLoading) {
      return (
        <div className="flex items-center justify-center py-12 text-gray-500">
          <svg className="w-5 h-5 mr-2 animate-spin" viewBox="0 0 24 24" fill="none">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
          </svg>
          Loading logs...
        </div>
      );
    }

    if (!hasLogs) {
      return (
        <div className="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg className="w-8 h-8 mb-2 opacity-50" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" strokeLinecap="round" strokeLinejoin="round" />
            <path d="M14 2v6h6M16 13H8M16 17H8M10 9H8" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          {isConnected ? 'Waiting for build output...' : 'Connecting...'}
        </div>
      );
    }

    return (
      <div className="py-2">
        {/* Historical logs */}
        {parsedHistoricalLogs.map((log) => (
          <div
            key={`hist-${log.lineNumber}`}
            className={cn(
              'flex px-4 py-0.5 hover:bg-gray-800/50',
              LOG_LINE_STYLES[log.type] ?? LOG_LINE_STYLES.default
            )}
          >
            <span className="w-12 flex-shrink-0 text-right pr-4 text-gray-600 select-none border-r border-gray-800 mr-4">
              {log.lineNumber}
            </span>
            <span className="whitespace-pre-wrap break-all flex-1">
              {log.content || ' '}
            </span>
          </div>
        ))}
        {/* Real-time logs */}
        {realtimeLogs.map((log, index) => {
          const lineNumber = parsedHistoricalLogs.length + index + 1;
          return (
            <div
              key={`rt-${log.timestamp}-${index}`}
              className={cn(
                'flex px-4 py-0.5 hover:bg-gray-800/50',
                LOG_LINE_STYLES[log.type] ?? LOG_LINE_STYLES.default
              )}
            >
              <span className="w-12 flex-shrink-0 text-right pr-4 text-gray-600 select-none border-r border-gray-800 mr-4">
                {lineNumber}
              </span>
              <span className="whitespace-pre-wrap break-all flex-1">
                {log.line || ' '}
              </span>
            </div>
          );
        })}
      </div>
    );
  };

  // Fullscreen modal
  const fullscreenModal = isFullscreen && typeof document !== 'undefined'
    ? createPortal(
        <div className="fixed inset-0 z-50 bg-black/80 backdrop-blur-sm flex items-center justify-center p-4">
          <div className="w-full h-full max-w-7xl flex flex-col border border-gray-700 rounded-lg overflow-hidden bg-[#0d1117] shadow-2xl">
            {renderHeader(true)}
            <div
              ref={fullscreenContainerRef}
              onScroll={() => {
                if (!fullscreenContainerRef.current) return;
                const { scrollTop, scrollHeight, clientHeight } = fullscreenContainerRef.current;
                const isAtBottom = scrollHeight - scrollTop - clientHeight < 50;
                setAutoScroll(isAtBottom);
              }}
              className="flex-1 overflow-auto font-mono text-[13px] leading-6 scrollbar-hide"
              style={{ scrollbarWidth: 'none', msOverflowStyle: 'none' }}
            >
              {renderLogsContent()}
              <div ref={bottomRef} />
            </div>
          </div>
        </div>,
        document.body
      )
    : null;

  return (
    <>
      <div className={cn('flex flex-col border border-gray-700 rounded-lg overflow-hidden bg-[#0d1117]', className)}>
        {renderHeader(false)}
        <div
          ref={containerRef}
          onScroll={handleScroll}
          className="overflow-auto font-mono text-[13px] leading-6 scrollbar-hide"
          style={{ maxHeight, scrollbarWidth: 'none', msOverflowStyle: 'none' }}
        >
          {renderLogsContent()}
          <div ref={bottomRef} />
        </div>
      </div>
      {fullscreenModal}
    </>
  );
}

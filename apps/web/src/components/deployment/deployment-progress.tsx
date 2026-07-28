'use client';

import { useEffect, useState } from 'react';
import { useDeploymentWebSocket, DeploymentStatusUpdate } from '@/hooks/use-websocket';
import type { DeploymentStatus } from '@/types';
import { cn } from '@/lib/utils';

interface DeploymentProgressProps {
  deploymentId: string;
  initialStatus?: DeploymentStatus;
  onStatusChange?: (status: DeploymentStatusUpdate) => void;
  className?: string;
}

const statusLabels: Record<DeploymentStatus, string> = {
  pending: 'Pending',
  building: 'Building',
  pushing: 'Pushing Image',
  deploying: 'Deploying',
  succeeded: 'Succeeded',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

const statusColors: Record<DeploymentStatus, string> = {
  pending: 'bg-gray-500',
  building: 'bg-blue-500',
  pushing: 'bg-blue-500',
  deploying: 'bg-purple-500',
  succeeded: 'bg-green-500',
  failed: 'bg-red-500',
  cancelled: 'bg-gray-500',
};

export function DeploymentProgress({
  deploymentId,
  initialStatus = 'pending',
  onStatusChange,
  className,
}: DeploymentProgressProps) {
  const [status, setStatus] = useState<DeploymentStatus>(initialStatus);
  const [progress, setProgress] = useState(0);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);

  const { isConnected } = useDeploymentWebSocket(deploymentId, {
    onStatusUpdate: (update) => {
      setStatus(update.status);
      if (update.progress !== undefined) {
        setProgress(update.progress);
      }
      if (update.error_message) {
        setErrorMessage(update.error_message);
      }
      onStatusChange?.(update);
    },
  });

  // Elapsed time counter
  useEffect(() => {
    const isComplete = status === 'succeeded' || status === 'failed' || status === 'cancelled';
    if (isComplete) return;

    const timer = setInterval(() => {
      setElapsedSeconds((prev) => prev + 1);
    }, 1000);

    return () => clearInterval(timer);
  }, [status]);

  // Set initial progress based on status
  useEffect(() => {
    switch (status) {
      case 'pending':
        setProgress(0);
        break;
      case 'building':
        setProgress(25);
        break;
      case 'pushing':
        setProgress(50);
        break;
      case 'deploying':
        setProgress(75);
        break;
      case 'succeeded':
        setProgress(100);
        break;
      case 'failed':
      case 'cancelled':
        // Keep current progress
        break;
    }
  }, [status]);

  const isComplete = status === 'succeeded' || status === 'failed' || status === 'cancelled';
  const isError = status === 'failed';

  return (
    <div className={cn('space-y-3', className)}>
      {/* Status indicator */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <div
            className={cn(
              'h-2 w-2 rounded-full',
              statusColors[status],
              !isComplete && 'animate-pulse'
            )}
          />
          <span className="text-sm font-medium">{statusLabels[status]}</span>
        </div>
        {!isComplete && (
          <span className="text-sm text-muted-foreground">
            {isConnected ? 'Live' : 'Connecting...'}
          </span>
        )}
      </div>

      {/* Progress bar */}
      <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            'h-full transition-all duration-500',
            isError ? 'bg-red-500' : 'bg-primary'
          )}
          style={{ width: `${progress}%` }}
        />
      </div>

      {/* Progress percentage and elapsed time */}
      <div className="flex justify-between text-xs text-muted-foreground">
        <span>{progress}%</span>
        <span className="flex items-center gap-2">
          <span className="font-mono">{formatElapsedTime(elapsedSeconds)}</span>
          {!isComplete && <LoadingDots />}
        </span>
      </div>

      {/* Error message */}
      {errorMessage && (
        <div className="mt-2 rounded-md bg-red-500/10 p-3 text-sm text-red-500">
          {errorMessage}
        </div>
      )}
    </div>
  );
}

function formatElapsedTime(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${mins}:${secs.toString().padStart(2, '0')}`;
}

function LoadingDots() {
  return (
    <span className="inline-flex items-center gap-0.5">
      <span className="h-1 w-1 animate-bounce rounded-full bg-current" style={{ animationDelay: '0ms' }} />
      <span className="h-1 w-1 animate-bounce rounded-full bg-current" style={{ animationDelay: '150ms' }} />
      <span className="h-1 w-1 animate-bounce rounded-full bg-current" style={{ animationDelay: '300ms' }} />
    </span>
  );
}

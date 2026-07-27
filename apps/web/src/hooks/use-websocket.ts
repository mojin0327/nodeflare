import { useCallback, useEffect, useRef, useState } from 'react';
import type { ServerStatus } from '@/types';

// Constants
const DEFAULT_RECONNECT_INTERVAL = 3000;
const DEFAULT_MAX_RECONNECT_ATTEMPTS = 10;

type WebSocketStatus = 'connecting' | 'connected' | 'disconnected' | 'error';

interface UseWebSocketOptions {
  url: string;
  enabled?: boolean;
  onMessage?: (message: WebSocketMessage) => void;
  onConnect?: () => void;
  onDisconnect?: () => void;
  onError?: (error: Event) => void;
  reconnect?: boolean;
  reconnectInterval?: number;
  maxReconnectAttempts?: number;
}

// WebSocket message types matching backend
export type WsMessageType =
  | 'DeploymentStatus'
  | 'ServerStatus'
  | 'BuildLog'
  | 'ServerLog'
  | 'Error'
  | 'Ping'
  | 'Pong'
  | 'Auth'
  | 'AuthSuccess'
  | 'AuthError';

export interface DeploymentStatusUpdate {
  deployment_id: string;
  server_id: string;
  status: DeploymentStatus;
  error_message?: string;
  progress?: number;
  timestamp: string;
}

export interface ServerStatusUpdate {
  server_id: string;
  status: ServerStatus;
  endpoint_url?: string;
  error_message?: string;
  timestamp: string;
}

export interface BuildLogLine {
  deployment_id: string;
  line: string;
  stream: 'stdout' | 'stderr';
  timestamp: string;
}

export interface ServerLogLine {
  server_id: string;
  line: string;
  level: 'debug' | 'info' | 'warn' | 'error';
  timestamp: string;
}

export type DeploymentStatus =
  | 'pending'
  | 'building'
  | 'pushing'
  | 'deploying'
  | 'succeeded'
  | 'failed'
  | 'cancelled';

export type WebSocketMessage =
  | { type: 'DeploymentStatus'; data: DeploymentStatusUpdate }
  | { type: 'ServerStatus'; data: ServerStatusUpdate }
  | { type: 'BuildLog'; data: BuildLogLine }
  | { type: 'ServerLog'; data: ServerLogLine }
  | { type: 'Error'; data: { code: string; message: string } }
  | { type: 'Ping' }
  | { type: 'Pong' }
  | { type: 'AuthSuccess' }
  | { type: 'AuthError'; data: { message: string } };

export function useWebSocket({
  url,
  enabled = true,
  onMessage,
  onConnect,
  onDisconnect,
  onError,
  reconnect = true,
  reconnectInterval = DEFAULT_RECONNECT_INTERVAL,
  maxReconnectAttempts = DEFAULT_MAX_RECONNECT_ATTEMPTS,
}: UseWebSocketOptions) {
  const [status, setStatus] = useState<WebSocketStatus>('disconnected');
  const [lastMessage, setLastMessage] = useState<WebSocketMessage | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectAttemptsRef = useRef(0);
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const pingIntervalRef = useRef<NodeJS.Timeout | null>(null);
  const isMountedRef = useRef(true);
  // Prevent onConnect from firing on every periodic Pong — only fire on first connect per session
  const hasCalledOnConnectRef = useRef(false);

  // Store callbacks in refs to avoid dependency issues
  const onMessageRef = useRef(onMessage);
  const onConnectRef = useRef(onConnect);
  const onDisconnectRef = useRef(onDisconnect);
  const onErrorRef = useRef(onError);

  // Update refs when callbacks change
  useEffect(() => {
    onMessageRef.current = onMessage;
    onConnectRef.current = onConnect;
    onDisconnectRef.current = onDisconnect;
    onErrorRef.current = onError;
  }, [onMessage, onConnect, onDisconnect, onError]);

  // Send periodic Ping to keep the connection alive through proxy idle timeouts
  useEffect(() => {
    if (status !== 'connected') {
      if (pingIntervalRef.current) {
        clearInterval(pingIntervalRef.current);
        pingIntervalRef.current = null;
      }
      return;
    }

    pingIntervalRef.current = setInterval(() => {
      if (wsRef.current?.readyState === WebSocket.OPEN) {
        wsRef.current.send(JSON.stringify({ type: 'Ping' }));
      }
    }, 30_000);

    return () => {
      if (pingIntervalRef.current) {
        clearInterval(pingIntervalRef.current);
        pingIntervalRef.current = null;
      }
    };
  }, [status]);

  const connect = useCallback(() => {
    if (!isMountedRef.current) return;
    if (!enabled || !url) return;
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      return;
    }

    setStatus('connecting');
    const ws = new WebSocket(url);

    ws.onopen = () => {
      if (!isMountedRef.current) {
        ws.close();
        return;
      }
      // Server authenticates via cookie from upgrade request
      // Send a ping to confirm connection
      ws.send(JSON.stringify({ type: 'Ping' }));
    };

    ws.onmessage = (event) => {
      if (!isMountedRef.current) return;
      try {
        const message = JSON.parse(event.data) as WebSocketMessage;

        // Handle ping/pong — also used for periodic keep-alive
        if (message.type === 'Pong') {
          setStatus('connected');
          reconnectAttemptsRef.current = 0;
          // Only fire onConnect once per WS session; periodic Pongs must not re-fire it
          if (!hasCalledOnConnectRef.current) {
            hasCalledOnConnectRef.current = true;
            onConnectRef.current?.();
          }
          return;
        }

        // Handle auth error (server could not authenticate via cookie)
        if (message.type === 'AuthError') {
          setStatus('error');
          ws.close();
          return;
        }

        setLastMessage(message);
        onMessageRef.current?.(message);
      } catch {
        // Silently ignore parse errors
      }
    };

    ws.onerror = (error) => {
      if (!isMountedRef.current) return;
      setStatus('error');
      onErrorRef.current?.(error);
    };

    ws.onclose = () => {
      if (!isMountedRef.current) return;
      hasCalledOnConnectRef.current = false;
      setStatus('disconnected');
      wsRef.current = null;
      onDisconnectRef.current?.();

      // Attempt to reconnect
      if (reconnect && reconnectAttemptsRef.current < maxReconnectAttempts && isMountedRef.current) {
        reconnectAttemptsRef.current++;
        reconnectTimeoutRef.current = setTimeout(() => {
          if (isMountedRef.current) {
            connect();
          }
        }, reconnectInterval);
      }
    };

    wsRef.current = ws;
  }, [url, enabled, reconnect, reconnectInterval, maxReconnectAttempts]);

  const disconnect = useCallback(() => {
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current);
      reconnectTimeoutRef.current = null;
    }
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
    setStatus('disconnected');
  }, []);

  const sendMessage = useCallback((message: object) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(message));
    }
  }, []);

  useEffect(() => {
    isMountedRef.current = true;
    connect();

    return () => {
      isMountedRef.current = false;
      if (pingIntervalRef.current) {
        clearInterval(pingIntervalRef.current);
        pingIntervalRef.current = null;
      }
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
        reconnectTimeoutRef.current = null;
      }
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, [connect]);

  return {
    status,
    lastMessage,
    connect,
    disconnect,
    sendMessage,
    isConnected: status === 'connected',
  };
}

// Helper to get WebSocket URL with authentication token
// Uses NEXT_PUBLIC_WS_URL for direct WebSocket connections to API
function getWsBaseUrl(): string {
  const wsUrl = process.env.NEXT_PUBLIC_WS_URL || process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8080';
  return wsUrl.replace(/^http/, 'ws');
}

// Fetch WebSocket token from API (called via Next.js proxy to handle cookies)
async function fetchWsToken(): Promise<string | null> {
  try {
    const response = await fetch('/api/v1/auth/ws-token', {
      credentials: 'include',
    });
    if (!response.ok) {
      return null;
    }
    const data = await response.json();
    return data.token;
  } catch {
    return null;
  }
}

// Build WebSocket URL with token
function buildWsUrl(path: string, token: string): string {
  const baseUrl = getWsBaseUrl();
  return `${baseUrl}${path}?token=${encodeURIComponent(token)}`;
}

// Convenience hooks for specific WebSocket connections

export function useDeploymentWebSocket(deploymentId: string, options?: {
  onStatusUpdate?: (status: DeploymentStatusUpdate) => void;
  onBuildLog?: (log: BuildLogLine) => void;
}) {
  const [wsUrl, setWsUrl] = useState<string>('');
  const isEnabled = !!deploymentId;

  useEffect(() => {
    if (!isEnabled) {
      setWsUrl('');
      return;
    }
    fetchWsToken().then((token) => {
      if (token) {
        setWsUrl(buildWsUrl(`/ws/deployments/${deploymentId}`, token));
      }
    });
  }, [deploymentId, isEnabled]);

  return useWebSocket({
    url: wsUrl,
    enabled: isEnabled && !!wsUrl,
    onMessage: (message) => {
      if (message.type === 'DeploymentStatus') {
        options?.onStatusUpdate?.(message.data);
      }
    },
  });
}

export function useBuildLogsWebSocket(deploymentId: string, options?: {
  onLog?: (log: BuildLogLine) => void;
}) {
  const [wsUrl, setWsUrl] = useState<string>('');
  const isEnabled = !!deploymentId;

  useEffect(() => {
    if (!isEnabled) {
      setWsUrl('');
      return;
    }
    fetchWsToken().then((token) => {
      if (token) {
        setWsUrl(buildWsUrl(`/ws/deployments/${deploymentId}/logs`, token));
      }
    });
  }, [deploymentId, isEnabled]);

  return useWebSocket({
    url: wsUrl,
    enabled: isEnabled && !!wsUrl,
    onMessage: (message) => {
      if (message.type === 'BuildLog') {
        options?.onLog?.(message.data);
      }
    },
  });
}

export function useServerLogsWebSocket(workspaceId: string, serverId: string, options?: {
  onLog?: (log: ServerLogLine) => void;
}) {
  const [wsUrl, setWsUrl] = useState<string>('');
  const isEnabled = !!(workspaceId && serverId);

  useEffect(() => {
    if (!isEnabled) {
      setWsUrl('');
      return;
    }
    fetchWsToken().then((token) => {
      if (token) {
        setWsUrl(buildWsUrl(`/ws/workspaces/${workspaceId}/servers/${serverId}/logs`, token));
      }
    });
  }, [workspaceId, serverId, isEnabled]);

  return useWebSocket({
    url: wsUrl,
    enabled: isEnabled && !!wsUrl,
    onMessage: (message) => {
      if (message.type === 'ServerLog') {
        options?.onLog?.(message.data);
      }
    },
  });
}

export function useServerStatusWebSocket(workspaceId: string, serverId: string, options?: {
  onStatusUpdate?: (status: ServerStatusUpdate) => void;
}) {
  const [wsUrl, setWsUrl] = useState<string>('');
  const isEnabled = !!(workspaceId && serverId);

  useEffect(() => {
    if (!isEnabled) {
      setWsUrl('');
      return;
    }
    fetchWsToken().then((token) => {
      if (token) {
        setWsUrl(buildWsUrl(`/ws/workspaces/${workspaceId}/servers/${serverId}/status`, token));
      }
    });
  }, [workspaceId, serverId, isEnabled]);

  return useWebSocket({
    url: wsUrl,
    enabled: isEnabled && !!wsUrl,
    onMessage: (message) => {
      if (message.type === 'ServerStatus') {
        options?.onStatusUpdate?.(message.data);
      }
    },
  });
}

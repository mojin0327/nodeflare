const API_BASE = process.env.NEXT_PUBLIC_API_URL
  ? `${process.env.NEXT_PUBLIC_API_URL}/api/v1`
  : '/api/v1';

// Scalability: Maximum number of refresh state listeners to prevent memory leaks
const MAX_REFRESH_LISTENERS = 100;

class ApiClient {
  private _isRefreshing = false;
  private refreshPromise: Promise<boolean> | null = null;
  private refreshStateListeners = new Set<() => void>();

  // リフレッシュ中かどうかを公開（認証状態の判定に使用）
  get isRefreshing(): boolean {
    return this._isRefreshing;
  }

  // Scalability: Get current listener count for debugging/monitoring
  get listenerCount(): number {
    return this.refreshStateListeners.size;
  }

  // リフレッシュ状態の変更を購読（イベントベース）
  // Scalability: Returns unsubscribe function, callers MUST call it when component unmounts
  subscribeToRefreshState(callback: () => void): () => void {
    // Prevent unbounded listener growth
    if (this.refreshStateListeners.size >= MAX_REFRESH_LISTENERS) {
      // Remove oldest listener to make room (simple FIFO eviction)
      const iterator = this.refreshStateListeners.values();
      const oldest = iterator.next().value;
      if (oldest) {
        this.refreshStateListeners.delete(oldest);
      }
    }

    this.refreshStateListeners.add(callback);

    // Return unsubscribe function
    return () => {
      this.refreshStateListeners.delete(callback);
    };
  }

  // Clear all listeners (useful for testing or full cleanup)
  clearAllListeners(): void {
    this.refreshStateListeners.clear();
  }

  private notifyRefreshStateChange() {
    this.refreshStateListeners.forEach((listener) => {
      try {
        listener();
      } catch {
        // Silently ignore listener errors to prevent breaking other listeners
      }
    });
  }

  private async refreshToken(): Promise<boolean> {
    // Prevent multiple concurrent refresh attempts
    if (this._isRefreshing && this.refreshPromise) {
      return this.refreshPromise;
    }

    this._isRefreshing = true;
    this.notifyRefreshStateChange();

    this.refreshPromise = (async () => {
      try {
        const response = await fetch(`${API_BASE}/auth/refresh`, {
          method: 'POST',
          credentials: 'include',
          headers: { 'Content-Type': 'application/json' },
        });
        return response.ok;
      } catch {
        return false;
      } finally {
        this._isRefreshing = false;
        this.refreshPromise = null;
        this.notifyRefreshStateChange();
      }
    })();

    return this.refreshPromise;
  }

  private async request<T>(
    path: string,
    options: RequestInit = {},
    isRetry = false
  ): Promise<T> {
    // Authentication is handled via HTTP-only cookies set by the server.
    // The credentials: 'include' option ensures cookies are sent with requests.
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...(options.headers as Record<string, string>),
    };

    const response = await fetch(`${API_BASE}${path}`, {
      ...options,
      headers,
      credentials: 'include',
    });

    // Handle 401 Unauthorized - try to refresh token
    if (response.status === 401 && !isRetry && path !== '/auth/refresh' && path !== '/auth/logout') {
      const refreshed = await this.refreshToken();
      if (refreshed) {
        // Retry the original request with new token
        return this.request<T>(path, options, true);
      }
    }

    if (!response.ok) {
      const contentType = response.headers.get('content-type') ?? '';
      let message = 'An error occurred';
      let code: string | undefined;
      let details: Record<string, unknown> | undefined;
      if (contentType.includes('application/json')) {
        const errorBody = await response.json().catch(() => ({}));
        message = errorBody.error?.message || errorBody.message || message;
        code = errorBody.error?.code || errorBody.code;
        details = errorBody.error?.details;
      } else {
        const text = await response.text().catch(() => '');
        if (text) message = text;
      }
      throw new ApiError(message, response.status, code, details);
    }

    // Handle 204 No Content
    if (response.status === 204) {
      return {} as T;
    }

    return response.json();
  }

  async get<T>(path: string): Promise<T> {
    return this.request<T>(path, { method: 'GET' });
  }

  async post<T>(path: string, data?: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'POST',
      body: data ? JSON.stringify(data) : undefined,
    });
  }

  async put<T>(path: string, data?: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'PUT',
      body: data ? JSON.stringify(data) : undefined,
    });
  }

  async patch<T>(path: string, data?: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'PATCH',
      body: data ? JSON.stringify(data) : undefined,
    });
  }

  async delete<T>(path: string): Promise<T> {
    return this.request<T>(path, { method: 'DELETE' });
  }
}

export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
    public code?: string,
    public details?: Record<string, unknown>
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

export const api = new ApiClient();

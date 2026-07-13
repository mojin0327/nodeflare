export interface User {
  id: string;
  name: string;
  email: string;
  avatar_url: string | null;
  created_at: string;
}

export type Plan = 'free' | 'pro' | 'team' | 'enterprise';

export interface Workspace {
  id: string;
  name: string;
  slug: string;
  plan: Plan;
  role: WorkspaceRole;
  created_at: string;
}

export type Runtime = 'node' | 'python' | 'go' | 'rust' | 'docker';
export type Visibility = 'public' | 'private' | 'team';
export type AccessMode = 'public' | 'vpn_only';
export type Transport = 'sse' | 'stdio';
export type ServerStatus = 'inactive' | 'building' | 'deploying' | 'running' | 'stopped' | 'failed';
export type DeploymentStatus = 'pending' | 'building' | 'pushing' | 'deploying' | 'succeeded' | 'failed' | 'cancelled';
export type WorkspaceRole = 'owner' | 'admin' | 'member' | 'viewer';

export interface McpServer {
  id: string;
  workspace_id: string;
  name: string;
  slug: string;
  description: string | null;
  github_repo: string;
  github_branch: string;
  runtime: Runtime;
  visibility: Visibility;
  access_mode: AccessMode;
  transport: Transport;
  status: ServerStatus;
  endpoint_url: string | null;
  region: string;
  root_directory: string;
  mcp_path: string;
  /** Custom entry command for the MCP server (e.g., "python server.py", "uv run mcp-server") */
  entry_command: string | null;
  /** Custom build command run at image-build time (e.g., "npm run build", "npm run compile") */
  build_command: string | null;
  /** When false, NodeFlare authentication is disabled (for servers with their own auth) */
  auth_enabled: boolean;
  /** User-selected machine memory in MB (256/512/1024/2048). null = auto. */
  memory_mb: number | null;
  /** Internal listening port for Streamable HTTP (SSE) servers. null = runtime default. */
  port: number | null;
  /** When true, the proxy filters tools/list down to tools the credential may call. */
  tool_list_filter_by_scope: boolean;
  /** When true, the proxy trims verbose tool schemas in tools/list to save tokens. */
  tool_schema_slim: boolean;
  /** When true, the proxy collapses tools/list into search_tools + call_tool meta-tools. */
  tool_search_mode: boolean;
  /** When true, the proxy exposes run_code and executes AI-written code in a sandbox. */
  tool_code_mode: boolean;
  /** Upstream OAuth provider name (matches upstream_oauth_providers catalog, null = disabled) */
  upstream_oauth_provider: string | null;
  /** OAuth scopes to request from the upstream provider */
  upstream_oauth_scopes: string[] | null;
  /** Custom provider only: authorization endpoint URL */
  upstream_oauth_authorization_url: string | null;
  /** Custom provider only: token endpoint URL */
  upstream_oauth_token_url: string | null;
  /** Custom provider only: client_id (client_secret is write-only) */
  upstream_oauth_client_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface UpstreamOAuthProvider {
  name: string;
  display_name: string;
  authorization_url: string;
  token_url: string;
  default_scopes: string[];
  is_managed: boolean;
}

/** Minimal server info for selection lists (only id, workspace_id, name) */
export interface McpServerMinimal {
  id: string;
  workspace_id: string;
  name: string;
}

/** Basic server info for dashboard/overview (includes status and runtime) */
export interface McpServerBasic {
  id: string;
  workspace_id: string;
  name: string;
  status: ServerStatus;
  runtime: Runtime;
}

/** Server list info for server list page (display-relevant fields only) */
export interface McpServerList {
  id: string;
  workspace_id: string;
  name: string;
  slug: string;
  runtime: Runtime;
  visibility: Visibility;
  status: ServerStatus;
  github_repo: string;
  endpoint_url: string | null;
}

export interface Deployment {
  id: string;
  server_id: string;
  version: number;
  commit_sha: string;
  status: DeploymentStatus;
  error_message: string | null;
  build_logs: string | null;
  started_at: string;
  finished_at: string | null;
  created_at: string;
  deployed_at: string | null;
  build_duration_seconds: number | null;
}

export type ToolPermissionLevel = 'normal' | 'elevated' | 'dangerous';

export interface Tool {
  id: string;
  server_id: string;
  name: string;
  description: string | null;
  input_schema: Record<string, unknown> | null;
  enabled: boolean;
  permission_level: ToolPermissionLevel;
  rate_limit_per_minute: number | null;
}

export interface AccessToken {
  id: string;
  name: string;
  key_prefix: string;
  scopes: string[];
  server_id?: string;
  last_used_at: string | null;
  expires_at: string | null;
  created_at: string;
}

export interface Secret {
  key: string;
  created_at: string;
  updated_at: string;
}

export interface RequestLog {
  id: string;
  server_id: string;
  tool_name: string | null;
  response_status: string;
  duration_ms: number;
  created_at: string;
}

// API Response types
export interface PaginatedResponse<T> {
  data: T[];
  total: number;
  page: number;
  per_page: number;
}

export interface CreateServerRequest {
  name: string;
  slug: string;
  description?: string;
  github_repo: string;
  github_branch?: string;
  github_installation_id?: number;
  runtime?: Runtime;
  visibility?: Visibility;
  access_mode?: AccessMode;
  transport?: Transport;
  region?: string;
  root_directory?: string;
  mcp_path?: string;
  /** Custom entry command for the MCP server (e.g., "python server.py", "uv run mcp-server") */
  entry_command?: string;
  /** Custom build command run at image-build time (e.g., "npm run build", "npm run compile") */
  build_command?: string;
  /** When false, skip NodeFlare authentication layer (for servers that handle their own auth) */
  auth_enabled?: boolean;
  /** User-selected machine memory in MB (256/512/1024/2048). Omit for auto. */
  memory_mb?: number;
  /** Internal listening port for Streamable HTTP (SSE) servers. Omit for runtime default. */
  port?: number;
  /** Environment variables (secrets) to provision before the initial deploy. */
  env_vars?: { key: string; value: string }[];
  /** Upstream OAuth provider for this server ('google', 'github', 'custom', undefined = none) */
  upstream_oauth_provider?: string;
  /** OAuth scopes to request from the upstream provider */
  upstream_oauth_scopes?: string[];
  /** Custom provider: authorization endpoint URL */
  upstream_oauth_authorization_url?: string;
  /** Custom provider: token endpoint URL */
  upstream_oauth_token_url?: string;
  /** Custom provider: client ID */
  upstream_oauth_client_id?: string;
  /** Custom provider: client secret (write-only) */
  upstream_oauth_client_secret?: string;
}

export interface UpdateServerRequest {
  name?: string;
  description?: string;
  github_branch?: string;
  visibility?: Visibility;
  access_mode?: AccessMode;
  region?: string;
  root_directory?: string;
  mcp_path?: string;
  /** Custom entry command for the MCP server (e.g., "python server.py", "uv run mcp-server") */
  entry_command?: string;
  /** Custom build command run at image-build time (e.g., "npm run build", "npm run compile") */
  build_command?: string;
  /** When false, skip NodeFlare authentication layer (for servers that handle their own auth) */
  auth_enabled?: boolean;
  /** User-selected machine memory in MB (256/512/1024/2048). */
  memory_mb?: number;
  /** Internal listening port for Streamable HTTP (SSE) servers. Omit to leave unchanged. */
  port?: number;
  /** Upstream OAuth provider for this server ('google', 'github', 'custom', null = remove) */
  upstream_oauth_provider?: string | null;
  /** OAuth scopes to request from the upstream provider */
  upstream_oauth_scopes?: string[] | null;
  /** Custom provider: authorization endpoint URL */
  upstream_oauth_authorization_url?: string | null;
  /** Custom provider: token endpoint URL */
  upstream_oauth_token_url?: string | null;
  /** Custom provider: client ID */
  upstream_oauth_client_id?: string | null;
  /** Custom provider: client secret (write-only) */
  upstream_oauth_client_secret?: string;
}

export interface CreateAccessTokenRequest {
  name: string;
  server_id?: string;
  scopes?: string[];
  expires_in_days?: number;
}

export interface CreateAccessTokenResponse {
  id: string;
  name: string;
  key: string;
  key_prefix: string;
  created_at: string;
}

export interface CreateSecretRequest {
  key: string;
  value: string;
}

export interface GitHubRepo {
  id: number;
  name: string;
  full_name: string;
  description: string | null;
  private: boolean;
  html_url: string;
  default_branch: string;
  updated_at: string;
  language: string | null;
}

export interface RequestLogStats {
  total_requests: number;
  success_count: number;
  error_count: number;
  avg_duration_ms: number;
}

export interface ToolUsageStats {
  tool_name: string;
  call_count: number;
  error_count: number;
  avg_duration_ms: number;
}

export interface ServerStatsResponse {
  stats: RequestLogStats;
  tool_usage: ToolUsageStats[];
}

// Team member types
export interface TeamMember {
  user_id: string;
  email: string;
  name: string;
  avatar_url: string | null;
  role: WorkspaceRole;
}

export interface AddMemberRequest {
  email: string;
  role?: WorkspaceRole;
}

export interface UpdateMemberRequest {
  role: WorkspaceRole;
}

// API Error types
export interface ApiErrorResponse {
  error?: {
    code?: string;
    message?: string;
  };
}

export interface ApiError {
  response?: {
    data?: ApiErrorResponse;
  };
  message?: string;
}

// Type guard for API errors
export function isApiError(error: unknown): error is ApiError {
  return (
    typeof error === 'object' &&
    error !== null &&
    ('response' in error || 'message' in error)
  );
}

// Helper to extract error code from API error
export function getApiErrorCode(error: unknown): string | undefined {
  // Check for ApiError from api.ts (has code directly on error)
  if (error && typeof error === 'object' && 'code' in error) {
    return (error as { code?: string }).code;
  }
  // Check for legacy/axios-style errors
  if (isApiError(error)) {
    return error.response?.data?.error?.code;
  }
  return undefined;
}

// Helper to extract error message from API error
export function getApiErrorMessage(error: unknown): string {
  // Check for ApiError from api.ts (has message directly on error)
  if (error instanceof Error) {
    return error.message;
  }
  // Check for legacy/axios-style errors
  if (isApiError(error)) {
    return error.response?.data?.error?.message || error.message || 'An error occurred';
  }
  return 'An error occurred';
}

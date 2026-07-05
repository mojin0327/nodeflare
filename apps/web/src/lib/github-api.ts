import { api } from './api';
import { GitHubRepo } from '@/types';

export interface LinkedGitHubAccount {
  id: string;
  github_id: number;
  github_username: string;
  github_avatar_url: string | null;
  is_primary: boolean;
  created_at: string;
}

export interface MessageResponse {
  message: string;
}

/**
 * Get all linked GitHub accounts for the current user
 */
export const getLinkedAccounts = (): Promise<LinkedGitHubAccount[]> => {
  return api.get('/github/accounts');
};

/**
 * Unlink a GitHub account
 */
export const unlinkAccount = (accountId: string): Promise<MessageResponse> => {
  return api.delete(`/github/accounts/${accountId}`);
};

/**
 * Set a linked account as primary
 */
export const setPrimaryAccount = (accountId: string): Promise<MessageResponse> => {
  return api.post(`/github/accounts/${accountId}/primary`);
};

/**
 * Get repositories from a linked GitHub account
 * @param accountId - Optional account ID. If not provided, uses primary or any linked account
 */
export const getRepos = (accountId?: string): Promise<GitHubRepo[]> => {
  const path = accountId ? `/github/repos?account_id=${accountId}` : '/github/repos';
  return api.get(path);
};

/**
 * List a repository's branch names. Called lazily (when the branch dropdown opens) so it
 * never blocks the create form's initial render or auto-detection. `repo` is `owner/repo`.
 */
export const getBranches = (repo: string, accountId?: string): Promise<string[]> => {
  const params = new URLSearchParams({ repo });
  if (accountId) params.set('account_id', accountId);
  return api.get(`/github/branches?${params.toString()}`);
};

export interface DetectedEnvVar {
  key: string;
  description?: string | null;
  required: boolean;
  secret: boolean;
}

/** Auto-detected deploy configuration for a repo (see the `/servers/inspect` endpoint). */
export interface RepoDetection {
  branch?: string;
  runtime?: 'node' | 'python' | 'go' | 'rust' | 'docker';
  transport?: 'stdio' | 'sse';
  root_directory?: string;
  build_command?: string;
  entry_command?: string;
  mcp_path?: string;
  port?: number;
  env_vars: DetectedEnvVar[];
  is_monorepo_member: boolean;
  signals: { field: string; source: string }[];
  warnings: string[];
}

/**
 * Inspect a GitHub repo and auto-detect the server-create form values. The same
 * detection algorithm the builder runs, so what's shown is what will build.
 */
export const inspectRepo = (body: {
  github_repo: string;
  github_branch?: string;
  account_id?: string;
  root_directory?: string;
}): Promise<RepoDetection> => {
  return api.post('/servers/inspect', body);
};

/**
 * Get the link URL to initiate GitHub OAuth for account linking
 * @param returnTo - Optional path to redirect to after linking
 */
export const getLinkUrl = (returnTo?: string): string => {
  const apiBase = process.env.NEXT_PUBLIC_API_URL || '';
  const params = returnTo ? `?return_to=${encodeURIComponent(returnTo)}` : '';
  return `${apiBase}/api/v1/github/accounts/link${params}`;
};

//! Redis-based caching layer for API key validation and server resolution
//!
//! This module provides high-performance caching to reduce database load
//! on the hot path of every proxy request.

use fred::prelude::*;
use mcp_db::{ApiKey, McpServer, Workspace};
use serde::{Deserialize, Serialize};

/// Cache TTL for API keys (5 minutes)
const API_KEY_CACHE_TTL_SECS: i64 = 300;
/// Cache TTL for server metadata (30 seconds)
const SERVER_CACHE_TTL_SECS: i64 = 30;
/// Cache TTL for workspace plan/subscription used on the quota hot path (short so
/// plan/subscription changes converge quickly without an explicit invalidate).
const WORKSPACE_CACHE_TTL_SECS: i64 = 30;
/// Cache TTL for tools/list (24 hours). A container's tool surface is fixed until
/// the image is rebuilt; the API invalidates this key on redeploy.
const TOOLS_LIST_CACHE_TTL_SECS: i64 = 86400;

/// Cached API key data (subset of ApiKey for caching)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedApiKey {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub server_id: Option<uuid::Uuid>,
    pub name: String,
    pub scopes: serde_json::Value,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<&ApiKey> for CachedApiKey {
    fn from(key: &ApiKey) -> Self {
        Self {
            id: key.id,
            workspace_id: key.workspace_id,
            server_id: key.server_id,
            name: key.name.clone(),
            scopes: key.scopes.clone(),
            expires_at: key.expires_at,
        }
    }
}

impl CachedApiKey {
    /// Convert back to ApiKey (fills in non-cached fields with defaults)
    pub fn to_api_key(&self) -> ApiKey {
        ApiKey {
            id: self.id,
            workspace_id: self.workspace_id,
            server_id: self.server_id,
            name: self.name.clone(),
            key_prefix: String::new(), // Not needed for validation
            key_hash: String::new(),   // Not needed for validation
            scopes: self.scopes.clone(),
            expires_at: self.expires_at,
            last_used_at: None,
            created_at: chrono::Utc::now(),
        }
    }
}

/// Cached server data (subset of McpServer for caching)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedServer {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub name: String,
    pub slug: String,
    pub container_name: String,
    pub endpoint_url: Option<String>,
    pub visibility: String,
    pub status: String,
    pub rate_limit_per_minute: Option<i32>,
    pub mcp_path: String,
    /// When false, skip NodeFlare authentication layer
    #[serde(default = "default_auth_enabled")]
    pub auth_enabled: bool,
    /// When true, filter `tools/list` to tools the credential may call.
    #[serde(default = "default_auth_enabled")]
    pub tool_list_filter_by_scope: bool,
    /// When true, trim verbose tool schemas in `tools/list`.
    #[serde(default)]
    pub tool_schema_slim: bool,
    /// When true, collapse `tools/list` into search_tools + call_tool meta-tools.
    #[serde(default)]
    pub tool_search_mode: bool,
    /// When true, expose run_code and execute AI-written code in a sandbox.
    #[serde(default)]
    pub tool_code_mode: bool,
}

fn default_auth_enabled() -> bool {
    true
}

impl From<&McpServer> for CachedServer {
    fn from(server: &McpServer) -> Self {
        Self {
            id: server.id,
            workspace_id: server.workspace_id,
            name: server.name.clone(),
            slug: server.slug.clone(),
            container_name: server.container_name.clone(),
            endpoint_url: server.endpoint_url.clone(),
            visibility: server.visibility.clone(),
            status: server.status.clone(),
            rate_limit_per_minute: server.rate_limit_per_minute,
            mcp_path: server.mcp_path.clone(),
            auth_enabled: server.auth_enabled,
            tool_list_filter_by_scope: server.tool_list_filter_by_scope,
            tool_schema_slim: server.tool_schema_slim,
            tool_search_mode: server.tool_search_mode,
            tool_code_mode: server.tool_code_mode,
        }
    }
}

impl CachedServer {
    /// True when the server is in a state that can actually serve proxied traffic.
    /// Scale-to-zero servers keep DB status `running` (Fly wakes the stopped Machine),
    /// so only `running` is serveable; building/deploying/stopped/failed are not.
    pub fn is_serveable(&self) -> bool {
        self.status == "running"
    }
}

/// Cached workspace plan/subscription data used by the quota hot path.
/// Avoids a per-request Postgres read in `check_monthly_quota`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedWorkspace {
    pub id: uuid::Uuid,
    pub plan: String,
    pub subscription_status: Option<String>,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<&Workspace> for CachedWorkspace {
    fn from(ws: &Workspace) -> Self {
        Self {
            id: ws.id,
            plan: ws.plan.clone(),
            subscription_status: ws.subscription_status.clone(),
            current_period_end: ws.current_period_end,
        }
    }
}

/// Authorization context for a single code-execution run, keyed by an opaque token in
/// Redis. The sandbox can present this token to the tool-call endpoint; the endpoint
/// (not the sandbox wrapper) enforces what it grants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeExecContext {
    pub server_id: uuid::Uuid,
    /// Upstream tools/call URL to forward to.
    pub target_url: String,
    /// Scope strings of the issuing credential (rebuilt into a ScopeChecker per call).
    pub scopes: Vec<String>,
}

/// Redis cache for proxy hot path data
#[derive(Clone)]
pub struct RedisCache {
    client: RedisClient,
}

impl RedisCache {
    pub fn new(client: RedisClient) -> Self {
        Self { client }
    }

    /// Cache key for API key lookup by hash
    fn api_key_cache_key(key_hash: &str) -> String {
        format!("proxy:apikey:{}", key_hash)
    }

    /// Cache key for server lookup by slug
    fn server_cache_key(slug: &str) -> String {
        format!("proxy:server:{}", slug)
    }

    /// Get cached API key by hash
    pub async fn get_api_key(&self, key_hash: &str) -> Option<CachedApiKey> {
        let cache_key = Self::api_key_cache_key(key_hash);

        let result: Option<String> = self.client.get(&cache_key).await.ok()?;

        result.and_then(|json| serde_json::from_str(&json).ok())
    }

    /// Cache an API key
    pub async fn set_api_key(&self, key_hash: &str, api_key: &ApiKey) {
        let cache_key = Self::api_key_cache_key(key_hash);
        let cached = CachedApiKey::from(api_key);

        if let Ok(json) = serde_json::to_string(&cached) {
            let _: Result<(), _> = self.client
                .set(
                    &cache_key,
                    json,
                    Some(Expiration::EX(API_KEY_CACHE_TTL_SECS)),
                    None,
                    false,
                )
                .await;
        }
    }

    /// Invalidate cached API key
    pub async fn invalidate_api_key(&self, key_hash: &str) {
        let cache_key = Self::api_key_cache_key(key_hash);
        let _: Result<(), _> = self.client.del(&cache_key).await;
    }

    /// Get cached server by slug
    pub async fn get_server(&self, slug: &str) -> Option<CachedServer> {
        let cache_key = Self::server_cache_key(slug);

        let result: Option<String> = self.client.get(&cache_key).await.ok()?;

        result.and_then(|json| serde_json::from_str(&json).ok())
    }

    /// Cache a server
    pub async fn set_server(&self, slug: &str, server: &McpServer) {
        let cache_key = Self::server_cache_key(slug);
        let cached = CachedServer::from(server);

        if let Ok(json) = serde_json::to_string(&cached) {
            let _: Result<(), _> = self.client
                .set(
                    &cache_key,
                    json,
                    Some(Expiration::EX(SERVER_CACHE_TTL_SECS)),
                    None,
                    false,
                )
                .await;
        }
    }

    /// Invalidate cached server
    pub async fn invalidate_server(&self, slug: &str) {
        let cache_key = Self::server_cache_key(slug);
        let _: Result<(), _> = self.client.del(&cache_key).await;
    }

    /// Cache key for a code-execution token.
    fn code_exec_key(token: &str) -> String {
        format!("proxy:codeexec:{}", token)
    }

    /// Store a code-exec token context with a short TTL. The sandbox presents the token
    /// to the tool-call endpoint, which looks the context up to enforce scope.
    pub async fn set_code_exec(&self, token: &str, ctx: &CodeExecContext, ttl_secs: i64) {
        let cache_key = Self::code_exec_key(token);
        if let Ok(json) = serde_json::to_string(ctx) {
            let _: Result<(), _> = self
                .client
                .set(&cache_key, json, Some(Expiration::EX(ttl_secs)), None, false)
                .await;
        }
    }

    /// Look up a code-exec token context.
    pub async fn get_code_exec(&self, token: &str) -> Option<CodeExecContext> {
        let cache_key = Self::code_exec_key(token);
        let result: Option<String> = self.client.get(&cache_key).await.ok()?;
        result.and_then(|json| serde_json::from_str(&json).ok())
    }

    /// Atomically fetch and delete a code-exec token (GETDEL).
    /// Use this in the tools-call endpoint so each token is single-use,
    /// preventing replay across the TTL window.
    pub async fn consume_code_exec(&self, token: &str) -> Option<CodeExecContext> {
        let cache_key = Self::code_exec_key(token);
        let result: Option<String> = self.client.getdel(&cache_key).await.ok()?;
        result.and_then(|json| serde_json::from_str(&json).ok())
    }

    /// Invalidate all cached data for a server (call when server is updated)
    pub async fn invalidate_server_all(&self, slug: &str) {
        self.invalidate_server(slug).await;
    }

    /// Cache key for workspace plan/subscription lookup by id
    fn workspace_cache_key(workspace_id: &uuid::Uuid) -> String {
        format!("proxy:workspace:{}", workspace_id)
    }

    /// Get cached workspace plan/subscription by id
    pub async fn get_workspace(&self, workspace_id: &uuid::Uuid) -> Option<CachedWorkspace> {
        let cache_key = Self::workspace_cache_key(workspace_id);
        let result: Option<String> = self.client.get(&cache_key).await.ok()?;
        result.and_then(|json| serde_json::from_str(&json).ok())
    }

    /// Cache a workspace's plan/subscription with a short TTL
    pub async fn set_workspace(&self, workspace: &Workspace) {
        let cache_key = Self::workspace_cache_key(&workspace.id);
        let cached = CachedWorkspace::from(workspace);
        if let Ok(json) = serde_json::to_string(&cached) {
            let _: Result<(), _> = self
                .client
                .set(
                    &cache_key,
                    json,
                    Some(Expiration::EX(WORKSPACE_CACHE_TTL_SECS)),
                    None,
                    false,
                )
                .await;
        }
    }

    /// Invalidate cached workspace plan/subscription (call when subscription changes)
    pub async fn invalidate_workspace(&self, workspace_id: &uuid::Uuid) {
        let cache_key = Self::workspace_cache_key(workspace_id);
        let _: Result<(), _> = self.client.del(&cache_key).await;
    }

    fn tools_list_key(server_id: &uuid::Uuid) -> String {
        format!("proxy:tools_list:{}", server_id)
    }

    /// Get the cached raw (id-stripped) tools/list response body for a server.
    pub async fn get_tools_list(&self, server_id: &uuid::Uuid) -> Option<Vec<u8>> {
        let key = Self::tools_list_key(server_id);
        let result: Option<Vec<u8>> = self.client.get(&key).await.ok()?;
        result
    }

    /// Cache the raw (id-stripped) tools/list response body for a server.
    pub async fn set_tools_list(&self, server_id: &uuid::Uuid, body: &[u8]) {
        let key = Self::tools_list_key(server_id);
        let _: Result<(), _> = self
            .client
            .set(&key, body, Some(Expiration::EX(TOOLS_LIST_CACHE_TTL_SECS)), None, false)
            .await;
    }

    /// Invalidate the tools/list cache for a server (call on redeploy or image change).
    pub async fn invalidate_tools_list(&self, server_id: &uuid::Uuid) {
        let key = Self::tools_list_key(server_id);
        let _: Result<(), _> = self.client.del(&key).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_generation() {
        assert_eq!(
            RedisCache::api_key_cache_key("abc123"),
            "proxy:apikey:abc123"
        );
        assert_eq!(
            RedisCache::server_cache_key("my-server"),
            "proxy:server:my-server"
        );
    }
}

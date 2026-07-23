use chrono::{DateTime, Utc};
use mcp_common::types::{AccessMode, Runtime, ServerStatus, Transport, Visibility};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct McpServer {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub github_repo: String,
    pub github_branch: String,
    pub github_installation_id: Option<i64>,
    pub runtime: String,
    pub visibility: String,
    pub access_mode: String,
    pub transport: String,
    pub status: String,
    pub endpoint_url: Option<String>,
    pub rate_limit_per_minute: Option<i32>,
    pub region: String,
    pub root_directory: String,
    pub mcp_path: String,
    /// Custom entry command for the MCP server (e.g., "python server.py", "uv run mcp-server")
    /// If None, auto-detect based on project structure
    pub entry_command: Option<String>,
    /// Custom build command run at image-build time (e.g., "npm run build", "npm run compile")
    /// If None, fall back to the runtime default (Node: `npm run build --if-present`)
    pub build_command: Option<String>,
    /// When false, skip NodeFlare authentication layer (for servers that handle their own auth)
    pub auth_enabled: bool,
    /// User-selected machine memory in MB (256/512/1024/2048). None = auto (builder default).
    pub memory_mb: Option<i32>,
    /// Internal listening port for Streamable HTTP (SSE) servers. None = runtime default
    /// (node 3000, python 8000, go/rust 8080). Ignored for stdio (adapter owns the port).
    pub port: Option<i32>,
    /// Container name on the bare-metal host (e.g. `mcp-<uuid-nohyphens>`). Decided ONCE at
    /// creation and persisted so it is never recomputed from a truncated UUID prefix.
    pub container_name: String,
    /// When true (default), the proxy filters a `tools/list` response down to the tools
    /// the calling credential may actually call (NodeFlare-auth mode), cutting the
    /// schema tokens an AI client loads upfront. Call-time scope checks apply regardless.
    pub tool_list_filter_by_scope: bool,
    /// When true (opt-in), the proxy trims verbose tool schemas in `tools/list` to
    /// further reduce tokens. Off by default because it alters tool descriptions.
    pub tool_schema_slim: bool,
    /// When true (opt-in), the proxy collapses `tools/list` into two meta-tools
    /// (`search_tools` + `call_tool`) so the upfront schema token cost stays roughly
    /// constant regardless of how many tools the server exposes.
    pub tool_search_mode: bool,
    /// When true (opt-in), the proxy exposes a `run_code` tool and executes AI-written
    /// JavaScript against the tool catalog in a sandbox, returning only the final
    /// result. Requires a configured code runner; otherwise treated as false.
    pub tool_code_mode: bool,
    /// Upstream OAuth provider name (matches upstream_oauth_providers.name, NULL = none)
    pub upstream_oauth_provider: Option<String>,
    /// OAuth scopes to request from the upstream provider
    pub upstream_oauth_scopes: Option<Vec<String>>,
    /// Custom provider only: OAuth2 authorization endpoint URL
    pub upstream_oauth_authorization_url: Option<String>,
    /// Custom provider only: OAuth2 token endpoint URL
    pub upstream_oauth_token_url: Option<String>,
    /// Custom provider only: OAuth2 client_id
    pub upstream_oauth_client_id: Option<String>,
    /// Custom provider only: OAuth2 client_secret (sensitive, never returned in API responses)
    pub upstream_oauth_client_secret: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl McpServer {
    /// Canonical, collision-free container name for a server: `mcp-<uuid-without-dashes>`.
    /// Uses the FULL UUID (128 bits) — the old `mcp-<first-segment>` scheme used only the
    /// first 32 bits and could map two distinct servers onto the same container name.
    pub fn new_container_name(id: Uuid) -> String {
        format!("mcp-{}", id.simple())
    }

    pub fn runtime(&self) -> Runtime {
        match self.runtime.as_str() {
            "node" => Runtime::Node,
            "python" => Runtime::Python,
            "go" => Runtime::Go,
            "rust" => Runtime::Rust,
            "docker" => Runtime::Docker,
            _ => Runtime::Node,
        }
    }

    pub fn visibility(&self) -> Visibility {
        match self.visibility.as_str() {
            "team" => Visibility::Team,
            "public" => Visibility::Public,
            _ => Visibility::Private,
        }
    }

    pub fn access_mode(&self) -> AccessMode {
        match self.access_mode.as_str() {
            "vpn_only" => AccessMode::VpnOnly,
            _ => AccessMode::Public,
        }
    }

    pub fn status(&self) -> ServerStatus {
        match self.status.as_str() {
            "building" => ServerStatus::Building,
            "deploying" => ServerStatus::Deploying,
            "running" => ServerStatus::Running,
            "failed" => ServerStatus::Failed,
            "stopped" => ServerStatus::Stopped,
            "deleting" => ServerStatus::Deleting,
            _ => ServerStatus::Inactive,
        }
    }

    pub fn transport(&self) -> Transport {
        match self.transport.as_str() {
            "stdio" => Transport::Stdio,
            _ => Transport::Sse,
        }
    }

    pub fn is_running(&self) -> bool {
        self.status == "running"
    }
}

#[derive(Debug, Clone)]
pub struct CreateServer {
    pub workspace_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub github_repo: String,
    pub github_branch: String,
    pub github_installation_id: Option<i64>,
    pub runtime: Runtime,
    pub visibility: Visibility,
    pub access_mode: AccessMode,
    pub transport: Transport,
    pub region: String,
    pub root_directory: String,
    pub mcp_path: String,
    pub entry_command: Option<String>,
    pub build_command: Option<String>,
    pub auth_enabled: bool,
    pub memory_mb: Option<i32>,
    pub port: Option<i32>,
    pub upstream_oauth_provider: Option<String>,
    pub upstream_oauth_scopes: Option<Vec<String>>,
    pub upstream_oauth_authorization_url: Option<String>,
    pub upstream_oauth_token_url: Option<String>,
    pub upstream_oauth_client_id: Option<String>,
    pub upstream_oauth_client_secret: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateServer {
    pub name: Option<String>,
    pub description: Option<String>,
    pub github_branch: Option<String>,
    pub visibility: Option<Visibility>,
    pub access_mode: Option<AccessMode>,
    pub status: Option<ServerStatus>,
    pub endpoint_url: Option<String>,
    pub region: Option<String>,
    pub root_directory: Option<String>,
    pub mcp_path: Option<String>,
    pub entry_command: Option<String>,
    pub build_command: Option<String>,
    pub auth_enabled: Option<bool>,
    pub memory_mb: Option<i32>,
    pub port: Option<i32>,
    pub tool_list_filter_by_scope: Option<bool>,
    pub tool_schema_slim: Option<bool>,
    pub tool_search_mode: Option<bool>,
    pub tool_code_mode: Option<bool>,
    pub upstream_oauth_provider: Option<String>,
    pub upstream_oauth_scopes: Option<Vec<String>>,
    pub upstream_oauth_authorization_url: Option<String>,
    pub upstream_oauth_token_url: Option<String>,
    pub upstream_oauth_client_id: Option<String>,
    pub upstream_oauth_client_secret: Option<String>,
}

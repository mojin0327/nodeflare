use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use uuid::Uuid;
use validator::Validate;

/// Regex for validating slugs: lowercase alphanumeric with hyphens, no leading/trailing hyphens
pub static SLUG_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("Invalid slug regex")
});

// ============================================================================
// Enums
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Node,
    Python,
    Go,
    Rust,
    Docker,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::Node
    }
}

impl std::fmt::Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Runtime::Node => write!(f, "node"),
            Runtime::Python => write!(f, "python"),
            Runtime::Go => write!(f, "go"),
            Runtime::Rust => write!(f, "rust"),
            Runtime::Docker => write!(f, "docker"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Private,
    Team,
    Public,
}

impl Default for Visibility {
    fn default() -> Self {
        Self::Private
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    /// Accessible via public URL
    Public,
    /// Accessible via VPN only
    VpnOnly,
}

impl Default for AccessMode {
    fn default() -> Self {
        Self::Public
    }
}

impl std::fmt::Display for AccessMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccessMode::Public => write!(f, "public"),
            AccessMode::VpnOnly => write!(f, "vpn_only"),
        }
    }
}

/// Transport type for MCP servers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// SSE transport - server already exposes HTTP/SSE endpoint
    Sse,
    /// STDIO transport - server uses stdin/stdout, needs adapter wrapper
    Stdio,
}

impl Default for Transport {
    fn default() -> Self {
        Self::Sse
    }
}

impl std::fmt::Display for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Transport::Sse => write!(f, "sse"),
            Transport::Stdio => write!(f, "stdio"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    Inactive,
    Building,
    Deploying,
    Running,
    Failed,
    Stopped,
    /// Soft-deleted: the row is being torn down. The Fly app teardown runs (DestroyJob),
    /// and only after it is confirmed is the row hard-deleted. Servers in this state are
    /// treated as gone by the UI/proxy.
    Deleting,
}

impl Default for ServerStatus {
    fn default() -> Self {
        Self::Inactive
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentStatus {
    Pending,
    Building,
    Pushing,
    Deploying,
    Succeeded,
    Failed,
    Cancelled,
}

impl Default for DeploymentStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceRole {
    Owner,
    Admin,
    Member,
    Viewer,
}

impl Default for WorkspaceRole {
    fn default() -> Self {
        Self::Member
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    Free,
    Pro,
    Team,
    Enterprise,
}

impl Default for Plan {
    fn default() -> Self {
        Self::Free
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolPermissionLevel {
    Normal,
    Elevated,
    Dangerous,
}

impl Default for ToolPermissionLevel {
    fn default() -> Self {
        Self::Normal
    }
}

// ============================================================================
// API Key Scopes
// ============================================================================

/// MCP API Key Scope definitions
///
/// Scope format: `{resource}:{action}` or `{resource}:{action}:{target}`
///
/// Examples:
/// - `*` - Full access (all permissions)
/// - `tools:*` - All tool operations
/// - `tools:list` - List available tools
/// - `tools:call` - Call any tool
/// - `tools:call:get_weather` - Call only the `get_weather` tool
/// - `resources:*` - All resource operations
/// - `resources:list` - List resources
/// - `resources:read` - Read resources
/// - `prompts:*` - All prompt operations
/// - `prompts:list` - List prompts
/// - `prompts:get` - Get prompt content
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Full access to everything
    All,
    /// All tool operations
    ToolsAll,
    /// List available tools
    ToolsList,
    /// Call any tool
    ToolsCall,
    /// Call a specific tool only
    ToolsCallSpecific(String),
    /// All resource operations
    ResourcesAll,
    /// List resources
    ResourcesList,
    /// Read resources
    ResourcesRead,
    /// Read a specific resource only
    ResourcesReadSpecific(String),
    /// All prompt operations
    PromptsAll,
    /// List prompts
    PromptsList,
    /// Get prompt content
    PromptsGet,
    /// Get a specific prompt only
    PromptsGetSpecific(String),
}

impl Scope {
    /// Parse a scope string into a Scope enum
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "*" => Some(Scope::All),
            "tools:*" => Some(Scope::ToolsAll),
            "tools:list" => Some(Scope::ToolsList),
            "tools:call" => Some(Scope::ToolsCall),
            "resources:*" => Some(Scope::ResourcesAll),
            "resources:list" => Some(Scope::ResourcesList),
            "resources:read" => Some(Scope::ResourcesRead),
            "prompts:*" => Some(Scope::PromptsAll),
            "prompts:list" => Some(Scope::PromptsList),
            "prompts:get" => Some(Scope::PromptsGet),
            _ => {
                // Check for specific target scopes
                if let Some(tool_name) = s.strip_prefix("tools:call:") {
                    Some(Scope::ToolsCallSpecific(tool_name.to_string()))
                } else if let Some(resource_uri) = s.strip_prefix("resources:read:") {
                    Some(Scope::ResourcesReadSpecific(resource_uri.to_string()))
                } else if let Some(prompt_name) = s.strip_prefix("prompts:get:") {
                    Some(Scope::PromptsGetSpecific(prompt_name.to_string()))
                } else {
                    None
                }
            }
        }
    }

    /// Convert scope to string representation
    /// Returns Cow to avoid allocation for static strings
    pub fn as_str(&self) -> Cow<'static, str> {
        match self {
            Scope::All => Cow::Borrowed("*"),
            Scope::ToolsAll => Cow::Borrowed("tools:*"),
            Scope::ToolsList => Cow::Borrowed("tools:list"),
            Scope::ToolsCall => Cow::Borrowed("tools:call"),
            Scope::ToolsCallSpecific(name) => {
                // Pre-allocate exact capacity: "tools:call:" (11) + name.len()
                let mut s = String::with_capacity(11 + name.len());
                s.push_str("tools:call:");
                s.push_str(name);
                Cow::Owned(s)
            }
            Scope::ResourcesAll => Cow::Borrowed("resources:*"),
            Scope::ResourcesList => Cow::Borrowed("resources:list"),
            Scope::ResourcesRead => Cow::Borrowed("resources:read"),
            Scope::ResourcesReadSpecific(uri) => {
                // Pre-allocate: "resources:read:" (15) + uri.len()
                let mut s = String::with_capacity(15 + uri.len());
                s.push_str("resources:read:");
                s.push_str(uri);
                Cow::Owned(s)
            }
            Scope::PromptsAll => Cow::Borrowed("prompts:*"),
            Scope::PromptsList => Cow::Borrowed("prompts:list"),
            Scope::PromptsGet => Cow::Borrowed("prompts:get"),
            Scope::PromptsGetSpecific(name) => {
                // Pre-allocate: "prompts:get:" (12) + name.len()
                let mut s = String::with_capacity(12 + name.len());
                s.push_str("prompts:get:");
                s.push_str(name);
                Cow::Owned(s)
            }
        }
    }

    /// Convert scope to owned String (for cases where String is needed)
    pub fn to_string_owned(&self) -> String {
        self.as_str().into_owned()
    }
}

/// MCP method to required scope mapping
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpMethod {
    ToolsList,
    ToolsCall,
    ResourcesList,
    ResourcesRead,
    PromptsList,
    PromptsGet,
    Unknown,
}

impl McpMethod {
    /// Parse MCP JSON-RPC method string
    pub fn parse(method: &str) -> Self {
        match method {
            "tools/list" => McpMethod::ToolsList,
            "tools/call" => McpMethod::ToolsCall,
            "resources/list" => McpMethod::ResourcesList,
            "resources/read" => McpMethod::ResourcesRead,
            "prompts/list" => McpMethod::PromptsList,
            "prompts/get" => McpMethod::PromptsGet,
            _ => McpMethod::Unknown,
        }
    }
}

/// Scope checker for API key authorization
pub struct ScopeChecker {
    scopes: Vec<Scope>,
}

impl ScopeChecker {
    /// Create a new scope checker from a list of scope strings
    pub fn new(scope_strings: &[String]) -> Self {
        let scopes: Vec<Scope> = scope_strings
            .iter()
            .filter_map(|s| Scope::parse(s))
            .collect();
        Self { scopes }
    }

    /// Check if the API key has permission for an MCP method
    pub fn is_allowed(&self, method: McpMethod, target: Option<&str>) -> bool {
        // If no valid scopes, deny by default
        if self.scopes.is_empty() {
            return false;
        }

        for scope in &self.scopes {
            match scope {
                // Wildcard allows everything
                Scope::All => return true,

                // Tools scopes
                Scope::ToolsAll => {
                    if matches!(method, McpMethod::ToolsList | McpMethod::ToolsCall) {
                        return true;
                    }
                }
                Scope::ToolsList => {
                    if matches!(method, McpMethod::ToolsList) {
                        return true;
                    }
                }
                Scope::ToolsCall => {
                    if matches!(method, McpMethod::ToolsCall) {
                        return true;
                    }
                }
                Scope::ToolsCallSpecific(allowed_tool) => {
                    if matches!(method, McpMethod::ToolsCall) {
                        if let Some(tool_name) = target {
                            if tool_name == allowed_tool {
                                return true;
                            }
                        }
                    }
                }

                // Resources scopes
                Scope::ResourcesAll => {
                    if matches!(method, McpMethod::ResourcesList | McpMethod::ResourcesRead) {
                        return true;
                    }
                }
                Scope::ResourcesList => {
                    if matches!(method, McpMethod::ResourcesList) {
                        return true;
                    }
                }
                Scope::ResourcesRead => {
                    if matches!(method, McpMethod::ResourcesRead) {
                        return true;
                    }
                }
                Scope::ResourcesReadSpecific(allowed_uri) => {
                    if matches!(method, McpMethod::ResourcesRead) {
                        if let Some(uri) = target {
                            if uri == allowed_uri {
                                return true;
                            }
                        }
                    }
                }

                // Prompts scopes
                Scope::PromptsAll => {
                    if matches!(method, McpMethod::PromptsList | McpMethod::PromptsGet) {
                        return true;
                    }
                }
                Scope::PromptsList => {
                    if matches!(method, McpMethod::PromptsList) {
                        return true;
                    }
                }
                Scope::PromptsGet => {
                    if matches!(method, McpMethod::PromptsGet) {
                        return true;
                    }
                }
                Scope::PromptsGetSpecific(allowed_prompt) => {
                    if matches!(method, McpMethod::PromptsGet) {
                        if let Some(prompt_name) = target {
                            if prompt_name == allowed_prompt {
                                return true;
                            }
                        }
                    }
                }
            }
        }

        false
    }

    /// Check if the API key has any valid scopes
    pub fn has_any_scope(&self) -> bool {
        !self.scopes.is_empty()
    }

    /// Get list of available predefined scopes (for UI)
    pub fn predefined_scopes() -> Vec<(&'static str, &'static str)> {
        vec![
            ("*", "Full access - all permissions"),
            ("tools:*", "Tools - all operations"),
            ("tools:list", "Tools - list only"),
            ("tools:call", "Tools - execute any tool"),
            ("resources:*", "Resources - all operations"),
            ("resources:list", "Resources - list only"),
            ("resources:read", "Resources - read any"),
            ("prompts:*", "Prompts - all operations"),
            ("prompts:list", "Prompts - list only"),
            ("prompts:get", "Prompts - get any"),
        ]
    }
}

// ============================================================================
// Request DTOs
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateWorkspaceRequest {
    #[validate(length(min = 1, max = 255))]
    pub name: String,
    #[validate(length(min = 1, max = 63), custom(function = "validate_slug"))]
    pub slug: String,
}

fn validate_slug(slug: &str) -> Result<(), validator::ValidationError> {
    if SLUG_REGEX.is_match(slug) {
        Ok(())
    } else {
        Err(validator::ValidationError::new("invalid_slug"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateServerRequest {
    #[validate(length(min = 1, max = 255))]
    pub name: String,
    #[validate(length(min = 1, max = 63), custom(function = "validate_slug"))]
    pub slug: String,
    #[validate(length(max = 1000))]
    pub description: Option<String>,
    #[validate(length(min = 1, max = 255))]
    pub github_repo: String,
    #[validate(length(max = 255))]
    pub github_branch: Option<String>,
    pub github_installation_id: Option<i64>,
    pub runtime: Option<Runtime>,
    pub visibility: Option<Visibility>,
    pub access_mode: Option<AccessMode>,
    /// Transport type: sse (HTTP/SSE endpoint) or stdio (stdin/stdout with adapter)
    pub transport: Option<Transport>,
    #[validate(length(max = 20))]
    pub region: Option<String>,
    #[validate(length(max = 255))]
    pub root_directory: Option<String>,
    #[validate(length(max = 255))]
    pub mcp_path: Option<String>,
    /// Custom entry command for the MCP server (e.g., "python server.py", "uv run mcp-server")
    /// If None, auto-detect based on project structure
    #[validate(length(max = 500))]
    pub entry_command: Option<String>,
    /// Custom build command run at image-build time (e.g., "npm run build", "npm run compile")
    /// If None, fall back to the runtime default (Node: `npm run build --if-present`)
    #[validate(length(max = 500))]
    pub build_command: Option<String>,
    /// When false, skip NodeFlare authentication layer (for servers that handle their own auth)
    /// Defaults to true if not specified
    pub auth_enabled: Option<bool>,
    /// User-selected machine memory in MB (256/512/1024/2048). None = auto.
    /// Validated against the workspace plan's ceiling at the API layer.
    pub memory_mb: Option<i32>,
    /// Internal listening port for Streamable HTTP (SSE) servers. None = runtime default
    /// (node 3000, python 8000, go/rust 8080). Ignored for stdio transport.
    #[validate(range(min = 1, max = 65535))]
    pub port: Option<i32>,
    /// Environment variables (secrets) to provision at creation time, before the
    /// initial auto-deploy. Persisted encrypted so the first build picks them up
    /// without requiring a second deploy. Each entry's key/value are validated at
    /// the API layer (same rules as the secrets endpoint).
    #[validate(length(max = 100))]
    pub env_vars: Option<Vec<SetSecretRequest>>,
    /// Upstream OAuth provider name (matches upstream_oauth_providers.name, None = disabled).
    pub upstream_oauth_provider: Option<String>,
    /// Scopes to request from the upstream OAuth provider.
    pub upstream_oauth_scopes: Option<Vec<String>>,
    /// Custom provider only: OAuth2 authorization endpoint URL.
    pub upstream_oauth_authorization_url: Option<String>,
    /// Custom provider only: OAuth2 token endpoint URL.
    pub upstream_oauth_token_url: Option<String>,
    /// Custom provider only: OAuth2 client_id.
    pub upstream_oauth_client_id: Option<String>,
    /// Custom provider only: OAuth2 client_secret (write-only, never returned in responses).
    pub upstream_oauth_client_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct UpdateServerRequest {
    #[validate(length(min = 1, max = 255))]
    pub name: Option<String>,
    pub description: Option<String>,
    pub github_branch: Option<String>,
    pub visibility: Option<Visibility>,
    pub access_mode: Option<AccessMode>,
    pub region: Option<String>,
    #[validate(length(max = 255))]
    pub root_directory: Option<String>,
    #[validate(length(max = 255))]
    pub mcp_path: Option<String>,
    /// Custom entry command for the MCP server (e.g., "python server.py", "uv run mcp-server")
    #[validate(length(max = 500))]
    pub entry_command: Option<String>,
    /// Custom build command run at image-build time (e.g., "npm run build", "npm run compile")
    #[validate(length(max = 500))]
    pub build_command: Option<String>,
    /// When false, skip NodeFlare authentication layer (for servers that handle their own auth)
    pub auth_enabled: Option<bool>,
    /// User-selected machine memory in MB (256/512/1024/2048). None = leave unchanged.
    pub memory_mb: Option<i32>,
    /// Internal listening port for Streamable HTTP (SSE) servers. None = leave unchanged.
    #[validate(range(min = 1, max = 65535))]
    pub port: Option<i32>,
    /// Filter `tools/list` down to tools the calling credential may call. None = unchanged.
    pub tool_list_filter_by_scope: Option<bool>,
    /// Trim verbose tool schemas in `tools/list` to save tokens. None = unchanged.
    pub tool_schema_slim: Option<bool>,
    /// Collapse `tools/list` into search_tools + call_tool meta-tools. None = unchanged.
    pub tool_search_mode: Option<bool>,
    /// Expose run_code and execute AI-written code in a sandbox. None = unchanged.
    pub tool_code_mode: Option<bool>,
    /// Upstream OAuth provider name (matches upstream_oauth_providers.name, None = disabled).
    pub upstream_oauth_provider: Option<String>,
    /// Scopes to request from the upstream OAuth provider.
    pub upstream_oauth_scopes: Option<Vec<String>>,
    /// Custom provider only: OAuth2 authorization endpoint URL.
    pub upstream_oauth_authorization_url: Option<String>,
    /// Custom provider only: OAuth2 token endpoint URL.
    pub upstream_oauth_token_url: Option<String>,
    /// Custom provider only: OAuth2 client_id.
    pub upstream_oauth_client_id: Option<String>,
    /// Custom provider only: OAuth2 client_secret (write-only, never returned in responses).
    pub upstream_oauth_client_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateApiKeyRequest {
    #[validate(length(min = 1, max = 255))]
    pub name: String,
    pub server_id: Option<Uuid>,
    pub scopes: Option<Vec<String>>,
    pub expires_in_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct UpdateToolRequest {
    pub enabled: Option<bool>,
    pub permission_level: Option<ToolPermissionLevel>,
    pub rate_limit_per_minute: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetSecretRequest {
    pub key: String,
    pub value: String,
}

// ============================================================================
// Response DTOs
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub plan: Plan,
    pub role: WorkspaceRole,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub github_repo: String,
    pub github_branch: String,
    pub runtime: Runtime,
    pub visibility: Visibility,
    pub access_mode: AccessMode,
    /// Transport type: sse (HTTP/SSE endpoint) or stdio (stdin/stdout with adapter)
    pub transport: Transport,
    pub status: ServerStatus,
    pub endpoint_url: Option<String>,
    pub region: String,
    pub root_directory: String,
    pub mcp_path: String,
    /// Custom entry command for the MCP server
    pub entry_command: Option<String>,
    /// Custom build command run at image-build time
    pub build_command: Option<String>,
    /// When false, NodeFlare authentication is disabled (for servers with their own auth)
    pub auth_enabled: bool,
    /// User-selected machine memory in MB (None = auto).
    pub memory_mb: Option<i32>,
    /// Internal listening port for Streamable HTTP (SSE) servers (None = runtime default).
    pub port: Option<i32>,
    /// When true, the proxy filters `tools/list` to tools the credential may call.
    pub tool_list_filter_by_scope: bool,
    /// When true, the proxy trims verbose tool schemas in `tools/list`.
    pub tool_schema_slim: bool,
    /// When true, the proxy collapses `tools/list` into search_tools + call_tool.
    pub tool_search_mode: bool,
    /// When true, the proxy exposes run_code and executes code in a sandbox.
    pub tool_code_mode: bool,
    /// Upstream OAuth provider name (matches upstream_oauth_providers.name, None = disabled).
    pub upstream_oauth_provider: Option<String>,
    /// Scopes requested from the upstream OAuth provider.
    pub upstream_oauth_scopes: Option<Vec<String>>,
    /// Custom provider only: OAuth2 authorization endpoint URL.
    pub upstream_oauth_authorization_url: Option<String>,
    /// Custom provider only: OAuth2 token endpoint URL.
    pub upstream_oauth_token_url: Option<String>,
    /// Custom provider only: OAuth2 client_id (client_secret is write-only and never returned).
    pub upstream_oauth_client_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamOAuthProviderResponse {
    pub name: String,
    pub display_name: String,
    pub authorization_url: String,
    pub token_url: String,
    pub default_scopes: Vec<String>,
    pub is_managed: bool,
}

/// Minimal server response for selection lists (only id and name)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMinimalResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
}

/// Basic server response for dashboard/overview (includes status and runtime)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerBasicResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub status: ServerStatus,
    pub runtime: Runtime,
}

/// Server list response for server list page (display-relevant fields only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerListResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub slug: String,
    pub runtime: Runtime,
    pub visibility: Visibility,
    pub status: ServerStatus,
    pub github_repo: String,
    pub endpoint_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentResponse {
    pub id: Uuid,
    pub server_id: Uuid,
    pub version: i32,
    pub commit_sha: String,
    pub status: DeploymentStatus,
    pub error_message: Option<String>,
    pub build_logs: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Alias for started_at (for frontend compatibility)
    pub created_at: DateTime<Utc>,
    /// Alias for finished_at (for frontend compatibility)
    pub deployed_at: Option<DateTime<Utc>>,
    /// Build duration in seconds (computed from finished_at - started_at)
    pub build_duration_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponse {
    pub id: Uuid,
    pub server_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<serde_json::Value>,
    pub enabled: bool,
    pub permission_level: ToolPermissionLevel,
    pub rate_limit_per_minute: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyResponse {
    pub id: Uuid,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub server_id: Option<Uuid>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyCreatedResponse {
    pub id: Uuid,
    pub name: String,
    pub key: String, // Full key, only shown once
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretResponse {
    pub key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogResponse {
    pub id: Uuid,
    pub server_id: Uuid,
    pub tool_name: Option<String>,
    pub response_status: String,
    pub duration_ms: i32,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Auth
// ============================================================================

/// Authentication response returned after login or token refresh.
///
/// SECURITY NOTE: The refresh_token is optionally included in the JSON response
/// for API clients (CLI tools, native apps) that cannot use cookies.
/// Web applications should use HttpOnly cookies for refresh tokens instead.
/// Set skip_serializing_if to hide refresh_token unless explicitly needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub access_token: String,
    /// Refresh token. Only included when X-Include-Refresh-Token header is set,
    /// or when accessed from non-browser clients. Web apps should rely on
    /// the HttpOnly cookie instead for better security against XSS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_in: i64,
    pub user: UserResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

// ============================================================================
// Pagination
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self {
            page: Some(1),
            per_page: Some(20),
        }
    }
}

impl PaginationParams {
    pub fn offset(&self) -> u32 {
        let page = self.page.unwrap_or(1).max(1);
        let per_page = self.per_page.unwrap_or(20);
        (page - 1) * per_page
    }

    pub fn limit(&self) -> u32 {
        self.per_page.unwrap_or(20).min(100)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub data: Vec<T>,
    pub pagination: PaginationMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
    pub total_pages: u32,
}

// ============================================================================
// WebSocket Messages
// ============================================================================

/// WebSocket message types for real-time updates
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WsMessage {
    /// Deployment status update
    DeploymentStatus(DeploymentStatusUpdate),
    /// Server status update
    ServerStatus(ServerStatusUpdate),
    /// Build log line
    BuildLog(BuildLogLine),
    /// Server log line
    ServerLog(ServerLogLine),
    /// Error message
    Error(WsError),
    /// Ping/Pong for connection keepalive
    Ping,
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentStatusUpdate {
    pub deployment_id: Uuid,
    pub server_id: Uuid,
    pub status: DeploymentStatus,
    pub error_message: Option<String>,
    pub progress: Option<u8>, // 0-100
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatusUpdate {
    pub server_id: Uuid,
    pub status: ServerStatus,
    pub endpoint_url: Option<String>,
    pub error_message: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildLogLine {
    pub deployment_id: Uuid,
    pub line: String,
    pub stream: LogStream,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerLogLine {
    pub server_id: Uuid,
    pub line: String,
    pub level: LogLevel,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsError {
    pub code: String,
    pub message: String,
}


use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ServerTemplate {
    pub id: Uuid,
    pub server_id: Uuid,
    pub owner_id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub github_repo: String,
    pub github_branch: String,
    pub runtime: String,
    pub transport: String,
    pub mcp_path: String,
    pub entry_command: Option<String>,
    pub build_command: Option<String>,
    pub root_directory: String,
    pub auth_enabled: bool,
    pub memory_mb: Option<i32>,
    pub port: Option<i32>,
    pub upstream_oauth_provider: Option<String>,
    pub upstream_oauth_scopes: Vec<String>,
    pub required_env_var_keys: Vec<String>,
    pub use_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateServerTemplate {
    pub server_id: Uuid,
    pub owner_id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub github_repo: String,
    pub github_branch: String,
    pub runtime: String,
    pub transport: String,
    pub mcp_path: String,
    pub entry_command: Option<String>,
    pub build_command: Option<String>,
    pub root_directory: String,
    pub auth_enabled: bool,
    pub memory_mb: Option<i32>,
    pub port: Option<i32>,
    pub upstream_oauth_provider: Option<String>,
    pub upstream_oauth_scopes: Vec<String>,
    pub required_env_var_keys: Vec<String>,
}

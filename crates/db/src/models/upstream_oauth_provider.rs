use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UpstreamOAuthProvider {
    pub id: Uuid,
    pub name: String,
    pub display_name: String,
    pub authorization_url: String,
    pub token_url: String,
    pub default_scopes: Vec<String>,
    pub is_managed: bool,
    pub is_enabled: bool,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// OAuth client_id stored in DB (takes precedence over env var for managed providers).
    pub client_id: Option<String>,
    /// OAuth client_secret stored in DB (takes precedence over env var for managed providers).
    pub client_secret: Option<String>,
    /// Extra query params appended to the authorization URL (e.g. access_type/prompt for Google).
    pub extra_auth_params: JsonValue,
    /// Whether a missing refresh_token in the callback should be treated as an error.
    pub requires_refresh_token: bool,
}

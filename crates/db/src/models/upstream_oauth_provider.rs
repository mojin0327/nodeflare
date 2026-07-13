use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
}

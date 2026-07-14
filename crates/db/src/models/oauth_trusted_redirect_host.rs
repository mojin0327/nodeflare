use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OAuthTrustedRedirectHost {
    pub hostname: String,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
}

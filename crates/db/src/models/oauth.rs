use chrono::{DateTime, Utc};
use mcp_common::{McpMethod, ScopeChecker};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// OAuth Client (Application)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OAuthClient {
    pub id: Uuid,
    pub client_id: String,
    #[serde(skip_serializing)]
    pub client_secret_hash: Option<String>,
    pub client_name: String,
    pub redirect_uris: serde_json::Value,
    pub grant_types: serde_json::Value,
    pub token_endpoint_auth_method: String,
    pub workspace_id: Option<Uuid>,
    pub server_id: Option<Uuid>,
    pub scopes: serde_json::Value,
    pub is_active: bool,
    pub is_dynamic: bool,
    pub software_id: Option<String>,
    pub software_version: Option<String>,
    /// Access token lifetime in seconds for tokens issued to this client.
    /// None = no expiration.
    pub access_token_ttl_seconds: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl OAuthClient {
    pub fn redirect_uris(&self) -> Vec<String> {
        serde_json::from_value(self.redirect_uris.clone()).unwrap_or_default()
    }

    pub fn grant_types(&self) -> Vec<String> {
        serde_json::from_value(self.grant_types.clone()).unwrap_or_default()
    }

    pub fn scopes(&self) -> Vec<String> {
        serde_json::from_value(self.scopes.clone()).unwrap_or_default()
    }

    pub fn is_redirect_uri_valid(&self, uri: &str) -> bool {
        self.redirect_uris().iter().any(|u| u == uri)
    }

    pub fn supports_grant_type(&self, grant_type: &str) -> bool {
        self.grant_types().iter().any(|g| g == grant_type)
    }
}

#[derive(Debug, Clone)]
pub struct CreateOAuthClient {
    pub client_id: String,
    pub client_secret_hash: Option<String>,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub workspace_id: Option<Uuid>,
    pub server_id: Option<Uuid>,
    pub scopes: Vec<String>,
    pub is_dynamic: bool,
    pub software_id: Option<String>,
    pub software_version: Option<String>,
    /// Access token lifetime in seconds. None = no expiration.
    pub access_token_ttl_seconds: Option<i64>,
}

/// OAuth Authorization Code
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OAuthAuthorizationCode {
    pub id: Uuid,
    #[serde(skip_serializing)]
    pub code_hash: String,
    pub client_id: Uuid,
    pub user_id: Uuid,
    pub redirect_uri: String,
    pub scopes: serde_json::Value,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl OAuthAuthorizationCode {
    pub fn scopes(&self) -> Vec<String> {
        serde_json::from_value(self.scopes.clone()).unwrap_or_default()
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at < Utc::now()
    }

    pub fn is_used(&self) -> bool {
        self.used_at.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct CreateOAuthAuthorizationCode {
    pub code_hash: String,
    pub client_id: Uuid,
    pub user_id: Uuid,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub expires_at: DateTime<Utc>,
}

/// OAuth Access Token
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OAuthAccessToken {
    pub id: Uuid,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub client_id: Uuid,
    pub user_id: Uuid,
    pub scopes: serde_json::Value,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl OAuthAccessToken {
    pub fn scopes(&self) -> Vec<String> {
        serde_json::from_value(self.scopes.clone()).unwrap_or_default()
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at < Utc::now()
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    pub fn is_valid(&self) -> bool {
        !self.is_expired() && !self.is_revoked()
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        let scopes = self.scopes();
        scopes.iter().any(|s| s == scope || s == "*")
    }

    /// Create a scope checker for this access token
    pub fn scope_checker(&self) -> ScopeChecker {
        ScopeChecker::new(&self.scopes())
    }

    /// Check if this access token is allowed to perform an MCP method
    pub fn is_method_allowed(&self, method: McpMethod, target: Option<&str>) -> bool {
        self.scope_checker().is_allowed(method, target)
    }
}

#[derive(Debug, Clone)]
pub struct CreateOAuthAccessToken {
    pub token_hash: String,
    pub client_id: Uuid,
    pub user_id: Uuid,
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
}

/// OAuth Refresh Token
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OAuthRefreshToken {
    pub id: Uuid,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub client_id: Uuid,
    pub user_id: Uuid,
    pub scopes: serde_json::Value,
    pub access_token_id: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl OAuthRefreshToken {
    pub fn scopes(&self) -> Vec<String> {
        serde_json::from_value(self.scopes.clone()).unwrap_or_default()
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at < Utc::now()
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    pub fn is_valid(&self) -> bool {
        !self.is_expired() && !self.is_revoked()
    }
}

#[derive(Debug, Clone)]
pub struct CreateOAuthRefreshToken {
    pub token_hash: String,
    pub client_id: Uuid,
    pub user_id: Uuid,
    pub scopes: Vec<String>,
    pub access_token_id: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
}

/// OAuth Access Token with Client info (for validation)
#[derive(Debug, Clone, FromRow)]
pub struct OAuthAccessTokenWithClient {
    // Token fields
    pub id: Uuid,
    pub token_hash: String,
    pub client_id: Uuid,
    pub user_id: Uuid,
    pub scopes: serde_json::Value,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    // Client fields
    pub oauth_client_id: String,
    pub workspace_id: Option<Uuid>,
    pub server_id: Option<Uuid>,
    pub client_is_active: bool,
}

impl OAuthAccessTokenWithClient {
    pub fn scopes(&self) -> Vec<String> {
        serde_json::from_value(self.scopes.clone()).unwrap_or_default()
    }

    pub fn is_valid(&self) -> bool {
        !self.is_expired() && !self.is_revoked() && self.client_is_active
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at < Utc::now()
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    pub fn scope_checker(&self) -> ScopeChecker {
        ScopeChecker::new(&self.scopes())
    }

    pub fn is_method_allowed(&self, method: McpMethod, target: Option<&str>) -> bool {
        self.scope_checker().is_allowed(method, target)
    }
}

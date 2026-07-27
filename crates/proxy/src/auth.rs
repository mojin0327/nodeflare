use axum::{body::Body, http::Request};
use mcp_auth::ApiKeyService;
use mcp_common::{McpMethod, ScopeChecker};
use mcp_db::{ApiKey, ApiKeyRepository, OAuthAccessTokenRepository, OAuthAccessTokenWithClient};
use uuid::Uuid;

use crate::rate_limit::{
    clear_api_key_failed_attempts, get_api_key_lockout_remaining, is_api_key_locked_out,
    record_api_key_failed_attempt,
};
use crate::{ProxyError, ProxyState};

/// Represents a validated authentication credential (either API key or OAuth token)
#[derive(Debug, Clone)]
pub enum AuthCredential {
    ApiKey(ApiKey),
    OAuthToken(OAuthAccessTokenWithClient),
}

impl AuthCredential {
    #[allow(dead_code)]
    pub fn workspace_id(&self) -> Option<Uuid> {
        match self {
            AuthCredential::ApiKey(key) => Some(key.workspace_id),
            AuthCredential::OAuthToken(token) => token.workspace_id,
        }
    }

    pub fn server_id(&self) -> Option<Uuid> {
        match self {
            AuthCredential::ApiKey(key) => key.server_id,
            AuthCredential::OAuthToken(token) => token.server_id,
        }
    }

    pub fn scope_checker(&self) -> ScopeChecker {
        match self {
            AuthCredential::ApiKey(key) => key.scope_checker(),
            AuthCredential::OAuthToken(token) => token.scope_checker(),
        }
    }

    pub fn is_method_allowed(&self, method: McpMethod, target: Option<&str>) -> bool {
        self.scope_checker().is_allowed(method, target)
    }

    /// Raw scope strings, persisted into a code-exec token so the sandbox tool-call
    /// endpoint can re-check scope without holding the credential.
    pub fn scopes(&self) -> Vec<String> {
        match self {
            AuthCredential::ApiKey(key) => key.scopes(),
            AuthCredential::OAuthToken(token) => token.scopes(),
        }
    }

    pub fn id(&self) -> Uuid {
        match self {
            AuthCredential::ApiKey(key) => key.id,
            AuthCredential::OAuthToken(token) => token.id,
        }
    }
}

/// Token type detected from the token format
#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    ApiKey,      // Starts with "mcp_"
    OAuthToken,  // Any other format
}

/// Detect the type of token based on its format
pub fn detect_token_type(token: &str) -> TokenType {
    if token.starts_with("mcp_") {
        TokenType::ApiKey
    } else {
        TokenType::OAuthToken
    }
}

/// Validate any credential (API key or OAuth token)
pub async fn validate_credential(
    state: &ProxyState,
    token: &str,
    client_ip: &str,
) -> Result<AuthCredential, ProxyError> {
    match detect_token_type(token) {
        TokenType::ApiKey => {
            let api_key = validate_api_key(state, token, client_ip).await?;
            Ok(AuthCredential::ApiKey(api_key))
        }
        TokenType::OAuthToken => {
            let oauth_token = validate_oauth_token(state, token, client_ip).await?;
            Ok(AuthCredential::OAuthToken(oauth_token))
        }
    }
}

pub fn extract_api_key(request: &Request<Body>) -> Result<String, ProxyError> {
    // Check Authorization header first
    if let Some(auth_header) = request.headers().get("authorization") {
        let auth_str = auth_header
            .to_str()
            .map_err(|_| ProxyError::Unauthorized("Invalid authorization header".into(), None))?;

        if let Some(key) = auth_str.strip_prefix("Bearer ") {
            return Ok(key.to_string());
        }
    }

    // Check X-API-Key header
    if let Some(api_key_header) = request.headers().get("x-api-key") {
        let key = api_key_header
            .to_str()
            .map_err(|_| ProxyError::Unauthorized("Invalid API key header".into(), None))?;
        return Ok(key.to_string());
    }

    // NOTE: Query parameter API key support removed for security reasons.
    // API keys in URLs are logged in access logs, browser history, and proxies.
    // Use Authorization header (Bearer) or X-API-Key header instead.

    Err(ProxyError::Unauthorized("Missing API key. Use Authorization header or X-API-Key header.".into(), None))
}

pub async fn validate_api_key(
    state: &ProxyState,
    api_key: &str,
    client_ip: &str,
) -> Result<ApiKey, ProxyError> {
    // Check if IP is locked out due to brute force
    if is_api_key_locked_out(state, client_ip).await {
        let remaining = get_api_key_lockout_remaining(state, client_ip)
            .await
            .unwrap_or(0);
        return Err(ProxyError::Unauthorized(format!(
            "Too many failed attempts. Please try again in {} seconds.",
            remaining
        ), None));
    }

    // Validate format
    if !ApiKeyService::is_valid_format(api_key) {
        record_api_key_failed_attempt(state, client_ip).await;
        return Err(ProxyError::Unauthorized("Invalid API key format".into(), None));
    }

    // Hash and lookup
    let key_hash = ApiKeyService::hash_key(api_key);

    // Try Redis cache first
    if let Some(cached) = state.redis_cache.get_api_key(&key_hash).await {
        let api_key_record = cached.to_api_key();

        // Check expiration
        if api_key_record.is_expired() {
            record_api_key_failed_attempt(state, client_ip).await;
            return Err(ProxyError::Unauthorized("API key expired".into(), None));
        }

        // Clear failed attempts on success
        clear_api_key_failed_attempts(state, client_ip).await;

        // Update last used (async, don't block)
        let db = state.db.clone();
        let key_id = api_key_record.id;
        tokio::spawn(async move {
            let _ = ApiKeyRepository::update_last_used(&db, key_id).await;
        });

        return Ok(api_key_record);
    }

    // Cache miss - query database
    let api_key_record = match ApiKeyRepository::find_by_hash(&state.db, &key_hash).await {
        Ok(Some(record)) => record,
        Ok(None) => {
            record_api_key_failed_attempt(state, client_ip).await;
            return Err(ProxyError::Unauthorized("Invalid API key".into(), None));
        }
        Err(e) => return Err(ProxyError::Internal(e.to_string())),
    };

    // Check expiration
    if api_key_record.is_expired() {
        record_api_key_failed_attempt(state, client_ip).await;
        return Err(ProxyError::Unauthorized("API key expired".into(), None));
    }

    // Clear failed attempts on success
    clear_api_key_failed_attempts(state, client_ip).await;

    // Cache the API key (async, don't block)
    let redis_cache = state.redis_cache.clone();
    let api_key_clone = api_key_record.clone();
    let key_hash_owned = key_hash.clone();
    tokio::spawn(async move {
        redis_cache.set_api_key(&key_hash_owned, &api_key_clone).await;
    });

    // Update last used (async, don't block)
    let db = state.db.clone();
    let key_id = api_key_record.id;
    tokio::spawn(async move {
        let _ = ApiKeyRepository::update_last_used(&db, key_id).await;
    });

    Ok(api_key_record)
}

/// Validate an OAuth access token
pub async fn validate_oauth_token(
    state: &ProxyState,
    token: &str,
    client_ip: &str,
) -> Result<OAuthAccessTokenWithClient, ProxyError> {
    tracing::debug!("Validating OAuth token from {}", client_ip);

    // Check if IP is locked out due to brute force
    tracing::debug!("Checking lockout status for {}", client_ip);
    if is_api_key_locked_out(state, client_ip).await {
        let remaining = get_api_key_lockout_remaining(state, client_ip)
            .await
            .unwrap_or(0);
        return Err(ProxyError::Unauthorized(format!(
            "Too many failed attempts. Please try again in {} seconds.",
            remaining
        ), None));
    }
    tracing::debug!("Lockout check passed for {}", client_ip);

    // Hash and lookup
    let token_hash = ApiKeyService::hash_key(token);
    tracing::debug!("Token hashed, querying database...");

    // Query database (OAuth tokens are not cached in Redis for now)
    let start = std::time::Instant::now();
    let oauth_token = match OAuthAccessTokenRepository::find_by_token_hash_with_client(&state.db, &token_hash).await {
        Ok(Some(record)) => {
            tracing::info!("OAuth token found in DB (took {:?}), client_id={}", start.elapsed(), record.oauth_client_id);
            record
        }
        Ok(None) => {
            tracing::warn!("OAuth token NOT found in DB (took {:?})", start.elapsed());
            record_api_key_failed_attempt(state, client_ip).await;
            return Err(ProxyError::Unauthorized("Invalid access token".into(), None));
        }
        Err(e) => {
            tracing::error!("Database error during OAuth token lookup (took {:?}): {}", start.elapsed(), e);
            return Err(ProxyError::Internal(e.to_string()));
        }
    };

    // Check validity (expiration, revocation, client active)
    if !oauth_token.is_valid() {
        tracing::warn!("OAuth token invalid: expired={}, revoked={}, client_active={}",
            oauth_token.is_expired(), oauth_token.is_revoked(), oauth_token.client_is_active);
        record_api_key_failed_attempt(state, client_ip).await;
        return Err(ProxyError::Unauthorized("Access token expired or revoked".into(), None));
    }

    // Clear failed attempts on success
    tracing::debug!("Clearing failed attempts for {}", client_ip);
    clear_api_key_failed_attempts(state, client_ip).await;

    // Update last used (async, don't block)
    let db = state.db.clone();
    let token_id = oauth_token.id;
    tokio::spawn(async move {
        let _ = OAuthAccessTokenRepository::update_last_used(&db, token_id).await;
    });

    tracing::info!("OAuth token validated successfully for workspace_id={:?}", oauth_token.workspace_id);
    Ok(oauth_token)
}

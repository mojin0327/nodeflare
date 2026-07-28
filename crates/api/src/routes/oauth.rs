use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{header::HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Form, Json,
};
use std::net::SocketAddr;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use mcp_auth::ApiKeyService;
use mcp_common::types::WorkspaceRole;
use mcp_db::{
    OAuthAccessTokenRepository, OAuthAuthorizationCodeRepository, OAuthClient, OAuthClientRepository,
    OAuthRefreshTokenRepository, WorkspaceRepository, ServerRepository,
    CreateOAuthAccessToken, CreateOAuthAuthorizationCode, CreateOAuthClient,
    CreateOAuthRefreshToken,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;
use rand::RngCore;

use crate::error::db_error;
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::routes::helpers::{ERR_INSUFFICIENT_PERMISSIONS, ERR_NOT_A_MEMBER, ERR_SERVER_NOT_FOUND};

// Token expiration times
const REFRESH_TOKEN_EXPIRES_DAYS: i64 = 30;
const AUTH_CODE_EXPIRES_MINUTES: i64 = 10;
// Default access token TTL for clients without an explicit setting (e.g. Claude's
// dynamically-registered clients). 30 days.
const DEFAULT_ACCESS_TOKEN_TTL_SECONDS: i64 = 30 * 24 * 3600;
// Sentinel lifetime used when a client opts into "no expiration" (~100 years).
const NO_EXPIRATION_SECONDS: i64 = 100 * 365 * 24 * 3600;

/// Compute (expires_at, expires_in_seconds) for an access token from a client's
/// configured TTL. `None` means no expiration.
fn access_token_expiry(ttl_seconds: Option<i64>) -> (chrono::DateTime<Utc>, i64) {
    let secs = ttl_seconds.unwrap_or(NO_EXPIRATION_SECONDS);
    (Utc::now() + Duration::seconds(secs), secs)
}

// ====================
// OAuth Client Management (for workspace owners)
// ====================

#[derive(Debug, Deserialize)]
pub struct CreateOAuthClientRequest {
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub server_id: Option<Uuid>,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    /// Access token lifetime in days. Omitted/null = no expiration.
    #[serde(default)]
    pub expires_in_days: Option<i64>,
}

fn default_scopes() -> Vec<String> {
    vec!["*".to_string()]
}

#[derive(Debug, Serialize)]
pub struct OAuthClientResponse {
    pub id: Uuid,
    pub client_id: String,
    pub client_secret: Option<String>, // Only returned on creation
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub server_id: Option<Uuid>,
    pub scopes: Vec<String>,
    pub created_at: String,
}

fn to_oauth_client_response(client: OAuthClient, secret: Option<String>) -> OAuthClientResponse {
    let redirect_uris = client.redirect_uris();
    let scopes = client.scopes();
    let created_at = client.created_at.to_rfc3339();
    OAuthClientResponse {
        id: client.id,
        client_id: client.client_id,
        client_secret: secret,
        client_name: client.client_name,
        redirect_uris,
        server_id: client.server_id,
        scopes,
        created_at,
    }
}

/// List OAuth clients for a workspace
pub async fn list_clients(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<OAuthClientResponse>>, (StatusCode, String)> {
    // Verify user is a member of this workspace
    let _ = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    let clients = OAuthClientRepository::list_by_workspace(&state.db, workspace_id)
        .await
        .map_err(db_error)?;

    let response: Vec<OAuthClientResponse> = clients
        .into_iter()
        .map(|c| to_oauth_client_response(c, None))
        .collect();

    Ok(Json(response))
}

/// Create a new OAuth client
pub async fn create_client(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<CreateOAuthClientRequest>,
) -> Result<Json<OAuthClientResponse>, (StatusCode, String)> {
    // Verify user is a member of this workspace with sufficient permissions
    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    // Only Admin and Owner can create OAuth clients
    if matches!(member.role(), WorkspaceRole::Viewer) {
        return Err((StatusCode::FORBIDDEN, ERR_INSUFFICIENT_PERMISSIONS.to_string()));
    }

    // Generate client_id and client_secret
    let client_id = Uuid::new_v4().to_string();
    let client_secret = generate_token(32);
    let client_secret_hash = ApiKeyService::hash_key(&client_secret);

    let data = CreateOAuthClient {
        client_id: client_id.clone(),
        client_secret_hash: Some(client_secret_hash),
        client_name: body.client_name.clone(),
        redirect_uris: body.redirect_uris.clone(),
        grant_types: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        token_endpoint_auth_method: "client_secret_basic".to_string(),
        workspace_id: Some(workspace_id),
        server_id: body.server_id,
        scopes: body.scopes.clone(),
        is_dynamic: false,
        software_id: None,
        software_version: None,
        // days -> seconds; omitted/null = no expiration
        access_token_ttl_seconds: body.expires_in_days.map(|d| d * 24 * 3600),
    };

    let client = OAuthClientRepository::create(&state.db, data)
        .await
        .map_err(db_error)?;

    Ok(Json(to_oauth_client_response(client, Some(client_secret))))
}

/// Get OAuth client for a specific server
pub async fn get_server_oauth(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<OAuthClientResponse>, (StatusCode, String)> {
    // Verify user is a member of this workspace
    let _ = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    let client = OAuthClientRepository::find_by_server_id(&state.db, server_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, "OAuth client not found for this server".to_string()))?;

    // Verify client belongs to workspace
    if client.workspace_id != Some(workspace_id) {
        return Err((StatusCode::NOT_FOUND, "OAuth client not found".to_string()));
    }

    Ok(Json(to_oauth_client_response(client, None)))
}

/// Regenerate OAuth client secret for a server
pub async fn regenerate_server_oauth_secret(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<OAuthClientResponse>, (StatusCode, String)> {
    // Verify user is a member of this workspace with sufficient permissions
    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    // Only Admin and Owner can regenerate OAuth secrets
    if matches!(member.role(), WorkspaceRole::Viewer) {
        return Err((StatusCode::FORBIDDEN, ERR_INSUFFICIENT_PERMISSIONS.to_string()));
    }

    let client = OAuthClientRepository::find_by_server_id(&state.db, server_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, "OAuth client not found for this server".to_string()))?;

    // Verify client belongs to workspace
    if client.workspace_id != Some(workspace_id) {
        return Err((StatusCode::NOT_FOUND, "OAuth client not found".to_string()));
    }

    // Generate new secret
    let new_secret = generate_token(32);
    let new_secret_hash = ApiKeyService::hash_key(&new_secret);

    // Update the secret
    OAuthClientRepository::update_secret(&state.db, client.id, &new_secret_hash)
        .await
        .map_err(db_error)?;

    // Revoke all existing tokens
    let _ = OAuthAccessTokenRepository::revoke_by_client(&state.db, client.id).await;
    let _ = OAuthRefreshTokenRepository::revoke_by_client(&state.db, client.id).await;

    Ok(Json(to_oauth_client_response(client, Some(new_secret))))
}

/// Delete an OAuth client
pub async fn delete_client(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, client_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Verify user is a member of this workspace with sufficient permissions
    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    // Only Admin and Owner can delete OAuth clients
    if matches!(member.role(), WorkspaceRole::Viewer) {
        return Err((StatusCode::FORBIDDEN, ERR_INSUFFICIENT_PERMISSIONS.to_string()));
    }

    // Verify client belongs to workspace
    let client = OAuthClientRepository::find_by_id(&state.db, client_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, "Client not found".to_string()))?;

    if client.workspace_id != Some(workspace_id) {
        return Err((StatusCode::NOT_FOUND, "Client not found".to_string()));
    }

    // Revoke all tokens for this client
    let _ = OAuthAccessTokenRepository::revoke_by_client(&state.db, client_id).await;
    let _ = OAuthRefreshTokenRepository::revoke_by_client(&state.db, client_id).await;

    OAuthClientRepository::delete(&state.db, client_id)
        .await
        .map_err(db_error)?;

    Ok(StatusCode::NO_CONTENT)
}

// ====================
// OAuth 2.1 Authorization Server Endpoints
// ====================

/// Authorization Server Metadata (RFC 8414)
/// GET /.well-known/oauth-authorization-server
#[derive(Debug, Serialize)]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

pub async fn authorization_server_metadata(
    State(state): State<Arc<AppState>>,
) -> Json<AuthorizationServerMetadata> {
    tracing::info!("OAuth authorization server metadata requested");

    // Use API_URL environment variable, fallback to constructing from server config
    let issuer = std::env::var("API_URL").unwrap_or_else(|_| {
        format!("http://{}:{}", state.config.server.host, state.config.server.port)
    });

    Json(AuthorizationServerMetadata {
        issuer: issuer.clone(),
        authorization_endpoint: format!("{}/oauth/authorize", issuer),
        token_endpoint: format!("{}/oauth/token", issuer),
        registration_endpoint: format!("{}/oauth/register", issuer),
        response_types_supported: vec!["code".to_string()],
        grant_types_supported: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        code_challenge_methods_supported: vec!["S256".to_string()],
        token_endpoint_auth_methods_supported: vec![
            "client_secret_basic".to_string(),
            "client_secret_post".to_string(),
            "none".to_string(),
        ],
    })
}

// ====================
// Dynamic Client Registration (RFC 7591)
// ====================

/// Client Registration Request (RFC 7591)
#[derive(Debug, Deserialize)]
pub struct ClientRegistrationRequest {
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default = "default_token_endpoint_auth_method")]
    pub token_endpoint_auth_method: String,
    #[serde(default = "default_grant_types_dcr")]
    pub grant_types: Vec<String>,
    #[serde(default = "default_response_types")]
    pub response_types: Vec<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub software_id: Option<String>,
    #[serde(default)]
    pub software_version: Option<String>,
}

fn default_token_endpoint_auth_method() -> String {
    "none".to_string()
}

fn default_grant_types_dcr() -> Vec<String> {
    vec!["authorization_code".to_string()]
}

fn default_response_types() -> Vec<String> {
    vec!["code".to_string()]
}

/// Client Registration Response (RFC 7591)
#[derive(Debug, Serialize)]
pub struct ClientRegistrationResponse {
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub client_id_issued_at: i64,
    pub client_secret_expires_at: i64,
    pub redirect_uris: Vec<String>,
    pub client_name: String,
    pub token_endpoint_auth_method: String,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software_version: Option<String>,
}

/// Client Registration Error Response (RFC 7591)
#[derive(Debug, Serialize)]
pub struct ClientRegistrationErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

/// Dynamic Client Registration endpoint (RFC 7591)
/// POST /oauth/register
pub async fn register_client(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ClientRegistrationRequest>,
) -> Result<Json<ClientRegistrationResponse>, (StatusCode, Json<ClientRegistrationErrorResponse>)> {
    // SECURITY: Rate limit OAuth client registration to prevent abuse
    let ip = crate::middleware::rate_limit::extract_client_ip(&headers, &addr);
    let rate_limit_key = format!("oauth_register_rate:{}", ip);

    // Allow max 10 registrations per hour per IP
    let lua_script = r#"
        local current = redis.call('INCR', KEYS[1])
        if current == 1 then
            redis.call('EXPIRE', KEYS[1], 3600)
        end
        return current
    "#;

    let result: Result<i64, _> = fred::interfaces::LuaInterface::eval(
        &state.redis,
        lua_script,
        vec![rate_limit_key],
        Vec::<String>::new(),
    )
    .await;

    if let Ok(count) = result {
        if count > 10 {
            tracing::warn!("OAuth registration rate limit exceeded for IP: {}", ip);
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(ClientRegistrationErrorResponse {
                    error: "rate_limit_exceeded".to_string(),
                    error_description: Some("Too many client registrations. Please try again later.".to_string()),
                }),
            ));
        }
    }

    tracing::info!(
        "OAuth dynamic client registration: redirect_uris={:?}, client_name={:?}",
        body.redirect_uris,
        body.client_name
    );

    // Validate redirect_uris
    if body.redirect_uris.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ClientRegistrationErrorResponse {
                error: "invalid_redirect_uri".to_string(),
                error_description: Some("At least one redirect_uri is required".to_string()),
            }),
        ));
    }

    // Load trusted redirect hosts from DB (replaces hardcoded allow-list)
    let trusted_hosts = mcp_db::OAuthTrustedRedirectHostRepository::list_enabled_hostnames(&state.db)
        .await
        .unwrap_or_default();

    // Validate redirect URIs (must be HTTPS or localhost or trusted hostname)
    for uri in &body.redirect_uris {
        if !is_valid_redirect_uri(uri, &trusted_hosts) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ClientRegistrationErrorResponse {
                    error: "invalid_redirect_uri".to_string(),
                    error_description: Some(format!("Invalid redirect_uri: {}. Must be HTTPS or localhost.", uri)),
                }),
            ));
        }
    }

    // Generate client_id
    let client_id = Uuid::new_v4().to_string();

    // For public clients (token_endpoint_auth_method = "none"), no secret is needed
    let (client_secret, client_secret_hash) = if body.token_endpoint_auth_method == "none" {
        (None, None)
    } else {
        let secret = generate_token(32);
        let hash = ApiKeyService::hash_key(&secret);
        (Some(secret), Some(hash))
    };

    let client_name = body.client_name.clone().unwrap_or_else(|| "Dynamic Client".to_string());

    // Parse scopes
    let scopes = body
        .scope
        .as_deref()
        .map(|s| s.split_whitespace().map(|s| s.to_string()).collect())
        .unwrap_or_else(|| vec!["*".to_string()]);

    let data = CreateOAuthClient {
        client_id: client_id.clone(),
        client_secret_hash,
        client_name: client_name.clone(),
        redirect_uris: body.redirect_uris.clone(),
        grant_types: body.grant_types.clone(),
        token_endpoint_auth_method: body.token_endpoint_auth_method.clone(),
        workspace_id: None, // Dynamic clients don't belong to a workspace
        server_id: None,
        scopes,
        is_dynamic: true,
        software_id: body.software_id.clone(),
        software_version: body.software_version.clone(),
        // Dynamically-registered clients (e.g. Claude) get a 30-day default so
        // long-lived connections aren't dropped hourly.
        access_token_ttl_seconds: Some(DEFAULT_ACCESS_TOKEN_TTL_SECONDS),
    };

    let created = OAuthClientRepository::create(&state.db, data)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create OAuth client: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ClientRegistrationErrorResponse {
                    error: "server_error".to_string(),
                    error_description: Some("Failed to register client".to_string()),
                }),
            )
        })?;

    let issued_at = created.created_at.timestamp();

    tracing::info!(
        "OAuth dynamic client registered: client_id={}, client_name={}",
        client_id,
        client_name
    );

    Ok(Json(ClientRegistrationResponse {
        client_id,
        client_secret,
        client_id_issued_at: issued_at,
        client_secret_expires_at: 0, // Never expires
        redirect_uris: body.redirect_uris,
        client_name,
        token_endpoint_auth_method: body.token_endpoint_auth_method,
        grant_types: body.grant_types,
        response_types: body.response_types,
        scope: body.scope,
        software_id: body.software_id,
        software_version: body.software_version,
    }))
}

/// Validate redirect URI (must be HTTPS, localhost, or a DB-configured trusted hostname).
fn is_valid_redirect_uri(uri: &str, trusted_hosts: &[String]) -> bool {
    let Ok(parsed) = Url::parse(uri) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or("");
    // Allow localhost (RFC 8252)
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return true;
    }
    // Allow DB-configured trusted hostnames over HTTPS
    if parsed.scheme() == "https" && trusted_hosts.iter().any(|h| h == host) {
        return true;
    }
    // All other URIs must be HTTPS
    parsed.scheme() == "https"
}

/// Authorization Request Parameters
#[derive(Debug, Deserialize)]
pub struct AuthorizeRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub code_challenge: String,
    pub code_challenge_method: Option<String>,
    pub scope: Option<String>,
}

/// Authorization endpoint - shows consent page or redirects to login
/// GET /oauth/authorize
pub async fn authorize(
    State(state): State<Arc<AppState>>,
    _auth_user: Option<AuthUser>,
    Query(params): Query<AuthorizeRequest>,
) -> Result<Response, (StatusCode, String)> {
    tracing::info!(
        "OAuth authorize request: client_id={}, redirect_uri={}, user_present={}",
        params.client_id,
        params.redirect_uri,
        _auth_user.is_some()
    );

    // Validate response_type
    if params.response_type != "code" {
        return Err((
            StatusCode::BAD_REQUEST,
            "unsupported_response_type".to_string(),
        ));
    }

    // Validate code_challenge_method
    let method = params.code_challenge_method.as_deref().unwrap_or("S256");
    if method != "S256" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Only S256 code_challenge_method is supported".to_string(),
        ));
    }

    // Find the OAuth client
    let client = OAuthClientRepository::find_by_client_id(&state.db, &params.client_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::BAD_REQUEST, "invalid_client".to_string()))?;

    // Validate redirect_uri
    if !client.is_redirect_uri_valid(&params.redirect_uri) {
        return Err((StatusCode::BAD_REQUEST, "invalid_redirect_uri".to_string()));
    }

    // Do NOT auto-issue a code. Redirect to the consent screen, which (after login if
    // needed) requires the user to explicitly approve this client before any authorization
    // code is issued. This is what prevents a malicious registered client from silently
    // obtaining a token for a logged-in victim via a cross-site navigation.
    let consent_url = format!(
        "{}/oauth/consent?response_type={}&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method={}&state={}&scope={}",
        state.config.server.frontend_url,
        urlencoding::encode(&params.response_type),
        urlencoding::encode(&params.client_id),
        urlencoding::encode(&params.redirect_uri),
        urlencoding::encode(&params.code_challenge),
        urlencoding::encode(method),
        urlencoding::encode(params.state.as_deref().unwrap_or("")),
        urlencoding::encode(params.scope.as_deref().unwrap_or("")),
    );
    Ok(Redirect::temporary(&consent_url).into_response())
}

/// Token Request
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

/// Token Response
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub refresh_token: Option<String>,
    pub scope: String,
}

/// Token endpoint - exchange code for tokens
/// POST /oauth/token
/// Accepts application/x-www-form-urlencoded as per OAuth 2.0 RFC 6749
pub async fn token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(mut body): Form<TokenRequest>,
) -> Result<Json<TokenResponse>, (StatusCode, Json<TokenErrorResponse>)> {
    // RFC 6749 client_secret_basic: fold HTTP Basic client credentials into the body so
    // the grant handlers see a single representation (client_secret_post uses the body
    // fields directly). Only fills gaps — an explicit body value wins.
    if let Some((basic_id, basic_secret)) = parse_basic_client_auth(&headers) {
        body.client_id.get_or_insert(basic_id);
        body.client_secret.get_or_insert(basic_secret);
    }

    tracing::info!(
        "OAuth token request: grant_type={}, client_id={:?}",
        body.grant_type,
        body.client_id
    );

    match body.grant_type.as_str() {
        "authorization_code" => exchange_code(state, body).await,
        "refresh_token" => refresh_access_token(state, body).await,
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "unsupported_grant_type".to_string(),
                error_description: Some("Only authorization_code and refresh_token are supported".to_string()),
            }),
        )),
    }
}

#[derive(Debug, Serialize)]
pub struct TokenErrorResponse {
    pub error: String,
    pub error_description: Option<String>,
}

/// Parse `Authorization: Basic <base64(client_id:client_secret)>` (RFC 6749 §2.3.1).
fn parse_basic_client_auth(headers: &HeaderMap) -> Option<(String, String)> {
    use base64::engine::general_purpose::STANDARD;
    let raw = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let b64 = raw
        .strip_prefix("Basic ")
        .or_else(|| raw.strip_prefix("basic "))?;
    let decoded = STANDARD.decode(b64.trim()).ok()?;
    let creds = String::from_utf8(decoded).ok()?;
    let (id, secret) = creds.split_once(':')?;
    // Credentials are application/x-www-form-urlencoded per the spec.
    let id = urlencoding::decode(id).ok()?.into_owned();
    let secret = urlencoding::decode(secret).ok()?.into_owned();
    Some((id, secret))
}

/// Authenticate the client at the token endpoint (RFC 6749 §3.2.1).
/// - A presented `client_id` (if any) must match the client the grant was issued to.
/// - Confidential clients (`token_endpoint_auth_method != "none"`) must present a valid
///   `client_secret`. Public clients (PKCE-only, e.g. dynamically-registered MCP clients
///   like Claude) have auth method "none" and are unaffected — PKCE is their protection.
fn verify_client_auth(
    client: &OAuthClient,
    presented_client_id: Option<&str>,
    presented_secret: Option<&str>,
) -> Result<(), (StatusCode, Json<TokenErrorResponse>)> {
    if let Some(presented) = presented_client_id {
        if presented != client.client_id {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(TokenErrorResponse {
                    error: "invalid_grant".to_string(),
                    error_description: Some("client_id does not match the grant".to_string()),
                }),
            ));
        }
    }

    if client.token_endpoint_auth_method != "none" {
        if let Some(hash) = client.client_secret_hash.as_deref() {
            let ok = presented_secret
                .map(|s| ApiKeyService::verify(s, hash))
                .unwrap_or(false);
            if !ok {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(TokenErrorResponse {
                        error: "invalid_client".to_string(),
                        error_description: Some("Invalid client credentials".to_string()),
                    }),
                ));
            }
        }
    }
    Ok(())
}

async fn exchange_code(
    state: Arc<AppState>,
    body: TokenRequest,
) -> Result<Json<TokenResponse>, (StatusCode, Json<TokenErrorResponse>)> {
    tracing::info!(
        "OAuth token exchange request: client_id={:?}, redirect_uri={:?}",
        body.client_id,
        body.redirect_uri
    );

    let code = body.code.ok_or((
        StatusCode::BAD_REQUEST,
        Json(TokenErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("code is required".to_string()),
        }),
    ))?;

    let code_verifier = body.code_verifier.ok_or((
        StatusCode::BAD_REQUEST,
        Json(TokenErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("code_verifier is required".to_string()),
        }),
    ))?;

    let redirect_uri = body.redirect_uri.ok_or((
        StatusCode::BAD_REQUEST,
        Json(TokenErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("redirect_uri is required".to_string()),
        }),
    ))?;

    // Find the authorization code
    let code_hash = ApiKeyService::hash_key(&code);
    let auth_code = OAuthAuthorizationCodeRepository::find_by_code_hash(&state.db, &code_hash)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TokenErrorResponse {
                    error: "server_error".to_string(),
                    error_description: None,
                }),
            )
        })?
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("Invalid or expired authorization code".to_string()),
            }),
        ))?;

    // Check if code is expired or used
    if auth_code.is_expired() || auth_code.is_used() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("Authorization code has expired or been used".to_string()),
            }),
        ));
    }

    // Verify redirect_uri matches
    if auth_code.redirect_uri != redirect_uri {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("redirect_uri does not match".to_string()),
            }),
        ));
    }

    // Verify PKCE code_verifier
    let computed_challenge = compute_code_challenge(&code_verifier);
    if computed_challenge != auth_code.code_challenge {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("Invalid code_verifier".to_string()),
            }),
        ));
    }

    // Authenticate the client the code was issued to (client_id match + client_secret for
    // confidential clients) BEFORE consuming the code.
    let client = OAuthClientRepository::find_by_id(&state.db, auth_code.client_id)
        .await
        .ok()
        .flatten()
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_client".to_string(),
                error_description: Some("Unknown client".to_string()),
            }),
        ))?;
    verify_client_auth(&client, body.client_id.as_deref(), body.client_secret.as_deref())?;

    // Mark code as used
    let _ = OAuthAuthorizationCodeRepository::mark_as_used(&state.db, auth_code.id).await;

    // Access token lifetime comes from the client's configured TTL (default 30d).
    let client_ttl = client.access_token_ttl_seconds;
    let (access_expires_at, access_expires_in) = access_token_expiry(client_ttl);

    // Generate access token
    let access_token = generate_token(32);
    let access_token_hash = ApiKeyService::hash_key(&access_token);

    let scopes = auth_code.scopes();

    let access_token_data = CreateOAuthAccessToken {
        token_hash: access_token_hash,
        client_id: auth_code.client_id,
        user_id: auth_code.user_id,
        scopes: scopes.clone(),
        expires_at: access_expires_at,
    };

    let created_access_token = OAuthAccessTokenRepository::create(&state.db, access_token_data)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TokenErrorResponse {
                    error: "server_error".to_string(),
                    error_description: None,
                }),
            )
        })?;

    // Generate refresh token
    let refresh_token = generate_token(32);
    let refresh_token_hash = ApiKeyService::hash_key(&refresh_token);
    let refresh_expires_at = Utc::now() + Duration::days(REFRESH_TOKEN_EXPIRES_DAYS);

    let refresh_token_data = CreateOAuthRefreshToken {
        token_hash: refresh_token_hash,
        client_id: auth_code.client_id,
        user_id: auth_code.user_id,
        scopes: scopes.clone(),
        access_token_id: Some(created_access_token.id),
        expires_at: refresh_expires_at,
    };

    OAuthRefreshTokenRepository::create(&state.db, refresh_token_data)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TokenErrorResponse {
                    error: "server_error".to_string(),
                    error_description: None,
                }),
            )
        })?;

    tracing::info!(
        "OAuth token exchange success: client_id={:?}, user_id={}",
        body.client_id,
        auth_code.user_id
    );

    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: access_expires_in,
        refresh_token: Some(refresh_token),
        scope: scopes.join(" "),
    }))
}

async fn refresh_access_token(
    state: Arc<AppState>,
    body: TokenRequest,
) -> Result<Json<TokenResponse>, (StatusCode, Json<TokenErrorResponse>)> {
    let refresh_token = body.refresh_token.ok_or((
        StatusCode::BAD_REQUEST,
        Json(TokenErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("refresh_token is required".to_string()),
        }),
    ))?;

    // Find the refresh token
    let token_hash = ApiKeyService::hash_key(&refresh_token);
    let stored_token = OAuthRefreshTokenRepository::find_by_token_hash(&state.db, &token_hash)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TokenErrorResponse {
                    error: "server_error".to_string(),
                    error_description: None,
                }),
            )
        })?
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("Invalid refresh token".to_string()),
            }),
        ))?;

    if !stored_token.is_valid() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_grant".to_string(),
                error_description: Some("Refresh token has expired or been revoked".to_string()),
            }),
        ));
    }

    // Authenticate the client the refresh token belongs to (client_id match +
    // client_secret for confidential clients).
    let client = OAuthClientRepository::find_by_id(&state.db, stored_token.client_id)
        .await
        .ok()
        .flatten()
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(TokenErrorResponse {
                error: "invalid_client".to_string(),
                error_description: Some("Unknown client".to_string()),
            }),
        ))?;
    verify_client_auth(&client, body.client_id.as_deref(), body.client_secret.as_deref())?;

    // Revoke old access token if it exists
    if let Some(access_token_id) = stored_token.access_token_id {
        let _ = OAuthAccessTokenRepository::revoke(&state.db, access_token_id).await;
    }

    // Access token lifetime comes from the client's configured TTL (default 30d).
    let client_ttl = client.access_token_ttl_seconds;
    let (access_expires_at, access_expires_in) = access_token_expiry(client_ttl);

    // Generate new access token
    let access_token = generate_token(32);
    let access_token_hash = ApiKeyService::hash_key(&access_token);

    let scopes = stored_token.scopes();

    let access_token_data = CreateOAuthAccessToken {
        token_hash: access_token_hash,
        client_id: stored_token.client_id,
        user_id: stored_token.user_id,
        scopes: scopes.clone(),
        expires_at: access_expires_at,
    };

    OAuthAccessTokenRepository::create(&state.db, access_token_data)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TokenErrorResponse {
                    error: "server_error".to_string(),
                    error_description: None,
                }),
            )
        })?;

    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: access_expires_in,
        refresh_token: None, // Don't issue new refresh token on refresh
        scope: scopes.join(" "),
    }))
}

// ====================
// Helper Functions
// ====================

fn generate_token(bytes: usize) -> String {
    // Generate a cryptographically secure random token
    // Uses rand's thread_rng which is backed by the OS CSPRNG
    let mut rng = rand::thread_rng();
    let mut token_bytes = vec![0u8; bytes];
    rng.fill_bytes(&mut token_bytes);
    // Use URL-safe base64 encoding without padding for OAuth tokens
    URL_SAFE_NO_PAD.encode(&token_bytes)
}

fn compute_code_challenge(code_verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let result = hasher.finalize();
    URL_SAFE_NO_PAD.encode(&result)
}

// ====================
// Upstream OAuth (provider-agnostic)
// ====================

const UPSTREAM_STATE_PREFIX: &str = "upstream_oauth_state:";
const UPSTREAM_STATE_TTL_SECS: i64 = 600; // 10 minutes

#[derive(Debug, Deserialize)]
pub struct UpstreamAuthorizeRequest {
    pub server_id: Uuid,
}

/// Upstream token response (standard OAuth2 JSON body).
#[derive(Debug, Deserialize)]
struct UpstreamTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default = "default_expires_in")]
    expires_in: i64,
}

fn default_expires_in() -> i64 {
    // Providers that don't return expires_in (e.g. GitHub OAuth Apps) have non-expiring tokens.
    // Use 1 year as a safe ceiling so the refresh job effectively never runs for them.
    365 * 24 * 3600
}

/// Resolve OAuth2 credentials for a provider.
/// - Managed providers: reads `client_id`/`client_secret` from the DB row.
/// - Custom providers: reads per-server columns.
pub struct ProviderCredentials {
    pub authorization_url: String,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// Extra query params to append to the authorization URL (from `extra_auth_params` column).
    pub extra_auth_params: serde_json::Value,
    /// Whether a missing refresh token in the callback should be treated as an error.
    pub requires_refresh_token: bool,
}

pub async fn resolve_credentials(
    db: &mcp_db::DbPool,
    provider_name: &str,
    server: &mcp_db::McpServer,
) -> Result<ProviderCredentials, (StatusCode, String)> {
    use mcp_db::UpstreamOAuthProviderRepository;

    let record = UpstreamOAuthProviderRepository::find_by_name(db, provider_name)
        .await
        .map_err(db_error)?
        .ok_or_else(|| (StatusCode::BAD_REQUEST, format!("Unknown upstream OAuth provider: {}", provider_name)))?;

    if !record.is_enabled {
        return Err((StatusCode::BAD_REQUEST, format!("Provider '{}' is currently disabled", provider_name)));
    }

    if record.is_managed {
        // Prefer credentials stored in DB; fall back to env vars for backwards compatibility.
        let client_id = record.client_id.clone()
            .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, format!("client_id is not set for provider '{}'", provider_name)))?;
        let client_secret = record.client_secret.clone()
            .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, format!("client_secret is not set for provider '{}'", provider_name)))?;
        Ok(ProviderCredentials {
            authorization_url: record.authorization_url,
            token_url: record.token_url,
            client_id,
            client_secret,
            extra_auth_params: record.extra_auth_params,
            requires_refresh_token: record.requires_refresh_token,
        })
    } else {
        // Custom provider: credentials must be on the server record
        let authorization_url = server.upstream_oauth_authorization_url.clone()
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "Custom provider: authorization URL not set".to_string()))?;
        let token_url = server.upstream_oauth_token_url.clone()
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "Custom provider: token URL not set".to_string()))?;
        let client_id = server.upstream_oauth_client_id.clone()
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "Custom provider: client_id not set".to_string()))?;
        let client_secret = server.upstream_oauth_client_secret.clone()
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "Custom provider: client_secret not set".to_string()))?;
        Ok(ProviderCredentials {
            authorization_url,
            token_url,
            client_id,
            client_secret,
            extra_auth_params: serde_json::Value::Object(Default::default()),
            requires_refresh_token: false,
        })
    }
}

/// Exchange or refresh an OAuth2 code/token against a token endpoint.
/// Returns (access_token, refresh_token_if_any, expires_in_secs).
async fn do_token_request(
    http: &reqwest::Client,
    token_url: &str,
    params: &[(&str, &str)],
) -> anyhow::Result<(String, Option<String>, i64)> {
    let resp = http
        .post(token_url)
        .header("Accept", "application/json")
        .form(params)
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Token endpoint returned error: {}", body);
    }

    let token: UpstreamTokenResponse = resp.json().await?;
    Ok((token.access_token, token.refresh_token, token.expires_in))
}

/// List enabled upstream OAuth providers.
/// GET /api/v1/oauth/upstream-providers
pub async fn list_upstream_providers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<mcp_common::types::UpstreamOAuthProviderResponse>>, (StatusCode, String)> {
    use mcp_db::UpstreamOAuthProviderRepository;

    let providers = UpstreamOAuthProviderRepository::list_enabled(&state.db)
        .await
        .map_err(db_error)?;

    let response = providers
        .into_iter()
        .map(|p| mcp_common::types::UpstreamOAuthProviderResponse {
            name: p.name,
            display_name: p.display_name,
            authorization_url: p.authorization_url,
            token_url: p.token_url,
            default_scopes: p.default_scopes,
            is_managed: p.is_managed,
        })
        .collect();

    Ok(Json(response))
}

/// Initiate upstream OAuth for an MCP server.
/// The user must be authenticated and a member of the server's workspace.
/// GET /oauth/upstream-authorize?server_id=...
pub async fn upstream_authorize(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Query(params): Query<UpstreamAuthorizeRequest>,
) -> Result<Response, (StatusCode, String)> {
    let server = ServerRepository::find_by_id(&state.db, params.server_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_SERVER_NOT_FOUND.to_string()))?;

    let _ = WorkspaceRepository::get_member(&state.db, server.workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    let provider_name = server
        .upstream_oauth_provider
        .as_deref()
        .ok_or((StatusCode::BAD_REQUEST, "Server has no upstream OAuth provider configured".to_string()))?;

    let creds = resolve_credentials(&state.db, provider_name, &server).await?;

    // Store nonce in Redis so the callback can verify it
    let nonce = generate_token(24);
    let state_key = format!("{}{}", UPSTREAM_STATE_PREFIX, nonce);
    let state_value = serde_json::json!({
        "server_id": params.server_id,
        "user_id": auth_user.user_id,
        "provider": provider_name,
    })
    .to_string();

    fred::interfaces::KeysInterface::set::<(), _, _>(
        &state.redis,
        &state_key,
        state_value,
        Some(fred::types::Expiration::EX(UPSTREAM_STATE_TTL_SECS)),
        None,
        false,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Redis error: {}", e)))?;

    let api_url = std::env::var("API_URL")
        .unwrap_or_else(|_| format!("http://{}:{}", state.config.server.host, state.config.server.port));
    let callback_url = format!("{}/oauth/upstream-callback", api_url);

    let scopes = server
        .upstream_oauth_scopes
        .as_deref()
        .map(|s| s.join(" "))
        .unwrap_or_default();

    let callback_encoded: String = url::form_urlencoded::byte_serialize(callback_url.as_bytes()).collect();
    let scopes_encoded: String = url::form_urlencoded::byte_serialize(scopes.as_bytes()).collect();

    // Build authorization URL, appending any extra params configured for this provider.
    let mut redirect_url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
        creds.authorization_url, creds.client_id, callback_encoded, scopes_encoded, nonce
    );
    if let Some(obj) = creds.extra_auth_params.as_object() {
        for (key, val) in obj {
            if let Some(v) = val.as_str() {
                redirect_url.push('&');
                redirect_url.push_str(&urlencoding::encode(key));
                redirect_url.push('=');
                redirect_url.push_str(&urlencoding::encode(v));
            }
        }
    }

    Ok(Redirect::temporary(&redirect_url).into_response())
}

#[derive(Debug, Deserialize)]
pub struct UpstreamCallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Receive upstream OAuth callback.
/// GET /oauth/upstream-callback
pub async fn upstream_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<UpstreamCallbackParams>,
) -> Result<Response, (StatusCode, String)> {
    if let Some(err) = params.error {
        let frontend_url = state.config.server.frontend_url.clone();
        let redirect = format!("{}/dashboard?upstream_oauth_error={}", frontend_url, urlencoding::encode(&err));
        return Ok(Redirect::temporary(&redirect).into_response());
    }

    let code = params.code.ok_or((StatusCode::BAD_REQUEST, "Missing code".to_string()))?;
    let nonce = params.state.ok_or((StatusCode::BAD_REQUEST, "Missing state".to_string()))?;

    // Validate and consume nonce
    let state_key = format!("{}{}", UPSTREAM_STATE_PREFIX, nonce);
    let raw: Option<String> = fred::interfaces::KeysInterface::get(&state.redis, &state_key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Redis error: {}", e)))?;
    let state_json = raw.ok_or((StatusCode::BAD_REQUEST, "Invalid or expired state".to_string()))?;
    let _: () = fred::interfaces::KeysInterface::del(&state.redis, &state_key)
        .await
        .unwrap_or(());

    #[derive(serde::Deserialize)]
    struct StatePayload {
        server_id: Uuid,
        #[allow(dead_code)]
        user_id: Uuid,
        provider: String,
    }
    let payload: StatePayload = serde_json::from_str(&state_json)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Corrupt state payload".to_string()))?;

    let server = ServerRepository::find_by_id(&state.db, payload.server_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_SERVER_NOT_FOUND.to_string()))?;

    let creds = resolve_credentials(&state.db, &payload.provider, &server).await?;

    let api_url = std::env::var("API_URL")
        .unwrap_or_else(|_| format!("http://{}:{}", state.config.server.host, state.config.server.port));
    let callback_url = format!("{}/oauth/upstream-callback", api_url);

    let (access_token, refresh_token_opt, expires_in) = do_token_request(
        &state.http,
        &creds.token_url,
        &[
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", callback_url.as_str()),
            ("grant_type", "authorization_code"),
        ],
    )
    .await
    .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Token exchange failed: {}", e)))?;

    // Some providers (e.g. Google with offline access) always return a refresh token.
    // Treat a missing one as an error when `requires_refresh_token` is set on the provider.
    let refresh_token = match refresh_token_opt {
        Some(rt) => rt,
        None if creds.requires_refresh_token => {
            return Err((
                StatusCode::BAD_GATEWAY,
                format!(
                    "Provider '{}' did not return a refresh_token. \
                     Check the extra_auth_params configured for this provider.",
                    payload.provider
                ),
            ));
        }
        None => String::new(), // non-expiring token; store empty string
    };

    let expires_at = chrono::Utc::now().timestamp() + expires_in;
    let token_payload = serde_json::json!({
        "access_token": access_token,
        "refresh_token": refresh_token,
        "expires_at": expires_at,
    })
    .to_string();

    let redis_key = format!("mcp:oauth:{}", payload.server_id);
    let ttl = expires_in + 90 * 24 * 3600;
    fred::interfaces::KeysInterface::set::<(), _, _>(
        &state.redis,
        &redis_key,
        token_payload,
        Some(fred::types::Expiration::EX(ttl)),
        None,
        false,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Redis error: {}", e)))?;

    tracing::info!(
        "Upstream OAuth complete: server_id={}, provider={}",
        payload.server_id, payload.provider
    );

    let frontend_url = state.config.server.frontend_url.clone();
    let redirect = format!("{}/dashboard/servers/{}", frontend_url, payload.server_id);
    Ok(Redirect::temporary(&redirect).into_response())
}

/// Refresh a single upstream token.
/// Returns (new_access_token, new_refresh_token_if_rotated, expires_in_secs).
pub async fn refresh_upstream_token(
    http: &reqwest::Client,
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> anyhow::Result<(String, Option<String>, i64)> {
    if refresh_token.is_empty() {
        // Token never expires (e.g. GitHub OAuth App); nothing to refresh
        anyhow::bail!("no refresh_token stored; token does not expire");
    }
    do_token_request(
        http,
        token_url,
        &[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ],
    )
    .await
}

// ====================
// Frontend OAuth Authorization Code Generation
// ====================

/// Request to generate authorization code from frontend
#[derive(Debug, Deserialize)]
pub struct AuthorizeCodeRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    #[serde(default = "default_code_challenge_method")]
    pub code_challenge_method: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub state: String,
    #[serde(default = "default_scope")]
    pub scope: String,
}

fn default_code_challenge_method() -> String {
    "S256".to_string()
}

fn default_scope() -> String {
    "*".to_string()
}

#[derive(Debug, Serialize)]
pub struct AuthorizeCodeResponse {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct ClientInfoQuery {
    pub client_id: String,
}

#[derive(Debug, Serialize)]
pub struct ClientInfoResponse {
    pub client_id: String,
    pub client_name: String,
    pub scopes: Vec<String>,
}

/// Public, non-secret client info for the consent screen (app name + requested scopes).
/// GET /api/v1/oauth/client-info?client_id=...
pub async fn client_info(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ClientInfoQuery>,
) -> Result<Json<ClientInfoResponse>, (StatusCode, String)> {
    let client = OAuthClientRepository::find_by_client_id(&state.db, &q.client_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, "invalid_client".to_string()))?;
    let scopes = client.scopes();
    Ok(Json(ClientInfoResponse {
        client_id: client.client_id,
        client_name: client.client_name,
        scopes,
    }))
}

/// Generate authorization code for logged-in user (called from frontend)
/// POST /api/v1/oauth/authorize-code
pub async fn authorize_code(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Json(body): Json<AuthorizeCodeRequest>,
) -> Result<Json<AuthorizeCodeResponse>, (StatusCode, String)> {
    tracing::info!(
        "OAuth authorize-code request: client_id={}, redirect_uri={}, user_id={}",
        body.client_id,
        body.redirect_uri,
        auth_user.user_id
    );
    // Validate response_type
    if body.response_type != "code" {
        return Err((
            StatusCode::BAD_REQUEST,
            "unsupported_response_type".to_string(),
        ));
    }

    // Validate code_challenge_method
    if body.code_challenge_method != "S256" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Only S256 code_challenge_method is supported".to_string(),
        ));
    }

    // Find the OAuth client
    let client = OAuthClientRepository::find_by_client_id(&state.db, &body.client_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::BAD_REQUEST, "invalid_client".to_string()))?;

    // Validate redirect_uri
    if !client.is_redirect_uri_valid(&body.redirect_uri) {
        return Err((StatusCode::BAD_REQUEST, "invalid_redirect_uri".to_string()));
    }

    // Generate authorization code
    let code = generate_token(32);
    let code_hash = ApiKeyService::hash_key(&code);

    let requested_scopes: Vec<String> = body
        .scope
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    // SECURITY: Validate requested scopes against client's allowed scopes
    let client_scopes = client.scopes();
    let has_wildcard = client_scopes.contains(&"*".to_string());

    if !has_wildcard {
        for scope in &requested_scopes {
            if scope != "*" && !client_scopes.contains(scope) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("Scope '{}' is not authorized for this client", scope),
                ));
            }
        }
    }

    // Use validated scopes (or client's scopes if wildcard requested)
    let scopes = if requested_scopes.contains(&"*".to_string()) && !has_wildcard {
        client_scopes
    } else {
        requested_scopes
    };

    // Capture values for logging before they are moved
    let log_client_id = body.client_id.clone();
    let log_redirect_uri = body.redirect_uri.clone();

    let auth_code = CreateOAuthAuthorizationCode {
        code_hash,
        client_id: client.id,
        user_id: auth_user.user_id,
        redirect_uri: body.redirect_uri,
        scopes,
        code_challenge: body.code_challenge,
        code_challenge_method: body.code_challenge_method,
        expires_at: Utc::now() + Duration::minutes(AUTH_CODE_EXPIRES_MINUTES),
    };

    OAuthAuthorizationCodeRepository::create(&state.db, auth_code)
        .await
        .map_err(db_error)?;

    tracing::info!(
        "OAuth authorize-code success: client_id={}, redirect_uri={}, user_id={}",
        log_client_id,
        log_redirect_uri,
        auth_user.user_id
    );

    Ok(Json(AuthorizeCodeResponse { code }))
}

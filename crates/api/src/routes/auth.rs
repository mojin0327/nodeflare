use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use chrono::{Duration, Utc};
use fred::interfaces::KeysInterface;
use mcp_auth::{GitHubOAuth, GoogleOAuth, hash_password, verify_password};
use mcp_common::types::{AuthResponse, RefreshTokenRequest, UserResponse};
use mcp_db::{
    CreateLinkedGitHubAccount, CreateUserFromEmail, EmailVerificationTokenRepository,
    LinkedGitHubAccountRepository, TokenType, UserRepository, WorkspaceRepository,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::error::{db_error, internal_error};
use crate::extractors::AuthUser;
use crate::middleware::rate_limit::{
    clear_failed_attempts, extract_client_ip, get_lockout_remaining, is_ip_locked_out,
    record_failed_attempt,
};
use crate::routes::helpers::{build_auth_cookies, clear_auth_cookies, CSRF_TOKEN_TTL_SECS};
use crate::state::AppState;

const CSRF_TOKEN_PREFIX: &str = "csrf:oauth:";

/// SECURITY: Validate return_to URL to prevent open redirect attacks
/// Returns None if the URL is potentially malicious, otherwise returns sanitized URL
fn validate_return_to_url(return_to: &str, frontend_url: &str) -> Option<String> {
    let return_to = return_to.trim();

    // Empty or whitespace-only
    if return_to.is_empty() {
        return None;
    }

    // Block dangerous characters that could be used for attacks
    if return_to.contains('\0') || return_to.contains('\n') || return_to.contains('\r') {
        return None;
    }

    // Case 1: Relative path (must start with single /)
    // Block // at start (protocol-relative URLs) and javascript: schemes
    if return_to.starts_with('/') && !return_to.starts_with("//") {
        // Additional check: no javascript, data, or vbscript schemes embedded
        let lower = return_to.to_lowercase();
        if lower.contains("javascript:") || lower.contains("data:") || lower.contains("vbscript:") {
            return None;
        }
        return Some(return_to.to_string());
    }

    // Case 2: Absolute URL - must match frontend origin
    if let Ok(return_url) = url::Url::parse(return_to) {
        if let Ok(frontend) = url::Url::parse(frontend_url) {
            // Must have same scheme and host
            if return_url.scheme() == frontend.scheme()
                && return_url.host_str() == frontend.host_str()
                && return_url.port() == frontend.port()
            {
                // Return just the path and query, not the full URL
                let path = return_url.path();
                if let Some(query) = return_url.query() {
                    return Some(format!("{}?{}", path, query));
                }
                return Some(path.to_string());
            }
        }
    }

    // Reject all other URLs
    None
}

#[derive(Debug, Deserialize)]
pub struct GitHubCallbackQuery {
    pub code: String,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GitHubLoginQuery {
    pub return_to: Option<String>,
}

pub async fn github_login(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GitHubLoginQuery>,
) -> impl IntoResponse {
    let redirect_url = if state.config.github.redirect_uri.is_empty() {
        format!(
            "{}://{}:{}/api/v1/auth/github/callback",
            if state.config.is_production() { "https" } else { "http" },
            state.config.server.host,
            state.config.server.port
        )
    } else {
        state.config.github.redirect_uri.clone()
    };

    match GitHubOAuth::from_config(&state.config, &redirect_url) {
        Ok(oauth) => {
            let (auth_url, csrf_token) = oauth.get_authorization_url();

            // Store CSRF token in Redis with TTL for validation on callback
            // Also store return_to if provided (for OAuth flow preservation)
            let csrf_key = format!("{}{}", CSRF_TOKEN_PREFIX, csrf_token);
            // SECURITY: Validate return_to URL to prevent open redirect attacks
            let validated_return_to = params.return_to.as_ref().and_then(|url| {
                validate_return_to_url(url, &state.config.server.frontend_url)
            });
            let redis_value = if let Some(ref return_to) = validated_return_to {
                // Store validated return_to along with the marker, separated by |
                format!("1|{}", return_to)
            } else {
                "1".to_string()
            };

            if let Err(e) = state
                .redis
                .set::<(), _, _>(
                    &csrf_key,
                    &redis_value,
                    Some(fred::types::Expiration::EX(CSRF_TOKEN_TTL_SECS)),
                    None,
                    false,
                )
                .await
            {
                tracing::error!("Failed to store CSRF token: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to initiate OAuth").into_response();
            }

            Redirect::temporary(&auth_url).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create GitHub OAuth client: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "OAuth configuration error").into_response()
        }
    }
}

const AUTH_BRUTE_FORCE_PREFIX: &str = "bf:auth:";

pub async fn github_callback(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<GitHubCallbackQuery>,
) -> Result<Response, (StatusCode, String)> {
    let ip = extract_client_ip(&headers, &addr);

    // Check if IP is locked out due to brute force
    if is_ip_locked_out(&state.redis, &ip, AUTH_BRUTE_FORCE_PREFIX).await {
        let remaining = get_lockout_remaining(&state.redis, &ip, AUTH_BRUTE_FORCE_PREFIX)
            .await
            .unwrap_or(0);
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "Too many failed attempts. Please try again in {} seconds.",
                remaining
            ),
        ));
    }

    // Validate CSRF token (state parameter)
    let csrf_state = query.state.as_ref().ok_or_else(|| {
        // Record failed attempt for missing state
        let redis = state.redis.clone();
        let ip_clone = ip.clone();
        tokio::spawn(async move {
            record_failed_attempt(&redis, &ip_clone, AUTH_BRUTE_FORCE_PREFIX).await;
        });
        (StatusCode::BAD_REQUEST, "Missing state parameter".to_string())
    })?;

    let csrf_key = format!("{}{}", CSRF_TOKEN_PREFIX, csrf_state);
    let csrf_value: Option<String> = state
        .redis
        .get(&csrf_key)
        .await
        .map_err(|e| internal_error("Redis CSRF check failed", e))?;

    if csrf_value.is_none() {
        // Record failed attempt for invalid/expired state
        record_failed_attempt(&state.redis, &ip, AUTH_BRUTE_FORCE_PREFIX).await;
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid or expired state parameter".to_string(),
        ));
    }

    // Delete used CSRF token (one-time use)
    let _ = state.redis.del::<(), _>(&csrf_key).await;

    // Check if this is a "link account" flow (format: "link|user_id|return_to")
    let csrf_value_str = csrf_value.as_ref().unwrap();
    if csrf_value_str.starts_with("link|") {
        return handle_github_link_callback(state, query, csrf_value_str, &ip).await;
    }

    // Extract return_to from Redis value if present (format: "1|return_to_url")
    let return_to: Option<String> = csrf_value.as_ref().and_then(|v| {
        if let Some(idx) = v.find('|') {
            Some(v[idx + 1..].to_string())
        } else {
            None
        }
    });

    let redirect_url = if state.config.github.redirect_uri.is_empty() {
        format!(
            "{}://{}:{}/api/v1/auth/github/callback",
            if state.config.is_production() { "https" } else { "http" },
            state.config.server.host,
            state.config.server.port
        )
    } else {
        state.config.github.redirect_uri.clone()
    };

    let oauth = GitHubOAuth::from_config(&state.config, &redirect_url)
        .map_err(|e| internal_error("OAuth initialization failed", e))?;

    // Exchange code for access token
    let access_token = oauth
        .exchange_code(&query.code)
        .await
        .map_err(|e| {
            tracing::error!("GitHub code exchange failed: {}", e);
            (StatusCode::BAD_REQUEST, "GitHub authentication failed".to_string())
        })?;

    // Get user info from GitHub
    let github_user = oauth
        .get_user(&access_token)
        .await
        .map_err(|e| {
            tracing::error!("GitHub get user failed: {}", e);
            (StatusCode::BAD_REQUEST, "Failed to get user info from GitHub".to_string())
        })?;

    // Get primary email if not provided
    let email = match &github_user.email {
        Some(e) if !e.is_empty() => e.clone(),
        _ => {
            oauth
                .get_primary_email(&access_token)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| format!("{}@users.noreply.github.com", github_user.id))
        }
    };

    // SECURITY: Don't log PII (email addresses) - only log non-sensitive identifiers
    tracing::info!("GitHub user authenticated: id={}, login={}", github_user.id, github_user.login);

    // Upsert user in database
    let user = UserRepository::upsert_from_github(
        &state.db,
        github_user.id,
        &email,
        &github_user.name.clone().unwrap_or(github_user.login.clone()),
        github_user.avatar_url.as_deref(),
    )
    .await
    .map_err(|e| internal_error("User upsert failed", e))?;

    // Encrypt and store GitHub access token
    let (encrypted_token, nonce) = state
        .crypto
        .encrypt_string(&access_token)
        .map_err(|e| internal_error("Token encryption failed", e))?;

    UserRepository::update_github_token(&state.db, user.id, &encrypted_token, &nonce)
        .await
        .map_err(|e| internal_error("Update GitHub token failed", e))?;

    // Check if user has any workspaces, if not create a personal one
    let workspaces = WorkspaceRepository::list_by_user(&state.db, user.id)
        .await
        .map_err(db_error)?;

    let workspace_id = if workspaces.is_empty() {
        // Create personal workspace - use first 8 chars of UUID (before first dash)
        let user_id_str = user.id.to_string();
        let user_id_prefix = user_id_str.split('-').next().unwrap_or(&user_id_str[..8.min(user_id_str.len())]);
        let ws = WorkspaceRepository::create(
            &state.db,
            mcp_db::CreateWorkspace {
                name: format!("{}'s Workspace", user.name),
                slug: format!("user-{}", user_id_prefix),
                owner_id: user.id,
            },
        )
        .await
        .map_err(|e| internal_error("Create workspace failed", e))?;
        Some(ws.id)
    } else {
        Some(workspaces[0].id)
    };

    // Generate JWT
    let access_token = state
        .jwt
        .generate_token(user.id, workspace_id)
        .map_err(|e| internal_error("JWT generation failed", e))?;

    // Generate refresh token
    let refresh = mcp_auth::jwt::RefreshToken::generate(
        user.id,
        state.config.auth.refresh_token_expiration_days,
    )
    .map_err(|e| {
        tracing::error!("Refresh token generation failed: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, "Token generation failed".to_string())
    })?;

    // Store refresh token hash in database
    sqlx::query(
        "INSERT INTO refresh_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(user.id)
    .bind(refresh.hash())
    .bind(refresh.expires_at)
    .execute(&state.db)
    .await
    .map_err(db_error)?;

    // Set tokens as HTTP-only secure cookies
    let is_production = state.config.is_production();
    let _cookie_domain = extract_domain(&state.config.server.frontend_url);
    let access_token_max_age = state.config.auth.jwt_expiration_hours * 3600;
    let refresh_token_max_age = state.config.auth.refresh_token_expiration_days * 24 * 3600;

    let (access_cookie, refresh_cookie) = build_auth_cookies(
        is_production, &access_token, access_token_max_age, &refresh.token, refresh_token_max_age,
    );

    // Clear failed attempts on successful login
    clear_failed_attempts(&state.redis, &ip, AUTH_BRUTE_FORCE_PREFIX).await;

    // Build frontend callback URL, preserving return_to if present
    let frontend_callback_url = if let Some(ref return_to_url) = return_to {
        format!(
            "{}/auth/callback?provider=github&return_to={}",
            state.config.server.frontend_url,
            urlencoding::encode(return_to_url)
        )
    } else {
        format!("{}/auth/callback?provider=github", state.config.server.frontend_url)
    };

    let mut headers = HeaderMap::new();
    // SECURITY: Handle potential invalid header values instead of panicking
    if let Ok(val) = HeaderValue::from_str(&access_cookie) {
        headers.insert(header::SET_COOKIE, val);
    } else {
        tracing::error!("Failed to create access cookie header");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Cookie generation failed".to_string()));
    }
    if let Ok(val) = HeaderValue::from_str(&refresh_cookie) {
        headers.append(header::SET_COOKIE, val);
    } else {
        tracing::error!("Failed to create refresh cookie header");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Cookie generation failed".to_string()));
    }
    if let Ok(val) = HeaderValue::from_str(&frontend_callback_url) {
        headers.insert(header::LOCATION, val);
    } else {
        tracing::error!("Failed to create location header");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Redirect URL generation failed".to_string()));
    }

    Ok((StatusCode::TEMPORARY_REDIRECT, headers, ()).into_response())
}

/// Handle GitHub OAuth callback for account linking flow
/// Called when csrf_value starts with "link|" (format: "link|user_id|return_to")
async fn handle_github_link_callback(
    state: Arc<AppState>,
    query: GitHubCallbackQuery,
    csrf_value: &str,
    _ip: &str,
) -> Result<Response, (StatusCode, String)> {
    // Parse "link|user_id|return_to" format
    let parts: Vec<&str> = csrf_value.splitn(3, '|').collect();
    if parts.len() < 2 {
        return Err((StatusCode::BAD_REQUEST, "Invalid state data".to_string()));
    }

    let user_id = uuid::Uuid::parse_str(parts[1]).map_err(|_| {
        (StatusCode::BAD_REQUEST, "Invalid user ID in state".to_string())
    })?;
    let return_to = parts.get(2).map(|s| s.to_string()).filter(|s| !s.is_empty());

    // Build redirect URL for token exchange
    let redirect_url = if state.config.github.redirect_uri.is_empty() {
        format!(
            "{}://{}:{}/api/v1/auth/github/callback",
            if state.config.is_production() { "https" } else { "http" },
            state.config.server.host,
            state.config.server.port
        )
    } else {
        state.config.github.redirect_uri.clone()
    };

    let oauth = GitHubOAuth::from_config(&state.config, &redirect_url)
        .map_err(|e| internal_error("OAuth initialization failed", e))?;

    // Exchange code for access token
    let access_token = oauth
        .exchange_code(&query.code)
        .await
        .map_err(|e| {
            tracing::error!("GitHub code exchange failed for link: {}", e);
            (StatusCode::BAD_REQUEST, "GitHub authentication failed".to_string())
        })?;

    // Get GitHub user info
    let github_user = oauth
        .get_user(&access_token)
        .await
        .map_err(|e| {
            tracing::error!("GitHub get user failed for link: {}", e);
            (StatusCode::BAD_REQUEST, "Failed to get GitHub user info".to_string())
        })?;

    // Check if this GitHub account is already linked to this user
    if let Some(existing) = LinkedGitHubAccountRepository::find_by_github_id(
        &state.db,
        user_id,
        github_user.id,
    )
    .await
    .map_err(db_error)?
    {
        // Update the token for existing linked account
        let encrypted = state
            .crypto
            .encrypt_string(&access_token)
            .map_err(|e| internal_error("Token encryption failed", e))?;

        LinkedGitHubAccountRepository::update_token(
            &state.db,
            existing.id,
            user_id,
            &encrypted.0,
            &encrypted.1,
            Some("repo,user:email"),
        )
        .await
        .map_err(db_error)?;

        tracing::info!(
            "Updated GitHub account token: user={}, github_user={}",
            user_id,
            github_user.login
        );
    } else {
        // Create new linked account
        let encrypted = state
            .crypto
            .encrypt_string(&access_token)
            .map_err(|e| internal_error("Token encryption failed", e))?;

        LinkedGitHubAccountRepository::create(
            &state.db,
            CreateLinkedGitHubAccount {
                user_id,
                github_id: github_user.id,
                github_username: github_user.login.clone(),
                github_avatar_url: github_user.avatar_url.clone(),
                access_token_encrypted: encrypted.0,
                access_token_nonce: encrypted.1,
                scopes: Some("repo,user:email".to_string()),
            },
        )
        .await
        .map_err(db_error)?;

        tracing::info!(
            "Linked GitHub account: user={}, github_user={}",
            user_id,
            github_user.login
        );
    }

    // Redirect to frontend.
    // SECURITY: `return_to` came from the client (stored unvalidated in the link CSRF
    // value), so validate it the same way the login flow does before concatenating —
    // otherwise `return_to=.evil.com` (no separator) redirects to an attacker host.
    let validated_return_to =
        return_to.as_deref().and_then(|rt| validate_return_to_url(rt, &state.config.server.frontend_url));
    let redirect_url = if let Some(return_to) = validated_return_to {
        format!("{}{}?success=github_linked", state.config.server.frontend_url, return_to)
    } else {
        format!("{}/dashboard/settings/github?success=github_linked", state.config.server.frontend_url)
    };

    Ok(Redirect::temporary(&redirect_url).into_response())
}

/// Extract domain from URL for cookie domain setting
fn extract_domain(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("localhost")
        .split(':')
        .next()
        .unwrap_or("localhost")
        .to_string()
}

const REFRESH_BRUTE_FORCE_PREFIX: &str = "bf:refresh:";

/// Extract refresh token from cookie header
fn extract_refresh_token_from_cookies(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .find_map(|cookie| {
                    let cookie = cookie.trim();
                    if cookie.starts_with("refresh_token=") {
                        Some(cookie.trim_start_matches("refresh_token=").to_string())
                    } else {
                        None
                    }
                })
        })
}

pub async fn refresh_token(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Option<Json<RefreshTokenRequest>>,
) -> Result<Response, (StatusCode, String)> {
    let ip = extract_client_ip(&headers, &addr);

    // Check if IP is locked out due to brute force
    if is_ip_locked_out(&state.redis, &ip, REFRESH_BRUTE_FORCE_PREFIX).await {
        let remaining = get_lockout_remaining(&state.redis, &ip, REFRESH_BRUTE_FORCE_PREFIX)
            .await
            .unwrap_or(0);
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "Too many failed attempts. Please try again in {} seconds.",
                remaining
            ),
        ));
    }

    // Try to get refresh token from cookie first, then fallback to JSON body
    let refresh_token = extract_refresh_token_from_cookies(&headers)
        .or_else(|| body.map(|b| b.refresh_token.clone()))
        .ok_or_else(|| {
            (StatusCode::BAD_REQUEST, "Refresh token required".to_string())
        })?;

    let token_hash = mcp_auth::jwt::hash_token(&refresh_token);

    // Find refresh token (including created_at for absolute lifetime check)
    let record: Option<(uuid::Uuid, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT user_id, expires_at, created_at FROM refresh_tokens WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(db_error)?;

    let (user_id, expires_at, created_at) = record.ok_or_else(|| {
        // Record failed attempt for invalid token
        let redis = state.redis.clone();
        let ip_clone = ip.clone();
        tokio::spawn(async move {
            record_failed_attempt(&redis, &ip_clone, REFRESH_BRUTE_FORCE_PREFIX).await;
        });
        (StatusCode::UNAUTHORIZED, "Invalid refresh token".to_string())
    })?;

    // Check expiration
    if expires_at < chrono::Utc::now() {
        // Delete expired token
        sqlx::query("DELETE FROM refresh_tokens WHERE token_hash = $1")
            .bind(&token_hash)
            .execute(&state.db)
            .await
            .ok();
        // Record failed attempt for expired token
        record_failed_attempt(&state.redis, &ip, REFRESH_BRUTE_FORCE_PREFIX).await;
        return Err((StatusCode::UNAUTHORIZED, "Refresh token expired".to_string()));
    }

    // SECURITY: Check absolute session lifetime limit
    // This prevents sessions from being extended indefinitely via sliding window
    let absolute_expiry = created_at + chrono::Duration::days(state.config.auth.absolute_session_lifetime_days);
    if chrono::Utc::now() > absolute_expiry {
        // Delete token that exceeded absolute lifetime
        sqlx::query("DELETE FROM refresh_tokens WHERE token_hash = $1")
            .bind(&token_hash)
            .execute(&state.db)
            .await
            .ok();
        tracing::info!(
            "Session exceeded absolute lifetime limit for user {}",
            user_id
        );
        return Err((
            StatusCode::UNAUTHORIZED,
            "Session expired. Please log in again.".to_string(),
        ));
    }

    // Clear failed attempts on successful token validation
    clear_failed_attempts(&state.redis, &ip, REFRESH_BRUTE_FORCE_PREFIX).await;

    // Get user
    let user = UserRepository::find_by_id(&state.db, user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    // Get user's workspaces
    let workspaces = WorkspaceRepository::list_by_user(&state.db, user.id)
        .await
        .map_err(db_error)?;

    let workspace_id = workspaces.first().map(|w| w.id);

    // Generate new tokens
    let new_access_token = state
        .jwt
        .generate_token(user.id, workspace_id)
        .map_err(|e| internal_error("JWT generation failed", e))?;

    // Generate new refresh token with fresh expiration (sliding window)
    // This extends the session by another 14 days from now
    let new_refresh = mcp_auth::jwt::RefreshToken::generate(
        user.id,
        state.config.auth.refresh_token_expiration_days,
    )
    .map_err(|e| internal_error("Token generation failed", e))?;

    // Delete old refresh token and insert new one (token rotation for security)
    sqlx::query("DELETE FROM refresh_tokens WHERE token_hash = $1")
        .bind(&token_hash)
        .execute(&state.db)
        .await
        .ok();

    sqlx::query(
        "INSERT INTO refresh_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(user.id)
    .bind(new_refresh.hash())
    .bind(new_refresh.expires_at)
    .execute(&state.db)
    .await
    .map_err(db_error)?;

    // Build response with both JSON body and Set-Cookie headers
    let is_production = state.config.is_production();
    let access_token_max_age = state.config.auth.jwt_expiration_hours * 3600;
    let refresh_token_max_age = state.config.auth.refresh_token_expiration_days * 24 * 3600;

    let (access_cookie, refresh_cookie) = build_auth_cookies(
        is_production, &new_access_token, access_token_max_age, &new_refresh.token, refresh_token_max_age,
    );

    // SECURITY: Only include refresh_token in JSON if explicitly requested via header.
    // This prevents XSS attacks from stealing refresh tokens in web apps.
    // API clients (CLI, native apps) that can't use cookies should set this header.
    let include_refresh_in_json = headers
        .get("x-include-refresh-token")
        .map(|v| v.to_str().unwrap_or("") == "true")
        .unwrap_or(false);

    let auth_response = AuthResponse {
        access_token: new_access_token.clone(),
        refresh_token: if include_refresh_in_json {
            Some(new_refresh.token.clone())
        } else {
            None // Web apps should use HttpOnly cookie instead
        },
        token_type: "Bearer".to_string(),
        expires_in: state.config.auth.jwt_expiration_hours * 3600,
        user: UserResponse {
            id: user.id,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            created_at: user.created_at,
        },
    };

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&access_cookie) {
        response_headers.insert(header::SET_COOKIE, val);
    }
    if let Ok(val) = HeaderValue::from_str(&refresh_cookie) {
        response_headers.append(header::SET_COOKIE, val);
    }

    Ok((StatusCode::OK, response_headers, Json(auth_response)).into_response())
}

pub async fn get_current_user(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<Json<UserResponse>, (StatusCode, String)> {
    let user = UserRepository::find_by_id(&state.db, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    Ok(Json(UserResponse {
        id: user.id,
        email: user.email,
        name: user.name,
        avatar_url: user.avatar_url,
        created_at: user.created_at,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub name: Option<String>,
}

pub async fn update_profile(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Json(body): Json<UpdateProfileRequest>,
) -> Result<Json<UserResponse>, (StatusCode, String)> {
    let user = UserRepository::find_by_id(&state.db, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let name = body.name.unwrap_or(user.name.clone());

    if name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name cannot be empty".to_string()));
    }

    if name.len() > 100 {
        return Err((StatusCode::BAD_REQUEST, "Name too long".to_string()));
    }

    let updated_user = UserRepository::update_name(&state.db, auth_user.user_id, &name)
        .await
        .map_err(db_error)?;

    Ok(Json(UserResponse {
        id: updated_user.id,
        email: updated_user.email,
        name: updated_user.name,
        avatar_url: updated_user.avatar_url,
        created_at: updated_user.created_at,
    }))
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> impl IntoResponse {
    // Delete all refresh tokens for this user
    let _ = sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
        .bind(auth_user.user_id)
        .execute(&state.db)
        .await;

    // SECURITY: Revoke all access tokens for this user.
    // The revocation timestamp is stored in Redis and checked during token verification.
    // TTL matches the maximum token lifetime to auto-cleanup.
    let token_ttl_secs = state.config.auth.jwt_expiration_hours * 3600;
    if let Err(e) = mcp_auth::revoke_all_user_tokens(
        &state.redis,
        &auth_user.user_id.to_string(),
        token_ttl_secs,
    )
    .await
    {
        tracing::warn!("Failed to revoke user tokens in Redis: {}", e);
        // Continue with logout even if revocation fails
    }

    tracing::info!("User {} logged out", auth_user.user_id);

    // Clear cookies by setting them with expired max-age
    let (clear_access_cookie, clear_refresh_cookie) = clear_auth_cookies(state.config.is_production());

    let mut headers = HeaderMap::new();
    // SECURITY: Handle potential invalid header values instead of panicking
    if let Ok(val) = HeaderValue::from_str(&clear_access_cookie) {
        headers.insert(header::SET_COOKIE, val);
    }
    if let Ok(val) = HeaderValue::from_str(&clear_refresh_cookie) {
        headers.append(header::SET_COOKIE, val);
    }

    (StatusCode::NO_CONTENT, headers, ())
}

pub async fn delete_account(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    // Get all workspaces where user is owner
    let owned_workspaces = WorkspaceRepository::list_owned_by_user(&state.db, auth_user.user_id)
        .await
        .map_err(db_error)?;

    // Delete owned workspaces and all their resources (servers, deployments, etc.)
    for workspace in owned_workspaces {
        WorkspaceRepository::delete(&state.db, workspace.id)
            .await
            .map_err(db_error)?;
    }

    // Remove user from other workspaces where they are a member
    let member_workspaces = WorkspaceRepository::list_by_user(&state.db, auth_user.user_id)
        .await
        .map_err(db_error)?;

    for workspace in member_workspaces {
        WorkspaceRepository::remove_member(&state.db, workspace.id, auth_user.user_id)
            .await
            .ok(); // Ignore errors - best effort cleanup
    }

    // Delete refresh tokens
    sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
        .bind(auth_user.user_id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;

    // Delete user
    UserRepository::delete(&state.db, auth_user.user_id)
        .await
        .map_err(db_error)?;

    tracing::info!("User {} deleted their account", auth_user.user_id);

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Serialize)]
pub struct WsTokenResponse {
    pub token: String,
}

/// Generate a token for WebSocket authentication (for cross-origin connections)
pub async fn ws_token(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<Json<WsTokenResponse>, (StatusCode, String)> {
    // Get user's workspaces
    let workspaces = WorkspaceRepository::list_by_user(&state.db, auth_user.user_id)
        .await
        .map_err(db_error)?;

    let workspace_id = workspaces.first().map(|w| w.id);

    // Generate a short-lived token for WebSocket connections
    let token = state
        .jwt
        .generate_token(auth_user.user_id, workspace_id)
        .map_err(|e| internal_error("Token generation failed", e))?;

    Ok(Json(WsTokenResponse { token }))
}

// ============================================================================
// Google OAuth Routes
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct GoogleLoginQuery {
    pub return_to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GoogleCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

pub async fn google_login(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GoogleLoginQuery>,
) -> impl IntoResponse {
    let redirect_url = if state.config.google.redirect_uri.is_empty() {
        format!(
            "{}://{}:{}/api/v1/auth/google/callback",
            if state.config.is_production() { "https" } else { "http" },
            state.config.server.host,
            state.config.server.port
        )
    } else {
        state.config.google.redirect_uri.clone()
    };

    match GoogleOAuth::new(&state.config, &redirect_url) {
        Ok(oauth) => {
            let (auth_url, csrf_token) = oauth.get_authorization_url();

            // Store CSRF token in Redis with TTL for validation on callback
            let csrf_key = format!("{}{}", CSRF_TOKEN_PREFIX, csrf_token);
            let validated_return_to = params.return_to.as_ref().and_then(|url| {
                validate_return_to_url(url, &state.config.server.frontend_url)
            });
            let redis_value = if let Some(ref return_to) = validated_return_to {
                format!("1|{}", return_to)
            } else {
                "1".to_string()
            };

            if let Err(e) = state
                .redis
                .set::<(), _, _>(
                    &csrf_key,
                    &redis_value,
                    Some(fred::types::Expiration::EX(CSRF_TOKEN_TTL_SECS)),
                    None,
                    false,
                )
                .await
            {
                tracing::error!("Failed to store CSRF token: {}", e);
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to initiate OAuth").into_response();
            }

            Redirect::temporary(&auth_url).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create Google OAuth client: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Google OAuth not configured").into_response()
        }
    }
}

pub async fn google_callback(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<GoogleCallbackQuery>,
) -> Result<Response, (StatusCode, String)> {
    let ip = extract_client_ip(&headers, &addr);

    // Handle OAuth errors (e.g., user cancelled)
    if let Some(error) = &query.error {
        let error_url = format!(
            "{}/login?error={}",
            state.config.server.frontend_url,
            urlencoding::encode(error)
        );
        return Ok(Redirect::temporary(&error_url).into_response());
    }

    // Check if code is present
    let code = query.code.as_ref().ok_or_else(|| {
        (StatusCode::BAD_REQUEST, "Missing authorization code".to_string())
    })?;

    // Check if IP is locked out due to brute force
    if is_ip_locked_out(&state.redis, &ip, AUTH_BRUTE_FORCE_PREFIX).await {
        let remaining = get_lockout_remaining(&state.redis, &ip, AUTH_BRUTE_FORCE_PREFIX)
            .await
            .unwrap_or(0);
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "Too many failed attempts. Please try again in {} seconds.",
                remaining
            ),
        ));
    }

    // Validate CSRF token (state parameter)
    let csrf_state = query.state.as_ref().ok_or_else(|| {
        let redis = state.redis.clone();
        let ip_clone = ip.clone();
        tokio::spawn(async move {
            record_failed_attempt(&redis, &ip_clone, AUTH_BRUTE_FORCE_PREFIX).await;
        });
        (StatusCode::BAD_REQUEST, "Missing state parameter".to_string())
    })?;

    let csrf_key = format!("{}{}", CSRF_TOKEN_PREFIX, csrf_state);
    let csrf_value: Option<String> = state
        .redis
        .get(&csrf_key)
        .await
        .map_err(|e| internal_error("Redis CSRF check failed", e))?;

    if csrf_value.is_none() {
        record_failed_attempt(&state.redis, &ip, AUTH_BRUTE_FORCE_PREFIX).await;
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid or expired state parameter".to_string(),
        ));
    }

    // Extract return_to from Redis value if present
    let return_to: Option<String> = csrf_value.as_ref().and_then(|v| {
        if let Some(idx) = v.find('|') {
            Some(v[idx + 1..].to_string())
        } else {
            None
        }
    });

    // Delete used CSRF token
    let _ = state.redis.del::<(), _>(&csrf_key).await;

    let redirect_url = if state.config.google.redirect_uri.is_empty() {
        format!(
            "{}://{}:{}/api/v1/auth/google/callback",
            if state.config.is_production() { "https" } else { "http" },
            state.config.server.host,
            state.config.server.port
        )
    } else {
        state.config.google.redirect_uri.clone()
    };

    let oauth = GoogleOAuth::new(&state.config, &redirect_url)
        .map_err(|e| internal_error("OAuth initialization failed", e))?;

    // Exchange code for access token
    let access_token = oauth
        .exchange_code(code)
        .await
        .map_err(|e| {
            tracing::error!("Google code exchange failed: {}", e);
            (StatusCode::BAD_REQUEST, "Google authentication failed".to_string())
        })?;

    // Get user info from Google
    let google_user = oauth
        .get_user(&access_token)
        .await
        .map_err(|e| {
            tracing::error!("Google get user failed: {}", e);
            (StatusCode::BAD_REQUEST, "Failed to get user info from Google".to_string())
        })?;

    // Get email
    let email = google_user.email.clone().ok_or_else(|| {
        (StatusCode::BAD_REQUEST, "Email not available from Google".to_string())
    })?;

    // Check if email is verified by Google
    if google_user.email_verified != Some(true) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Google email is not verified".to_string(),
        ));
    }

    tracing::info!("Google user authenticated: sub={}", google_user.sub);

    // Upsert user in database
    let user = UserRepository::upsert_from_google(
        &state.db,
        &google_user.sub,
        &email,
        &google_user.name.clone().unwrap_or_else(|| email.split('@').next().unwrap_or("User").to_string()),
        google_user.picture.as_deref(),
    )
    .await
    .map_err(|e| internal_error("User upsert failed", e))?;

    // Check if user has any workspaces, if not create a personal one
    let workspaces = WorkspaceRepository::list_by_user(&state.db, user.id)
        .await
        .map_err(db_error)?;

    let workspace_id = if workspaces.is_empty() {
        let user_id_str = user.id.to_string();
        let user_id_prefix = user_id_str.split('-').next().unwrap_or(&user_id_str[..8.min(user_id_str.len())]);
        let ws = WorkspaceRepository::create(
            &state.db,
            mcp_db::CreateWorkspace {
                name: format!("{}'s Workspace", user.name),
                slug: format!("user-{}", user_id_prefix),
                owner_id: user.id,
            },
        )
        .await
        .map_err(|e| internal_error("Create workspace failed", e))?;
        Some(ws.id)
    } else {
        Some(workspaces[0].id)
    };

    // Generate JWT
    let jwt_access_token = state
        .jwt
        .generate_token(user.id, workspace_id)
        .map_err(|e| internal_error("JWT generation failed", e))?;

    // Generate refresh token
    let refresh = mcp_auth::jwt::RefreshToken::generate(
        user.id,
        state.config.auth.refresh_token_expiration_days,
    )
    .map_err(|e| {
        tracing::error!("Refresh token generation failed: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, "Token generation failed".to_string())
    })?;

    // Store refresh token hash in database
    sqlx::query(
        "INSERT INTO refresh_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(user.id)
    .bind(refresh.hash())
    .bind(refresh.expires_at)
    .execute(&state.db)
    .await
    .map_err(db_error)?;

    // Set tokens as HTTP-only secure cookies
    let is_production = state.config.is_production();
    let access_token_max_age = state.config.auth.jwt_expiration_hours * 3600;
    let refresh_token_max_age = state.config.auth.refresh_token_expiration_days * 24 * 3600;

    let (access_cookie, refresh_cookie) = build_auth_cookies(
        is_production, &jwt_access_token, access_token_max_age, &refresh.token, refresh_token_max_age,
    );

    // Clear failed attempts on successful login
    clear_failed_attempts(&state.redis, &ip, AUTH_BRUTE_FORCE_PREFIX).await;

    // Build frontend callback URL
    let frontend_callback_url = if let Some(ref return_to_url) = return_to {
        format!(
            "{}/auth/callback?provider=google&return_to={}",
            state.config.server.frontend_url,
            urlencoding::encode(return_to_url)
        )
    } else {
        format!("{}/auth/callback?provider=google", state.config.server.frontend_url)
    };

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&access_cookie) {
        response_headers.insert(header::SET_COOKIE, val);
    } else {
        tracing::error!("Failed to create access cookie header");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Cookie generation failed".to_string()));
    }
    if let Ok(val) = HeaderValue::from_str(&refresh_cookie) {
        response_headers.append(header::SET_COOKIE, val);
    } else {
        tracing::error!("Failed to create refresh cookie header");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Cookie generation failed".to_string()));
    }
    if let Ok(val) = HeaderValue::from_str(&frontend_callback_url) {
        response_headers.insert(header::LOCATION, val);
    } else {
        tracing::error!("Failed to create location header");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Redirect URL generation failed".to_string()));
    }

    Ok((StatusCode::TEMPORARY_REDIRECT, response_headers, ()).into_response())
}

// ============================================================================
// Email/Password Authentication Routes
// ============================================================================

const EMAIL_REGISTER_RATE_LIMIT_PREFIX: &str = "rl:register:";
const EMAIL_LOGIN_RATE_LIMIT_PREFIX: &str = "rl:login:";
const PASSWORD_RESET_RATE_LIMIT_PREFIX: &str = "rl:reset:";

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub message: String,
    pub email: String,
}

/// Register a new user with email and password
pub async fn register(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, (StatusCode, String)> {
    let ip = extract_client_ip(&headers, &addr);

    // Rate limit registration (3 per hour per IP)
    if is_ip_locked_out(&state.redis, &ip, EMAIL_REGISTER_RATE_LIMIT_PREFIX).await {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many registration attempts. Please try again later.".to_string(),
        ));
    }

    // Validate email format
    if !body.email.contains('@') || body.email.len() > 255 {
        return Err((StatusCode::BAD_REQUEST, "Invalid email address".to_string()));
    }

    // Validate password strength
    if body.password.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, "Password must be at least 8 characters".to_string()));
    }

    if body.password.len() > 128 {
        return Err((StatusCode::BAD_REQUEST, "Password too long".to_string()));
    }

    // Validate name
    let name = body.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err((StatusCode::BAD_REQUEST, "Invalid name".to_string()));
    }

    // Check if email already exists
    if let Some(existing) = UserRepository::find_by_email(&state.db, &body.email)
        .await
        .map_err(db_error)?
    {
        // If user exists but is NOT verified, resend verification email
        if !existing.email_verified {
            tracing::info!("User exists but unverified, resending verification email to {}", body.email);

            // Delete existing verification tokens for this user
            EmailVerificationTokenRepository::delete_by_type_for_user(
                &state.db,
                existing.id,
                TokenType::EmailVerification,
            )
            .await
            .ok();

            // Generate new verification token
            let verification_token = mcp_auth::jwt::generate_random_token(32)
                .map_err(|e| internal_error("Token generation failed", e))?;
            let token_hash = mcp_auth::jwt::hash_token(&verification_token);
            let expires_at = Utc::now() + Duration::hours(24);

            // Store verification token
            EmailVerificationTokenRepository::create(
                &state.db,
                existing.id,
                &token_hash,
                TokenType::EmailVerification,
                expires_at,
            )
            .await
            .map_err(db_error)?;

            // Send verification email
            tracing::info!("Attempting to send verification email to {}", body.email);
            match mcp_email::EmailService::from_env() {
                Ok(email_service) => {
                    let verification_url = format!(
                        "{}/verify-email?token={}",
                        state.config.server.frontend_url,
                        verification_token
                    );
                    tracing::info!("Email service initialized, sending to {}", body.email);

                    match email_service
                        .send_email_verification(&body.email, &verification_url)
                        .await
                    {
                        Ok(email_id) => {
                            tracing::info!("Verification email sent successfully: id={}", email_id);
                        }
                        Err(e) => {
                            tracing::error!("Failed to send verification email: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Email service not configured: {:?}", e);
                }
            }

            return Ok(Json(RegisterResponse {
                message: "Registration successful. Please check your email to verify your account.".to_string(),
                email: body.email,
            }));
        }

        // User exists and is verified - don't reveal this
        record_failed_attempt(&state.redis, &ip, EMAIL_REGISTER_RATE_LIMIT_PREFIX).await;
        return Ok(Json(RegisterResponse {
            message: "If this email is not already registered, a verification email will be sent.".to_string(),
            email: body.email,
        }));
    }

    // Hash password
    let password_hash = hash_password(&body.password)
        .map_err(|e| internal_error("Password hashing failed", e))?;

    // Create user (unverified)
    let user = UserRepository::create_from_email(
        &state.db,
        CreateUserFromEmail {
            email: body.email.clone(),
            name: name.to_string(),
            password_hash,
        },
    )
    .await
    .map_err(|e| {
        // Could be a race condition - email already exists
        tracing::warn!("Failed to create user: {}", e);
        (StatusCode::BAD_REQUEST, "Registration failed".to_string())
    })?;

    // Generate verification token
    let verification_token = mcp_auth::jwt::generate_random_token(32)
        .map_err(|e| internal_error("Token generation failed", e))?;
    let token_hash = mcp_auth::jwt::hash_token(&verification_token);
    let expires_at = Utc::now() + Duration::hours(24);

    // Store verification token
    EmailVerificationTokenRepository::create(
        &state.db,
        user.id,
        &token_hash,
        TokenType::EmailVerification,
        expires_at,
    )
    .await
    .map_err(db_error)?;

    // Send verification email
    tracing::info!("Attempting to send verification email to {}", body.email);
    match mcp_email::EmailService::from_env() {
        Ok(email_service) => {
            let verification_url = format!(
                "{}/verify-email?token={}",
                state.config.server.frontend_url,
                verification_token
            );
            tracing::info!("Email service initialized, sending to {}", body.email);

            match email_service
                .send_email_verification(&body.email, &verification_url)
                .await
            {
                Ok(email_id) => {
                    tracing::info!("Verification email sent successfully: id={}", email_id);
                }
                Err(e) => {
                    tracing::error!("Failed to send verification email: {}", e);
                }
            }
        }
        Err(e) => {
            tracing::error!("Email service not configured: {:?}", e);
        }
    }

    Ok(Json(RegisterResponse {
        message: "Registration successful. Please check your email to verify your account.".to_string(),
        email: body.email,
    }))
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Login with email and password
pub async fn login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<Response, (StatusCode, String)> {
    let ip = extract_client_ip(&headers, &addr);

    // Check if IP is locked out
    if is_ip_locked_out(&state.redis, &ip, EMAIL_LOGIN_RATE_LIMIT_PREFIX).await {
        let remaining = get_lockout_remaining(&state.redis, &ip, EMAIL_LOGIN_RATE_LIMIT_PREFIX)
            .await
            .unwrap_or(0);
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("Too many login attempts. Please try again in {} seconds.", remaining),
        ));
    }

    // Find user by email with password hash
    let user_with_token = UserRepository::get_for_email_login(&state.db, &body.email)
        .await
        .map_err(db_error)?;

    let user_with_token = match user_with_token {
        Some(u) => u,
        None => {
            record_failed_attempt(&state.redis, &ip, EMAIL_LOGIN_RATE_LIMIT_PREFIX).await;
            return Err((StatusCode::UNAUTHORIZED, "Invalid email or password".to_string()));
        }
    };

    // Verify password
    let password_hash = user_with_token.password_hash.as_ref().ok_or_else(|| {
        (StatusCode::UNAUTHORIZED, "Invalid email or password".to_string())
    })?;

    let is_valid = verify_password(&body.password, password_hash)
        .map_err(|e| internal_error("Password verification failed", e))?;

    if !is_valid {
        record_failed_attempt(&state.redis, &ip, EMAIL_LOGIN_RATE_LIMIT_PREFIX).await;
        return Err((StatusCode::UNAUTHORIZED, "Invalid email or password".to_string()));
    }

    // Check if email is verified
    if !user_with_token.email_verified {
        return Err((
            StatusCode::FORBIDDEN,
            "Please verify your email address before logging in".to_string(),
        ));
    }

    // Clear failed attempts
    clear_failed_attempts(&state.redis, &ip, EMAIL_LOGIN_RATE_LIMIT_PREFIX).await;

    // Get workspaces
    let workspaces = WorkspaceRepository::list_by_user(&state.db, user_with_token.id)
        .await
        .map_err(db_error)?;

    let workspace_id = if workspaces.is_empty() {
        let user_id_str = user_with_token.id.to_string();
        let user_id_prefix = user_id_str.split('-').next().unwrap_or(&user_id_str[..8.min(user_id_str.len())]);
        let ws = WorkspaceRepository::create(
            &state.db,
            mcp_db::CreateWorkspace {
                name: format!("{}'s Workspace", user_with_token.name),
                slug: format!("user-{}", user_id_prefix),
                owner_id: user_with_token.id,
            },
        )
        .await
        .map_err(|e| internal_error("Create workspace failed", e))?;
        Some(ws.id)
    } else {
        Some(workspaces[0].id)
    };

    // Generate JWT
    let access_token = state
        .jwt
        .generate_token(user_with_token.id, workspace_id)
        .map_err(|e| internal_error("JWT generation failed", e))?;

    // Generate refresh token
    let refresh = mcp_auth::jwt::RefreshToken::generate(
        user_with_token.id,
        state.config.auth.refresh_token_expiration_days,
    )
    .map_err(|e| internal_error("Token generation failed", e))?;

    // Store refresh token hash
    sqlx::query(
        "INSERT INTO refresh_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(user_with_token.id)
    .bind(refresh.hash())
    .bind(refresh.expires_at)
    .execute(&state.db)
    .await
    .map_err(db_error)?;

    // Build response with cookies
    let is_production = state.config.is_production();
    let access_token_max_age = state.config.auth.jwt_expiration_hours * 3600;
    let refresh_token_max_age = state.config.auth.refresh_token_expiration_days * 24 * 3600;

    let (access_cookie, refresh_cookie) = build_auth_cookies(
        is_production, &access_token, access_token_max_age, &refresh.token, refresh_token_max_age,
    );

    let auth_response = AuthResponse {
        access_token: access_token.clone(),
        refresh_token: None,
        token_type: "Bearer".to_string(),
        expires_in: state.config.auth.jwt_expiration_hours * 3600,
        user: UserResponse {
            id: user_with_token.id,
            email: user_with_token.email,
            name: user_with_token.name,
            avatar_url: user_with_token.avatar_url,
            created_at: user_with_token.created_at,
        },
    };

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&access_cookie) {
        response_headers.insert(header::SET_COOKIE, val);
    }
    if let Ok(val) = HeaderValue::from_str(&refresh_cookie) {
        response_headers.append(header::SET_COOKIE, val);
    }

    Ok((StatusCode::OK, response_headers, Json(auth_response)).into_response())
}

#[derive(Debug, Deserialize)]
pub struct VerifyEmailQuery {
    pub token: String,
}

/// Verify email address with token and auto-login
pub async fn verify_email(
    State(state): State<Arc<AppState>>,
    Query(query): Query<VerifyEmailQuery>,
) -> Result<Response, (StatusCode, String)> {
    let token_hash = mcp_auth::jwt::hash_token(&query.token);

    // Find valid token
    let token = EmailVerificationTokenRepository::find_valid_token(&state.db, &token_hash)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::BAD_REQUEST, "Invalid or expired verification link".to_string()))?;

    // Check token type
    if token.token_type != TokenType::EmailVerification.to_string() {
        return Err((StatusCode::BAD_REQUEST, "Invalid token type".to_string()));
    }

    // Mark email as verified
    let user = UserRepository::verify_email(&state.db, token.user_id)
        .await
        .map_err(db_error)?;

    // Mark token as used
    EmailVerificationTokenRepository::mark_as_used(&state.db, token.id)
        .await
        .map_err(db_error)?;

    // Delete all verification tokens for this user
    EmailVerificationTokenRepository::delete_by_type_for_user(
        &state.db,
        token.user_id,
        TokenType::EmailVerification,
    )
    .await
    .ok();

    // Auto-login: Generate JWT and refresh token
    let workspaces = WorkspaceRepository::list_by_user(&state.db, user.id)
        .await
        .map_err(db_error)?;

    let workspace_id = if workspaces.is_empty() {
        let user_id_str = user.id.to_string();
        let user_id_prefix = user_id_str.split('-').next().unwrap_or(&user_id_str[..8.min(user_id_str.len())]);
        let ws = WorkspaceRepository::create(
            &state.db,
            mcp_db::CreateWorkspace {
                name: format!("{}'s Workspace", user.name),
                slug: format!("user-{}", user_id_prefix),
                owner_id: user.id,
            },
        )
        .await
        .map_err(|e| internal_error("Create workspace failed", e))?;
        Some(ws.id)
    } else {
        Some(workspaces[0].id)
    };

    // Generate JWT
    let access_token = state
        .jwt
        .generate_token(user.id, workspace_id)
        .map_err(|e| internal_error("JWT generation failed", e))?;

    // Generate refresh token
    let refresh = mcp_auth::jwt::RefreshToken::generate(
        user.id,
        state.config.auth.refresh_token_expiration_days,
    )
    .map_err(|e| internal_error("Token generation failed", e))?;

    // Store refresh token hash
    sqlx::query(
        "INSERT INTO refresh_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(user.id)
    .bind(refresh.hash())
    .bind(refresh.expires_at)
    .execute(&state.db)
    .await
    .map_err(db_error)?;

    // Build response with cookies
    let is_production = state.config.is_production();
    let access_token_max_age = state.config.auth.jwt_expiration_hours * 3600;
    let refresh_token_max_age = state.config.auth.refresh_token_expiration_days * 24 * 3600;

    let (access_cookie, refresh_cookie) = build_auth_cookies(
        is_production, &access_token, access_token_max_age, &refresh.token, refresh_token_max_age,
    );

    let auth_response = AuthResponse {
        access_token: access_token.clone(),
        refresh_token: None,
        token_type: "Bearer".to_string(),
        expires_in: state.config.auth.jwt_expiration_hours * 3600,
        user: UserResponse {
            id: user.id,
            email: user.email,
            name: user.name,
            avatar_url: user.avatar_url,
            created_at: user.created_at,
        },
    };

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&access_cookie) {
        response_headers.insert(header::SET_COOKIE, val);
    }
    if let Ok(val) = HeaderValue::from_str(&refresh_cookie) {
        response_headers.append(header::SET_COOKIE, val);
    }

    Ok((StatusCode::OK, response_headers, Json(auth_response)).into_response())
}

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct ForgotPasswordResponse {
    pub message: String,
}

/// Request password reset email
pub async fn forgot_password(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ForgotPasswordRequest>,
) -> Result<Json<ForgotPasswordResponse>, (StatusCode, String)> {
    let ip = extract_client_ip(&headers, &addr);

    // Rate limit password reset requests
    if is_ip_locked_out(&state.redis, &ip, PASSWORD_RESET_RATE_LIMIT_PREFIX).await {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many password reset requests. Please try again later.".to_string(),
        ));
    }

    // Always return success to prevent email enumeration
    let success_response = Json(ForgotPasswordResponse {
        message: "If an account exists with this email, a password reset link will be sent.".to_string(),
    });

    // Find user by email (must have password auth enabled)
    let user = match UserRepository::get_for_email_login(&state.db, &body.email).await {
        Ok(Some(u)) => u,
        _ => {
            record_failed_attempt(&state.redis, &ip, PASSWORD_RESET_RATE_LIMIT_PREFIX).await;
            return Ok(success_response);
        }
    };

    // Delete any existing password reset tokens for this user
    EmailVerificationTokenRepository::delete_by_type_for_user(
        &state.db,
        user.id,
        TokenType::PasswordReset,
    )
    .await
    .ok();

    // Generate reset token (1 hour validity)
    let reset_token = mcp_auth::jwt::generate_random_token(32)
        .map_err(|e| internal_error("Token generation failed", e))?;
    let token_hash = mcp_auth::jwt::hash_token(&reset_token);
    let expires_at = Utc::now() + Duration::hours(1);

    // Store reset token
    EmailVerificationTokenRepository::create(
        &state.db,
        user.id,
        &token_hash,
        TokenType::PasswordReset,
        expires_at,
    )
    .await
    .map_err(db_error)?;

    // Send password reset email
    if let Ok(email_service) = mcp_email::EmailService::from_env() {
        let reset_url = format!(
            "{}/reset-password?token={}",
            state.config.server.frontend_url,
            reset_token
        );

        if let Err(e) = email_service
            .send_password_reset(&body.email, &reset_url)
            .await
        {
            tracing::error!("Failed to send password reset email: {}", e);
        }
    } else {
        tracing::warn!("Email service not configured - password reset email not sent");
    }

    record_failed_attempt(&state.redis, &ip, PASSWORD_RESET_RATE_LIMIT_PREFIX).await;
    Ok(success_response)
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct ResetPasswordResponse {
    pub message: String,
}

/// Reset password with token
pub async fn reset_password(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ResetPasswordRequest>,
) -> Result<Json<ResetPasswordResponse>, (StatusCode, String)> {
    // Validate new password
    if body.password.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, "Password must be at least 8 characters".to_string()));
    }

    if body.password.len() > 128 {
        return Err((StatusCode::BAD_REQUEST, "Password too long".to_string()));
    }

    let token_hash = mcp_auth::jwt::hash_token(&body.token);

    // Find valid token
    let token = EmailVerificationTokenRepository::find_valid_token(&state.db, &token_hash)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::BAD_REQUEST, "Invalid or expired reset link".to_string()))?;

    // Check token type
    if token.token_type != TokenType::PasswordReset.to_string() {
        return Err((StatusCode::BAD_REQUEST, "Invalid token type".to_string()));
    }

    // Hash new password
    let password_hash = hash_password(&body.password)
        .map_err(|e| internal_error("Password hashing failed", e))?;

    // Update password
    UserRepository::update_password(&state.db, token.user_id, &password_hash)
        .await
        .map_err(db_error)?;

    // Mark token as used
    EmailVerificationTokenRepository::mark_as_used(&state.db, token.id)
        .await
        .map_err(db_error)?;

    // Delete all reset tokens for this user
    EmailVerificationTokenRepository::delete_by_type_for_user(
        &state.db,
        token.user_id,
        TokenType::PasswordReset,
    )
    .await
    .ok();

    // Invalidate all existing sessions for this user (security best practice)
    sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
        .bind(token.user_id)
        .execute(&state.db)
        .await
        .map_err(db_error)?;

    Ok(Json(ResetPasswordResponse {
        message: "Password reset successfully. Please log in with your new password.".to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ResendVerificationRequest {
    pub email: String,
}

/// Resend verification email
pub async fn resend_verification(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ResendVerificationRequest>,
) -> Result<Json<RegisterResponse>, (StatusCode, String)> {
    let ip = extract_client_ip(&headers, &addr);

    // Rate limit
    if is_ip_locked_out(&state.redis, &ip, EMAIL_REGISTER_RATE_LIMIT_PREFIX).await {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests. Please try again later.".to_string(),
        ));
    }

    // Find user by email
    let user = match UserRepository::find_by_email(&state.db, &body.email).await {
        Ok(Some(u)) => u,
        _ => {
            record_failed_attempt(&state.redis, &ip, EMAIL_REGISTER_RATE_LIMIT_PREFIX).await;
            // Don't reveal if email exists
            return Ok(Json(RegisterResponse {
                message: "If this email is registered and not verified, a verification email will be sent.".to_string(),
                email: body.email,
            }));
        }
    };

    // Check if already verified
    if user.email_verified {
        record_failed_attempt(&state.redis, &ip, EMAIL_REGISTER_RATE_LIMIT_PREFIX).await;
        return Ok(Json(RegisterResponse {
            message: "If this email is registered and not verified, a verification email will be sent.".to_string(),
            email: body.email,
        }));
    }

    // Delete existing verification tokens
    EmailVerificationTokenRepository::delete_by_type_for_user(
        &state.db,
        user.id,
        TokenType::EmailVerification,
    )
    .await
    .ok();

    // Generate new verification token
    let verification_token = mcp_auth::jwt::generate_random_token(32)
        .map_err(|e| internal_error("Token generation failed", e))?;
    let token_hash = mcp_auth::jwt::hash_token(&verification_token);
    let expires_at = Utc::now() + Duration::hours(24);

    // Store verification token
    EmailVerificationTokenRepository::create(
        &state.db,
        user.id,
        &token_hash,
        TokenType::EmailVerification,
        expires_at,
    )
    .await
    .map_err(db_error)?;

    // Send verification email
    if let Ok(email_service) = mcp_email::EmailService::from_env() {
        let verification_url = format!(
            "{}/verify-email?token={}",
            state.config.server.frontend_url,
            verification_token
        );

        if let Err(e) = email_service
            .send_email_verification(&body.email, &verification_url)
            .await
        {
            tracing::error!("Failed to send verification email: {}", e);
        }
    }

    record_failed_attempt(&state.redis, &ip, EMAIL_REGISTER_RATE_LIMIT_PREFIX).await;

    Ok(Json(RegisterResponse {
        message: "If this email is registered and not verified, a verification email will be sent.".to_string(),
        email: body.email,
    }))
}

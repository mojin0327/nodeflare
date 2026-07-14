use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use fred::interfaces::KeysInterface;
use mcp_auth::GitHubOAuth;
use mcp_db::{LinkedGitHubAccount, LinkedGitHubAccountRepository};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{db_error, internal_error};
use crate::extractors::AuthUser;
use crate::routes::helpers::CSRF_TOKEN_TTL_SECS;
use crate::state::AppState;

const CSRF_TOKEN_PREFIX: &str = "csrf:oauth:";

// ============================================================================
// Response Types
// ============================================================================

#[derive(Debug, Serialize)]
pub struct LinkedAccountResponse {
    pub id: Uuid,
    pub github_id: i64,
    pub github_username: String,
    pub github_avatar_url: Option<String>,
    pub is_primary: bool,
    pub created_at: String,
}

impl From<LinkedGitHubAccount> for LinkedAccountResponse {
    fn from(account: LinkedGitHubAccount) -> Self {
        Self {
            id: account.id,
            github_id: account.github_id,
            github_username: account.github_username,
            github_avatar_url: account.github_avatar_url,
            is_primary: account.is_primary,
            created_at: account.created_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// ============================================================================
// Query Parameters
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct LinkQuery {
    pub return_to: Option<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// List all linked GitHub accounts for the current user
pub async fn list_accounts(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<Json<Vec<LinkedAccountResponse>>, (StatusCode, String)> {
    let accounts = LinkedGitHubAccountRepository::list_by_user(&state.db, auth_user.user_id)
        .await
        .map_err(db_error)?;

    let response: Vec<LinkedAccountResponse> = accounts.into_iter().map(Into::into).collect();

    Ok(Json(response))
}

/// Initiate GitHub OAuth flow to link a new account
pub async fn link_account(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Query(params): Query<LinkQuery>,
) -> Result<Response, (StatusCode, String)> {
    // Generate CSRF token
    let csrf_token = mcp_auth::jwt::generate_random_token(32)
        .map_err(|e| internal_error("Token generation failed", e))?;

    // Store in Redis with user_id and optional return_to
    // Format: "link|user_id|return_to" to distinguish from login flow
    let csrf_key = format!("{}{}", CSRF_TOKEN_PREFIX, csrf_token);
    let redis_value = format!(
        "link|{}|{}",
        auth_user.user_id,
        params.return_to.unwrap_or_default()
    );

    state
        .redis
        .set::<(), _, _>(
            &csrf_key,
            &redis_value,
            Some(fred::types::Expiration::EX(CSRF_TOKEN_TTL_SECS)),
            None,
            false,
        )
        .await
        .map_err(|e| internal_error("Redis set failed", e))?;

    // Use the same callback URL as normal GitHub login
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

    let oauth = GitHubOAuth::new(
        &state.config.github.client_id,
        &state.config.github.client_secret,
        &redirect_url,
    );

    // Request repo scope for full repository access
    let scopes = "repo,user:email";
    let auth_url = oauth.get_authorization_url_with_state_and_scopes(&csrf_token, scopes);

    // Redirect to GitHub
    Ok(Redirect::temporary(&auth_url).into_response())
}

/// Delete a linked GitHub account
pub async fn unlink_account(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(account_id): Path<Uuid>,
) -> Result<Json<MessageResponse>, (StatusCode, String)> {
    let deleted = LinkedGitHubAccountRepository::delete(&state.db, account_id, auth_user.user_id)
        .await
        .map_err(db_error)?;

    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Account not found".to_string()));
    }

    tracing::info!(
        "Unlinked GitHub account: user={}, account={}",
        auth_user.user_id,
        account_id
    );

    Ok(Json(MessageResponse {
        message: "GitHub account unlinked successfully".to_string(),
    }))
}

/// Set a linked account as primary
pub async fn set_primary(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(account_id): Path<Uuid>,
) -> Result<Json<MessageResponse>, (StatusCode, String)> {
    let updated = LinkedGitHubAccountRepository::set_primary(&state.db, account_id, auth_user.user_id)
        .await
        .map_err(db_error)?;

    if !updated {
        return Err((StatusCode::NOT_FOUND, "Account not found".to_string()));
    }

    tracing::info!(
        "Set primary GitHub account: user={}, account={}",
        auth_user.user_id,
        account_id
    );

    Ok(Json(MessageResponse {
        message: "Primary account updated successfully".to_string(),
    }))
}

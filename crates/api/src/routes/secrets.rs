use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use mcp_common::types::{SecretResponse, SetSecretRequest};
use mcp_db::{CreateSecret, SecretRepository, ServerRepository, WorkspaceRepository};
use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::db_error;
use crate::extractors::AuthUser;
use crate::routes::helpers::{verify_server_ownership, ERR_INSUFFICIENT_PERMISSIONS, ERR_NOT_A_MEMBER};
use crate::state::AppState;

/// Regex for valid secret key names: must start with letter, contain only A-Z, 0-9, _
/// Max length 256 characters for safety
static SECRET_KEY_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[A-Za-z][A-Za-z0-9_]{0,255}$").unwrap()
});

/// Validate secret key name to prevent injection attacks
pub(crate) fn validate_secret_key(key: &str) -> Result<(), (StatusCode, String)> {
    if key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Secret key cannot be empty".to_string()));
    }

    if !SECRET_KEY_REGEX.is_match(key) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Secret key must start with a letter and contain only letters, numbers, and underscores (max 256 chars)".to_string(),
        ));
    }

    Ok(())
}



pub async fn list(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<SecretResponse>>, (StatusCode, String)> {
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    // Verify server belongs to workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    let secrets = SecretRepository::list_by_server(&state.db, server_id)
        .await
        .map_err(db_error)?;

    let response: Vec<SecretResponse> = secrets
        .into_iter()
        .map(|s| SecretResponse {
            key: s.key,
            created_at: s.created_at,
            updated_at: s.updated_at,
        })
        .collect();

    Ok(Json(response))
}

pub async fn set(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<SetSecretRequest>,
) -> Result<Json<SecretResponse>, (StatusCode, String)> {
    // Validate secret key name first (security: prevent injection attacks)
    validate_secret_key(&body.key)?;

    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    if matches!(member.role(), mcp_common::types::WorkspaceRole::Viewer) {
        return Err((StatusCode::FORBIDDEN, ERR_INSUFFICIENT_PERMISSIONS.to_string()));
    }

    // Verify server belongs to workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    // Encrypt the value
    let (encrypted_value, nonce) = state
        .crypto
        .encrypt_string(&body.value)
        .map_err(|e| {
            tracing::error!("Encryption failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Encryption failed".to_string())
        })?;

    let secret = SecretRepository::upsert(
        &state.db,
        CreateSecret {
            server_id,
            key: body.key,
            encrypted_value,
            nonce,
        },
    )
    .await
    .map_err(db_error)?;

    Ok(Json(SecretResponse {
        key: secret.key,
        created_at: secret.created_at,
        updated_at: secret.updated_at,
    }))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id, key)): Path<(Uuid, Uuid, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    if matches!(member.role(), mcp_common::types::WorkspaceRole::Viewer) {
        return Err((StatusCode::FORBIDDEN, ERR_INSUFFICIENT_PERMISSIONS.to_string()));
    }

    // Verify server belongs to workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    SecretRepository::delete_by_key(&state.db, server_id, &key)
        .await
        .map_err(db_error)?;

    Ok(StatusCode::NO_CONTENT)
}

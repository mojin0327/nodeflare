use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use mcp_auth::ApiKeyService;
use mcp_billing::Plan as BillingPlan;
use mcp_common::types::{ApiKeyCreatedResponse, ApiKeyResponse, CreateApiKeyRequest};
use mcp_db::{ApiKeyRepository, WorkspaceRepository};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::db_error;
use crate::extractors::{workspace, AuthUser};
use crate::state::AppState;
use crate::routes::helpers::ERR_WORKSPACE_NOT_FOUND;

pub async fn list(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<ApiKeyResponse>>, (StatusCode, String)> {
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let keys = ApiKeyRepository::list_by_workspace(&state.db, workspace_id)
        .await
        .map_err(db_error)?;

    let response: Vec<ApiKeyResponse> = keys
        .into_iter()
        .map(|k| {
            let scopes = k.scopes();
            ApiKeyResponse {
                id: k.id,
                name: k.name,
                key_prefix: k.key_prefix,
                scopes,
                server_id: k.server_id,
                last_used_at: k.last_used_at,
                expires_at: k.expires_at,
                created_at: k.created_at,
            }
        })
        .collect();

    Ok(Json(response))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyCreatedResponse>, (StatusCode, String)> {
    workspace::require_write_access(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    // Check API key limit based on plan
    let ws = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    let current_count = ApiKeyRepository::count_by_workspace(&state.db, workspace_id)
        .await
        .map_err(db_error)?;

    let limit = BillingPlan::from_str(&ws.plan).limits().max_api_keys as i64;
    if current_count >= limit {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "API key limit reached. Your {} plan allows up to {} API keys.",
                ws.plan, limit
            ),
        ));
    }

    // Require explicit scopes - no default to prevent over-permissioning
    // If no scopes provided, use minimal read-only scope
    let scopes = body.scopes.unwrap_or_else(|| vec!["tools:list".to_string()]);

    // Validate scopes
    let valid_scopes = [
        "*",
        "tools:list",
        "tools:call",
        "resources:list",
        "resources:read",
        "prompts:list",
        "prompts:get",
    ];
    for scope in &scopes {
        // Allow wildcard patterns like "tools:call:specific_tool"
        let base_scope = scope.split(':').take(2).collect::<Vec<_>>().join(":");
        if !valid_scopes.contains(&scope.as_str()) && !valid_scopes.contains(&base_scope.as_str()) && scope != "*" {
            return Err((StatusCode::BAD_REQUEST, format!("Invalid scope: {}", scope)));
        }
    }

    let (data, full_key) = ApiKeyService::create_api_key_data(
        workspace_id,
        body.server_id,
        body.name.clone(),
        scopes,
        body.expires_in_days,
    )
    .map_err(db_error)?;

    let key = ApiKeyRepository::create(&state.db, data)
        .await
        .map_err(db_error)?;

    Ok(Json(ApiKeyCreatedResponse {
        id: key.id,
        name: key.name,
        key: full_key,
        key_prefix: key.key_prefix,
        created_at: key.created_at,
    }))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, key_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
    workspace::require_write_access(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    // Verify api key belongs to this workspace
    let existing = ApiKeyRepository::find_by_id(&state.db, key_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, "API key not found".to_string()))?;

    if existing.workspace_id != workspace_id {
        return Err((StatusCode::NOT_FOUND, "API key not found".to_string()));
    }

    ApiKeyRepository::delete(&state.db, key_id)
        .await
        .map_err(db_error)?;

    Ok(StatusCode::NO_CONTENT)
}

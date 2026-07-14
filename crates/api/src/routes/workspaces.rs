use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use mcp_common::types::{CreateWorkspaceRequest, WorkspaceResponse};
use mcp_db::{CreateWorkspace, WorkspaceRepository};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::db_error;
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::routes::helpers::{ERR_INSUFFICIENT_PERMISSIONS, ERR_WORKSPACE_NOT_FOUND};

pub async fn list(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<Json<Vec<WorkspaceResponse>>, (StatusCode, String)> {
    let workspaces = WorkspaceRepository::list_by_user(&state.db, auth_user.user_id)
        .await
        .map_err(db_error)?;

    let response: Vec<WorkspaceResponse> = workspaces
        .into_iter()
        .map(|w| {
            let plan = w.plan();
            let role = w.role();
            WorkspaceResponse {
                id: w.id,
                name: w.name,
                slug: w.slug,
                plan,
                role,
                created_at: w.created_at,
            }
        })
        .collect();

    Ok(Json(response))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Json(body): Json<CreateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, String)> {
    // Check if slug is already taken
    if WorkspaceRepository::find_by_slug(&state.db, &body.slug)
        .await
        .map_err(db_error)?
        .is_some()
    {
        return Err((StatusCode::CONFLICT, "Slug already taken".to_string()));
    }

    let workspace = WorkspaceRepository::create(
        &state.db,
        CreateWorkspace {
            name: body.name,
            slug: body.slug,
            owner_id: auth_user.user_id,
        },
    )
    .await
    .map_err(db_error)?;

    let plan = workspace.plan();
    Ok(Json(WorkspaceResponse {
        id: workspace.id,
        name: workspace.name,
        slug: workspace.slug,
        plan,
        role: mcp_common::types::WorkspaceRole::Owner,
        created_at: workspace.created_at,
    }))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, String)> {
    // Check membership
    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, "Not a member of this workspace".to_string()))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    let plan = workspace.plan();
    let role = member.role();
    Ok(Json(WorkspaceResponse {
        id: workspace.id,
        name: workspace.name,
        slug: workspace.slug,
        plan,
        role,
        created_at: workspace.created_at,
    }))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, String)> {
    // Check membership and permission
    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, "Not a member of this workspace".to_string()))?;

    if !matches!(member.role(), mcp_common::types::WorkspaceRole::Owner | mcp_common::types::WorkspaceRole::Admin) {
        return Err((StatusCode::FORBIDDEN, ERR_INSUFFICIENT_PERMISSIONS.to_string()));
    }

    let workspace = WorkspaceRepository::update(
        &state.db,
        workspace_id,
        mcp_db::UpdateWorkspace {
            name: body.get("name").and_then(|v| v.as_str()).map(String::from),
            plan: None,
        },
    )
    .await
    .map_err(db_error)?;

    // Invalidate workspace cache after update
    state.cache.invalidate_workspace_info(workspace_id).await;

    let plan = workspace.plan();
    let role = member.role();
    Ok(Json(WorkspaceResponse {
        id: workspace.id,
        name: workspace.name,
        slug: workspace.slug,
        plan,
        role,
        created_at: workspace.created_at,
    }))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Only owner can delete workspace
    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    if workspace.owner_id != auth_user.user_id {
        return Err((StatusCode::FORBIDDEN, "Only owner can delete workspace".to_string()));
    }

    WorkspaceRepository::delete(&state.db, workspace_id)
        .await
        .map_err(db_error)?;

    // Invalidate workspace cache after delete
    state.cache.invalidate_workspace_info(workspace_id).await;

    Ok(StatusCode::NO_CONTENT)
}

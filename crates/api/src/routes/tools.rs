use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use mcp_common::types::{ToolResponse, UpdateToolRequest};
use mcp_db::{ToolRepository, UpdateTool, WorkspaceRepository};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::db_error;
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::routes::helpers::{verify_server_ownership, ERR_INSUFFICIENT_PERMISSIONS, ERR_NOT_A_MEMBER, ERR_SERVER_NOT_FOUND};

#[derive(serde::Deserialize)]
pub struct ToolPath {
    pub workspace_id: Uuid,
    pub server_id: Uuid,
    pub tool_id: Uuid,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<ToolResponse>>, (StatusCode, String)> {
    // Check membership
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    // SECURITY: Verify server belongs to this workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    let tools = ToolRepository::list_by_server(&state.db, server_id)
        .await
        .map_err(db_error)?;

    let response: Vec<ToolResponse> = tools
        .into_iter()
        .map(|t| {
            let permission_level = t.permission_level();
            ToolResponse {
                id: t.id,
                server_id: t.server_id,
                name: t.name,
                description: t.description,
                input_schema: t.input_schema,
                enabled: t.enabled,
                permission_level,
                rate_limit_per_minute: t.rate_limit_per_minute,
            }
        })
        .collect();

    Ok(Json(response))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(path): Path<ToolPath>,
    Json(body): Json<UpdateToolRequest>,
) -> Result<Json<ToolResponse>, (StatusCode, String)> {
    // Check membership and permission
    let member = WorkspaceRepository::get_member(&state.db, path.workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    if matches!(member.role(), mcp_common::types::WorkspaceRole::Viewer) {
        return Err((StatusCode::FORBIDDEN, ERR_INSUFFICIENT_PERMISSIONS.to_string()));
    }

    // SECURITY: Verify server belongs to this workspace
    verify_server_ownership(&state, path.workspace_id, path.server_id).await?;

    // SECURITY: Verify tool belongs to this server
    let existing_tool = ToolRepository::find_by_id(&state.db, path.tool_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, "Tool not found".to_string()))?;

    if existing_tool.server_id != path.server_id {
        return Err((StatusCode::NOT_FOUND, "Tool not found".to_string()));
    }

    let tool = ToolRepository::update(
        &state.db,
        path.tool_id,
        UpdateTool {
            description: None,
            enabled: body.enabled,
            permission_level: body.permission_level,
            rate_limit_per_minute: body.rate_limit_per_minute,
        },
    )
    .await
    .map_err(db_error)?;

    let permission_level = tool.permission_level();
    Ok(Json(ToolResponse {
        id: tool.id,
        server_id: tool.server_id,
        name: tool.name,
        description: tool.description,
        input_schema: tool.input_schema,
        enabled: tool.enabled,
        permission_level,
        rate_limit_per_minute: tool.rate_limit_per_minute,
    }))
}

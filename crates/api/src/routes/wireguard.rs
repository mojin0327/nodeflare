use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use mcp_db::WorkspaceRepository;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::db_error;
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::routes::helpers::{ERR_NOT_A_MEMBER, ERR_WORKSPACE_NOT_FOUND};

#[derive(Debug, Deserialize)]
pub struct CreateWireGuardRequest {}

#[derive(Debug, Serialize)]
pub struct WireGuardPeerResponse {
    pub name: String,
    pub region: String,
    pub peer_ip: String,
}

#[derive(Debug, Serialize)]
pub struct WireGuardConfigResponse {
    pub peer_name: String,
    pub config_file: String,
    pub peer_ip: String,
    pub instructions: Vec<String>,
}

/// List WireGuard peers (not supported in bare-metal mode)
pub async fn list_wireguard_peers(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<WireGuardPeerResponse>>, (StatusCode, String)> {
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    Err((StatusCode::SERVICE_UNAVAILABLE, "WireGuard is not available in bare-metal mode".to_string()))
}

/// Create a WireGuard peer (not supported in bare-metal mode)
pub async fn create_wireguard_peer(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Json(_body): Json<CreateWireGuardRequest>,
) -> Result<Json<WireGuardConfigResponse>, (StatusCode, String)> {
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    Err((StatusCode::SERVICE_UNAVAILABLE, "WireGuard is not available in bare-metal mode".to_string()))
}

/// Delete a WireGuard peer (not supported in bare-metal mode)
pub async fn delete_wireguard_peer(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, _peer_name)): Path<(Uuid, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    Err((StatusCode::SERVICE_UNAVAILABLE, "WireGuard is not available in bare-metal mode".to_string()))
}

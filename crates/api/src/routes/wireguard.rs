use anyhow;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use mcp_container::FlyioRuntime;
use mcp_db::WorkspaceRepository;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{db_error, internal_error};
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::routes::helpers::{ERR_NOT_A_MEMBER, ERR_WORKSPACE_NOT_FOUND};

#[derive(Debug, Deserialize)]
pub struct CreateWireGuardRequest {
    pub name: String,
    #[serde(default = "default_region")]
    pub region: String,
}

fn default_region() -> String {
    "nrt".to_string() // Tokyo
}

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

/// List WireGuard peers for a workspace
pub async fn list_wireguard_peers(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<WireGuardPeerResponse>>, (StatusCode, String)> {
    // Verify user is member
    let _member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    // Get workspace for filtering peers by prefix
    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    // Get Fly.io runtime
    let fly_runtime = state.fly_runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Fly.io runtime not configured".to_string(),
    ))?;

    let org_slug = std::env::var("FLY_ORG_SLUG").unwrap_or_else(|_| "personal".to_string());

    // List all peers and filter by workspace prefix
    let all_peers = fly_runtime
        .list_wireguard_peers(&org_slug)
        .await
        .map_err(|e: anyhow::Error| internal_error("WireGuard operation failed", e))?;

    let workspace_prefix = format!("{}-", workspace.slug);
    let filtered_peers: Vec<WireGuardPeerResponse> = all_peers
        .into_iter()
        .filter(|p| p.name.starts_with(&workspace_prefix))
        .map(|p| WireGuardPeerResponse {
            name: p.name,
            region: p.region,
            peer_ip: p.peerip,
        })
        .collect();

    Ok(Json(filtered_peers))
}

/// Create a WireGuard peer for accessing MCP servers directly
pub async fn create_wireguard_peer(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<CreateWireGuardRequest>,
) -> Result<Json<WireGuardConfigResponse>, (StatusCode, String)> {
    // Verify user is owner/admin
    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    if !matches!(
        member.role(),
        mcp_common::types::WorkspaceRole::Owner | mcp_common::types::WorkspaceRole::Admin
    ) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only owners and admins can create VPN connections".to_string(),
        ));
    }

    // Get workspace for org slug
    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    // Get Fly.io runtime
    let fly_runtime = state.fly_runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Fly.io runtime not configured".to_string(),
    ))?;

    // Create unique peer name
    let peer_name = format!(
        "{}-{}-{}",
        workspace.slug,
        body.name,
        &Uuid::new_v4().to_string()[..8]
    );

    // Get org slug from environment or workspace
    let org_slug = std::env::var("FLY_ORG_SLUG").unwrap_or_else(|_| "personal".to_string());

    // Create WireGuard peer
    let config = fly_runtime
        .create_wireguard_peer(&org_slug, &body.region, &peer_name)
        .await
        .map_err(|e: anyhow::Error| internal_error("WireGuard operation failed", e))?;

    // Generate config file content
    let config_file = FlyioRuntime::generate_wireguard_config(&config);

    Ok(Json(WireGuardConfigResponse {
        peer_name: config.peer_name,
        config_file,
        peer_ip: config.peer_ip,
        instructions: vec![
            "1. WireGuardをインストール: https://www.wireguard.com/install/".to_string(),
            "2. 上記の設定ファイルを nodeflare.conf として保存".to_string(),
            "3. WireGuardアプリで設定をインポート".to_string(),
            "4. 接続を有効化".to_string(),
            format!(
                "5. MCPサーバーに直接アクセス: curl http://[server].internal:8080"
            ),
        ],
    }))
}

/// Delete a WireGuard peer
pub async fn delete_wireguard_peer(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, peer_name)): Path<(Uuid, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Verify user is owner/admin
    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    if !matches!(
        member.role(),
        mcp_common::types::WorkspaceRole::Owner | mcp_common::types::WorkspaceRole::Admin
    ) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only owners and admins can delete VPN connections".to_string(),
        ));
    }

    // SECURITY: all workspaces share one Fly org, and peers are named
    // `{workspace.slug}-...`. Enforce that the requested peer belongs to THIS workspace,
    // otherwise an owner/admin of workspace A could delete workspace B's VPN peer by name
    // (the `list` handler already filters by this same prefix).
    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;
    if !peer_name.starts_with(&format!("{}-", workspace.slug)) {
        return Err((StatusCode::NOT_FOUND, "Peer not found".to_string()));
    }

    // Get Fly.io runtime
    let fly_runtime = state.fly_runtime.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Fly.io runtime not configured".to_string(),
    ))?;

    let org_slug = std::env::var("FLY_ORG_SLUG").unwrap_or_else(|_| "personal".to_string());

    fly_runtime
        .remove_wireguard_peer(&org_slug, &peer_name)
        .await
        .map_err(|e: anyhow::Error| internal_error("WireGuard operation failed", e))?;

    Ok(StatusCode::NO_CONTENT)
}

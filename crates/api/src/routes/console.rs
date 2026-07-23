use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use mcp_db::{ServerRegionRepository, ServerRepository, WorkspaceRepository};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::db_error;
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::routes::helpers::{ERR_NOT_A_MEMBER, ERR_SERVER_NOT_FOUND};

#[derive(Debug, Deserialize)]
pub struct ExecRequest {
    pub command: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout: u32,
    /// Optional region code to execute on. If not specified, uses primary region.
    pub region: Option<String>,
}

fn default_timeout() -> u32 {
    30
}

#[derive(Debug, Serialize)]
pub struct ExecResponseBody {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Execute a command on an MCP server container via Builder internal API
pub async fn exec_command(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<ExecRequest>,
) -> Result<Json<ExecResponseBody>, (StatusCode, String)> {
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
            "Only owners and admins can execute commands".to_string(),
        ));
    }

    // Get server
    let server = ServerRepository::find_by_id(&state.db, server_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_SERVER_NOT_FOUND.to_string()))?;

    if server.workspace_id != workspace_id {
        return Err((StatusCode::NOT_FOUND, ERR_SERVER_NOT_FOUND.to_string()));
    }

    // Get target region's machine ID (stored as container_name)
    let target_region = if let Some(region_code) = &body.region {
        ServerRegionRepository::find_by_server_and_region(&state.db, server_id, region_code)
            .await
            .map_err(db_error)?
            .ok_or((StatusCode::NOT_FOUND, format!("Region '{}' not found", region_code)))?
    } else {
        ServerRegionRepository::find_primary(&state.db, server_id)
            .await
            .map_err(db_error)?
            .ok_or((StatusCode::BAD_REQUEST, "No primary region configured".to_string()))?
    };

    let container_name = target_region
        .machine_id
        .ok_or((StatusCode::BAD_REQUEST, "Server not deployed in this region".to_string()))?;

    // Call Builder's /internal/exec/{container} endpoint
    let url = format!("{}/internal/exec/{}", state.builder_internal_url, container_name);
    let builder_token = std::env::var("BUILDER_TOKEN").unwrap_or_default();

    let payload = serde_json::json!({
        "command": body.command,
        "timeout": body.timeout,
    });

    let mut req = state.http.post(&url).json(&payload);
    if !builder_token.is_empty() {
        req = req.bearer_auth(&builder_token);
    }

    let resp = req.send().await.map_err(|e| {
        (StatusCode::BAD_GATEWAY, format!("Builder unreachable: {}", e))
    })?;

    let status = resp.status();
    let json: serde_json::Value = resp.json().await.map_err(|e| {
        (StatusCode::BAD_GATEWAY, format!("Builder response parse error: {}", e))
    })?;

    if !status.is_success() {
        return Err((
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            json.get("error").and_then(|v| v.as_str()).unwrap_or("exec failed").to_string(),
        ));
    }

    Ok(Json(ExecResponseBody {
        stdout: json.get("stdout").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        stderr: json.get("stderr").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        exit_code: json.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1) as i32,
    }))
}

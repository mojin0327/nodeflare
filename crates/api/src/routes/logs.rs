use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, Utc};
use mcp_common::types::RequestLogResponse;
use mcp_db::{RequestLogRepository, RequestLogStats, ServerRepository, ToolUsageStats, WorkspaceRepository};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::db_error;
use crate::extractors::AuthUser;
use crate::routes::helpers::{verify_server_ownership, ERR_NOT_A_MEMBER, ERR_WORKSPACE_NOT_FOUND};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct LogsFilterParams {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    pub status: Option<String>,
    #[allow(dead_code)]
    pub method: Option<String>,
    pub time_range: Option<String>,
    pub search: Option<String>,
}

fn default_page() -> u32 { 1 }
fn default_per_page() -> u32 { 50 }

#[derive(Serialize)]
pub struct PaginatedLogsResponse {
    pub data: Vec<RequestLogResponse>,
    pub total: i64,
    pub page: u32,
    pub per_page: u32,
}



pub async fn list(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
    Query(params): Query<LogsFilterParams>,
) -> Result<Json<PaginatedLogsResponse>, (StatusCode, String)> {
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    // Verify server belongs to workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    // Get workspace to check plan
    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    // Free plan users can only see logs from last 1 hour
    let is_free_plan = workspace.plan == "free" || workspace.plan.is_empty();
    let effective_time_range = if is_free_plan {
        Some("1h")
    } else {
        params.time_range.as_deref()
    };

    let limit = params.per_page.min(100) as i64;
    let offset = ((params.page.saturating_sub(1)) * params.per_page) as i64;

    let (logs, total) = RequestLogRepository::list_by_server_filtered(
        &state.db,
        server_id,
        limit,
        offset,
        params.status.as_deref(),
        effective_time_range,
        params.search.as_deref(),
    )
    .await
    .map_err(db_error)?;

    let data: Vec<RequestLogResponse> = logs
        .into_iter()
        .map(|l| RequestLogResponse {
            id: l.id,
            server_id: l.server_id,
            tool_name: l.tool_name,
            response_status: l.response_status,
            duration_ms: l.duration_ms,
            created_at: l.created_at,
        })
        .collect();

    Ok(Json(PaginatedLogsResponse {
        data,
        total,
        page: params.page,
        per_page: params.per_page,
    }))
}

#[derive(Serialize)]
pub struct ServerStatsResponse {
    pub stats: RequestLogStats,
    pub tool_usage: Vec<ToolUsageStats>,
}

pub async fn stats(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ServerStatsResponse>, (StatusCode, String)> {
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    // Verify server belongs to workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    let since = Utc::now() - Duration::days(7);

    let stats = RequestLogRepository::get_stats(&state.db, server_id, since)
        .await
        .map_err(db_error)?;

    let tool_usage = RequestLogRepository::get_tool_usage_stats(&state.db, server_id, since)
        .await
        .map_err(db_error)?;

    Ok(Json(ServerStatsResponse { stats, tool_usage }))
}

/// Batch stats response for multiple servers
#[derive(Serialize)]
pub struct BatchStatsResponse {
    pub servers: Vec<mcp_db::ServerStatsSummary>,
}

/// Get stats for all servers in a workspace (batch endpoint for dashboard)
pub async fn batch_stats(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<BatchStatsResponse>, (StatusCode, String)> {
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    let since = Utc::now() - Duration::days(7);

    // Use a single query to get stats for all servers in the workspace
    let servers = RequestLogRepository::get_batch_stats(&state.db, workspace_id, since)
        .await
        .map_err(db_error)?;

    Ok(Json(BatchStatsResponse { servers }))
}

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Datelike, TimeZone, Utc};
use mcp_billing::Plan as BillingPlan;
use mcp_common::types::{DeploymentResponse, DeploymentStatus, PaginationParams};
use mcp_db::{CreateDeployment, DeploymentRepository, ServerRepository, WorkspaceRepository};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::db_error;
use crate::extractors::{workspace, AuthUser};
use crate::routes::helpers::{
    verify_server_ownership, ERR_DEPLOYMENT_NOT_FOUND, ERR_SERVER_NOT_FOUND, ERR_WORKSPACE_NOT_FOUND,
};
use crate::state::AppState;

pub(crate) fn to_deployment_response(d: mcp_db::Deployment) -> DeploymentResponse {
    let status = d.status();
    let build_duration_seconds = d.finished_at.map(|f| (f - d.started_at).num_seconds());
    DeploymentResponse {
        id: d.id,
        server_id: d.server_id,
        version: d.version,
        commit_sha: d.commit_sha,
        status,
        error_message: d.error_message,
        build_logs: d.build_logs,
        started_at: d.started_at,
        finished_at: d.finished_at,
        created_at: d.started_at,
        deployed_at: d.finished_at,
        build_duration_seconds,
    }
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<Vec<DeploymentResponse>>, (StatusCode, String)> {
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    // Verify server belongs to workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    let deployments = DeploymentRepository::list_by_server(
        &state.db,
        server_id,
        pagination.limit() as i64,
        pagination.offset() as i64,
    )
    .await
    .map_err(db_error)?;

    let response: Vec<DeploymentResponse> = deployments
        .into_iter()
        .map(to_deployment_response)
        .collect();

    Ok(Json(response))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id, deployment_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<Json<DeploymentResponse>, (StatusCode, String)> {
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    // Verify server belongs to workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    let deployment = DeploymentRepository::find_by_id(&state.db, deployment_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_DEPLOYMENT_NOT_FOUND.to_string()))?;

    // Verify deployment belongs to the specified server (prevents IDOR)
    if deployment.server_id != server_id {
        return Err((StatusCode::NOT_FOUND, ERR_DEPLOYMENT_NOT_FOUND.to_string()));
    }

    Ok(Json(to_deployment_response(deployment)))
}

#[derive(serde::Serialize)]
pub struct DeploymentLogsResponse {
    pub logs: Option<String>,
}

pub async fn get_logs(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id, deployment_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<Json<DeploymentLogsResponse>, (StatusCode, String)> {
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    // Verify server belongs to workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    let deployment = DeploymentRepository::find_by_id(&state.db, deployment_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_DEPLOYMENT_NOT_FOUND.to_string()))?;

    // Verify deployment belongs to the specified server (prevents IDOR)
    if deployment.server_id != server_id {
        return Err((StatusCode::NOT_FOUND, ERR_DEPLOYMENT_NOT_FOUND.to_string()));
    }

    Ok(Json(DeploymentLogsResponse {
        logs: deployment.build_logs,
    }))
}

/// Rollback to a previous successful deployment
pub async fn rollback(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id, deployment_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<Json<DeploymentResponse>, (StatusCode, String)> {
    workspace::require_write_access(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    // Verify server belongs to workspace
    verify_server_ownership(&state, workspace_id, server_id).await?;

    // Get the deployment to rollback to
    let target_deployment = DeploymentRepository::find_by_id(&state.db, deployment_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_DEPLOYMENT_NOT_FOUND.to_string()))?;

    // Verify deployment belongs to this server
    if target_deployment.server_id != server_id {
        return Err((StatusCode::NOT_FOUND, ERR_DEPLOYMENT_NOT_FOUND.to_string()));
    }

    // Only allow rollback to successful deployments
    if target_deployment.status() != DeploymentStatus::Succeeded {
        return Err((StatusCode::BAD_REQUEST, "Can only rollback to successful deployments".to_string()));
    }

    // Get server info
    let server = ServerRepository::find_by_id(&state.db, server_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_SERVER_NOT_FOUND.to_string()))?;

    // Create new deployment with same commit SHA
    let deployment = DeploymentRepository::create(
        &state.db,
        CreateDeployment {
            server_id,
            commit_sha: target_deployment.commit_sha.clone(),
            deployed_by: Some(auth_user.user_id),
        },
    )
    .await
    .map_err(db_error)?;

    // Update server status to building
    ServerRepository::update_status(
        &state.db,
        server_id,
        mcp_common::types::ServerStatus::Building,
        None,
    )
    .await
    .map_err(db_error)?;

    // Enqueue build job
    let build_job = mcp_queue::BuildJob {
        deployment_id: deployment.id,
        server_id,
        app_name: server.fly_app_name,
        github_repo: server.github_repo,
        github_branch: server.github_branch,
        commit_sha: target_deployment.commit_sha,
        runtime: server.runtime,
        github_installation_id: server.github_installation_id,
        region: server.region,
        root_directory: server.root_directory,
        mcp_path: server.mcp_path,
        transport: server.transport,
        entry_command: server.entry_command,
        build_command: server.build_command,
        memory_mb: server.memory_mb,
        port: server.port,
        upstream_oauth_provider: server.upstream_oauth_provider,
    };

    state
        .job_queue
        .push_build_job(build_job)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to enqueue build job: {}", e)))?;

    tracing::info!("Rollback build job enqueued for deployment {}", deployment.id);

    Ok(Json(to_deployment_response(deployment)))
}

/// Deployment usage stats response
#[derive(serde::Serialize)]
pub struct DeploymentUsageResponse {
    pub deployments_this_month: i64,
    pub max_deployments: u32,
}

/// Get deployment usage stats for a workspace
pub async fn usage(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<DeploymentUsageResponse>, (StatusCode, String)> {
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    // Get workspace to check plan
    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    // Get plan limits
    let billing_plan = BillingPlan::from_str(&workspace.plan);
    let limits = billing_plan.limits();

    // Count deployments this month (try cache first)
    let now = Utc::now();
    let year = now.year();
    let month = now.month();

    let deployments_this_month = if let Some(cached_count) = state.cache.get_deployment_count(workspace_id, year, month).await {
        cached_count
    } else {
        // Cache miss - fetch from database
        let month_start = Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
            .single()
            .unwrap_or(now);

        let count = DeploymentRepository::count_by_workspace_since(
            &state.db,
            workspace_id,
            month_start,
        )
        .await
        .map_err(db_error)?;

        // Cache the count
        state.cache.set_deployment_count(workspace_id, year, month, count).await;
        count
    };

    Ok(Json(DeploymentUsageResponse {
        deployments_this_month,
        max_deployments: limits.max_deployments_per_month,
    }))
}

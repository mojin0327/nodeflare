use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use mcp_common::types::{
    PublishTemplateRequest, ServerTemplatePrivateResponse, ServerTemplateResponse, Transport,
    Runtime,
};
use mcp_db::{SecretRepository, ServerRepository, ServerTemplate, ServerTemplateRepository, CreateServerTemplate, WorkspaceRepository};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;
use validator::Validate;

use crate::error::AppError;
use crate::extractors::{workspace, AuthUser};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// List all published templates — no auth required (public explore page).
/// Only templates whose backing server is still public are returned.
pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<ServerTemplateResponse>>, AppError> {
    let limit = q.limit.clamp(1, 100);
    let offset = q.offset.clamp(0, 100_000);
    let templates = ServerTemplateRepository::list_public(&state.db, limit, offset).await?;
    let response = templates.into_iter().map(into_public_response).collect();
    Ok(Json(response))
}

/// Get a single template by ID — no auth required.
/// Returns 404 if the backing server is no longer public.
pub async fn get(
    State(state): State<Arc<AppState>>,
    Path(template_id): Path<Uuid>,
) -> Result<Json<ServerTemplateResponse>, AppError> {
    let template = ServerTemplateRepository::find_public_by_id(&state.db, template_id)
        .await?
        .ok_or_else(|| AppError::not_found("template"))?;

    Ok(Json(into_public_response(template)))
}

/// Publish (or update) a server as a public template.
/// Requires write access. Only the template owner or an admin/owner may update an existing one.
pub async fn publish(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<PublishTemplateRequest>,
) -> Result<Json<ServerTemplatePrivateResponse>, AppError> {
    // Requires at least write access (not viewer)
    workspace::require_write_access(&state.db, workspace_id, auth_user.user_id).await?;

    let server = ServerRepository::find_by_id(&state.db, server_id)
        .await?
        .ok_or_else(|| AppError::not_found("server"))?;

    if server.workspace_id != workspace_id {
        return Err(AppError::not_found("server"));
    }

    if server.visibility != "public" {
        tracing::warn!(
            server_id = %server_id,
            visibility = %server.visibility,
            user_id = %auth_user.user_id,
            "Attempted to publish template for non-public server"
        );
        return Err(AppError::bad_request(
            "NOT_PUBLIC",
            "Only public servers can be shared as templates. Change the server visibility to Public first.",
        ));
    }

    // If a template already exists for this server, only the original owner or an admin/owner
    // may overwrite it to prevent content hijacking by other workspace members.
    if let Some(existing) =
        ServerTemplateRepository::find_by_server_id(&state.db, server_id).await?
    {
        let is_owner_or_admin = {
            use mcp_common::types::WorkspaceRole;
            let member =
                WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
                    .await?
                    .ok_or_else(|| AppError::forbidden("Not a member of this workspace"))?;
            let role = member.role();
            existing.owner_id == auth_user.user_id
                || role == WorkspaceRole::Admin
                || role == WorkspaceRole::Owner
        };
        if !is_owner_or_admin {
            return Err(AppError::forbidden(
                "Only the template owner or a workspace admin can update this template",
            ));
        }
    }

    // Validate request body
    if let Err(errors) = body.validate() {
        return Err(AppError::bad_request(
            "VALIDATION_ERROR",
            &format!("{}", errors),
        ));
    }

    // Validate that the provided env var keys actually exist as secrets on this server
    let existing_secrets = SecretRepository::list_by_server(&state.db, server_id).await?;
    let existing_keys: std::collections::HashSet<&str> =
        existing_secrets.iter().map(|s| s.key.as_str()).collect();

    let invalid_keys: Vec<&str> = body
        .required_env_var_keys
        .iter()
        .map(|k| k.as_str())
        .filter(|k| !existing_keys.contains(k))
        .collect();

    if !invalid_keys.is_empty() {
        tracing::warn!(
            server_id = %server_id,
            invalid_keys = ?invalid_keys,
            "Publish template rejected: env var keys not found in secrets"
        );
        return Err(AppError::bad_request(
            "INVALID_ENV_VAR_KEYS",
            &format!(
                "The following env var keys do not exist on this server: {}",
                invalid_keys.join(", ")
            ),
        ));
    }

    let name = body.name.unwrap_or_else(|| server.name.clone());

    let template = ServerTemplateRepository::upsert(
        &state.db,
        CreateServerTemplate {
            server_id,
            owner_id: auth_user.user_id,
            workspace_id,
            name,
            description: body.description.or(server.description.clone()),
            github_repo: server.github_repo.clone(),
            github_branch: server.github_branch.clone(),
            runtime: server.runtime.clone(),
            transport: server.transport.clone(),
            mcp_path: server.mcp_path.clone(),
            entry_command: server.entry_command.clone(),
            build_command: server.build_command.clone(),
            root_directory: server.root_directory.clone(),
            auth_enabled: server.auth_enabled,
            memory_mb: server.memory_mb,
            port: server.port,
            upstream_oauth_provider: server.upstream_oauth_provider.clone(),
            upstream_oauth_scopes: server.upstream_oauth_scopes.clone().unwrap_or_default(),
            required_env_var_keys: body.required_env_var_keys,
            icon_url: body.icon_url,
        },
    )
    .await?;

    Ok(Json(into_private_response(template)))
}

/// Get the template published from a specific server (if any) — authenticated.
pub async fn get_by_server(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Option<ServerTemplatePrivateResponse>>, AppError> {
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::forbidden("Not a member of this workspace"))?;

    let server = ServerRepository::find_by_id(&state.db, server_id)
        .await?
        .ok_or_else(|| AppError::not_found("server"))?;

    if server.workspace_id != workspace_id {
        return Err(AppError::not_found("server"));
    }

    let template = ServerTemplateRepository::find_by_server_id(&state.db, server_id).await?;
    Ok(Json(template.map(into_private_response)))
}

/// Unpublish (delete) the template for a server.
/// Only the template owner or a workspace admin/owner may do this.
pub async fn unpublish(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    use mcp_common::types::WorkspaceRole;
    let member = workspace::require_write_access(&state.db, workspace_id, auth_user.user_id).await?;

    let server = ServerRepository::find_by_id(&state.db, server_id)
        .await?
        .ok_or_else(|| AppError::not_found("server"))?;

    if server.workspace_id != workspace_id {
        return Err(AppError::not_found("server"));
    }

    // Check ownership: only the template owner or admin/owner may delete
    if let Some(existing) =
        ServerTemplateRepository::find_by_server_id(&state.db, server_id).await?
    {
        let role = member.role();
        let can_delete = existing.owner_id == auth_user.user_id
            || role == WorkspaceRole::Admin
            || role == WorkspaceRole::Owner;
        if !can_delete {
            return Err(AppError::forbidden(
                "Only the template owner or a workspace admin can unpublish this template",
            ));
        }
    }

    ServerTemplateRepository::delete_by_server_id(&state.db, server_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn into_public_response(t: ServerTemplate) -> ServerTemplateResponse {
    let runtime = parse_runtime(&t.runtime);
    let transport = parse_transport(&t.transport);
    ServerTemplateResponse {
        id: t.id,
        name: t.name,
        description: t.description,
        github_repo: t.github_repo,
        github_branch: t.github_branch,
        runtime,
        transport,
        mcp_path: t.mcp_path,
        entry_command: t.entry_command,
        build_command: t.build_command,
        root_directory: t.root_directory,
        auth_enabled: t.auth_enabled,
        memory_mb: t.memory_mb,
        port: t.port,
        upstream_oauth_provider: t.upstream_oauth_provider,
        upstream_oauth_scopes: t.upstream_oauth_scopes,
        required_env_var_keys: t.required_env_var_keys,
        icon_url: t.icon_url,
        use_count: t.use_count,
        created_at: t.created_at,
    }
}

fn into_private_response(t: ServerTemplate) -> ServerTemplatePrivateResponse {
    let runtime = parse_runtime(&t.runtime);
    let transport = parse_transport(&t.transport);
    ServerTemplatePrivateResponse {
        id: t.id,
        server_id: t.server_id,
        owner_id: t.owner_id,
        workspace_id: t.workspace_id,
        name: t.name,
        description: t.description,
        github_repo: t.github_repo,
        github_branch: t.github_branch,
        runtime,
        transport,
        mcp_path: t.mcp_path,
        entry_command: t.entry_command,
        build_command: t.build_command,
        root_directory: t.root_directory,
        auth_enabled: t.auth_enabled,
        memory_mb: t.memory_mb,
        port: t.port,
        upstream_oauth_provider: t.upstream_oauth_provider,
        upstream_oauth_scopes: t.upstream_oauth_scopes,
        required_env_var_keys: t.required_env_var_keys,
        icon_url: t.icon_url,
        use_count: t.use_count,
        created_at: t.created_at,
    }
}

fn parse_runtime(s: &str) -> Runtime {
    match s {
        "python" => Runtime::Python,
        "go" => Runtime::Go,
        "rust" => Runtime::Rust,
        "docker" => Runtime::Docker,
        _ => Runtime::Node,
    }
}

fn parse_transport(s: &str) -> Transport {
    match s {
        "stdio" => Transport::Stdio,
        _ => Transport::Sse,
    }
}

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::Datelike;
use mcp_billing::{validate_memory_choice, MemoryChoiceError, Plan as BillingPlan, MEMORY_LADDER_MB};
use mcp_common::types::{CreateServerRequest, PaginationParams, ServerResponse, ServerMinimalResponse, ServerBasicResponse, ServerListResponse, UpdateServerRequest};
use mcp_db::{CreateDeployment, CreateSecret, CreateServer, CreateServerRegion, DeploymentRepository, SecretRepository, ServerRegionRepository, ServerRepository, UpdateServer, WorkspaceRepository};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{workspace, AuthUser};
use crate::state::AppState;

#[derive(serde::Deserialize)]
pub struct ServerPath {
    pub workspace_id: Uuid,
    pub server_id: Uuid,
}

/// Validate a user-selected per-server memory size against the allowed ladder and
/// the workspace plan's ceiling, turning a rejection into a client-facing error.
fn check_memory_choice(
    memory_mb: Option<i32>,
    limits: &mcp_billing::PlanLimits,
    plan: &str,
) -> Result<(), AppError> {
    match validate_memory_choice(memory_mb.map(|m| m as u32), limits) {
        Ok(()) => Ok(()),
        Err(MemoryChoiceError::NotOnLadder) => Err(AppError::bad_request(
            "INVALID_MEMORY",
            &format!("Memory must be one of {:?} MB.", MEMORY_LADDER_MB),
        )),
        Err(MemoryChoiceError::ExceedsPlan { ceiling_mb }) => Err(AppError::payment_required(
            "MEMORY_LIMIT_REACHED",
            &format!(
                "Your {} plan allows up to {} MB of memory per server. Upgrade to select a larger size.",
                plan, ceiling_mb
            ),
        )
        .with_details(json!({
            "requested_mb": memory_mb,
            "max_allowed_mb": ceiling_mb,
            "plan": plan,
            "upgrade_url": "/dashboard/billing"
        }))),
    }
}

/// List servers in a workspace with minimal fields (id, workspace_id, name only)
/// Use this for selection lists to reduce payload size
/// List all servers across all workspaces the user has access to
pub async fn list_all(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<Json<Vec<ServerResponse>>, AppError> {
    // Use single JOIN query to prevent N+1 problem
    let servers = ServerRepository::list_all_by_user(&state.db, auth_user.user_id)
        .await?;

    let response: Vec<ServerResponse> = servers
        .into_iter()
        .map(|s| {
            let runtime = s.runtime();
            let visibility = s.visibility();
            let access_mode = s.access_mode();
            let transport = s.transport();
            let status = s.status();
            ServerResponse {
                id: s.id,
                workspace_id: s.workspace_id,
                name: s.name,
                slug: s.slug,
                description: s.description,
                github_repo: s.github_repo,
                github_branch: s.github_branch,
                runtime,
                visibility,
                access_mode,
                transport,
                status,
                endpoint_url: s.endpoint_url,
                region: s.region,
                root_directory: s.root_directory,
                mcp_path: s.mcp_path,
                entry_command: s.entry_command,
                build_command: s.build_command,
                auth_enabled: s.auth_enabled,
                memory_mb: s.memory_mb,
                port: s.port,
                tool_list_filter_by_scope: s.tool_list_filter_by_scope,
                tool_schema_slim: s.tool_schema_slim,
                tool_search_mode: s.tool_search_mode,
                tool_code_mode: s.tool_code_mode,
                upstream_oauth_provider: s.upstream_oauth_provider,
                upstream_oauth_scopes: s.upstream_oauth_scopes,
                upstream_oauth_authorization_url: s.upstream_oauth_authorization_url,
                upstream_oauth_token_url: s.upstream_oauth_token_url,
                upstream_oauth_client_id: s.upstream_oauth_client_id,
                created_at: s.created_at,
                updated_at: s.updated_at,
            }
        })
        .collect();

    Ok(Json(response))
}

/// List all servers with minimal fields (id, workspace_id, name only)
/// Use this for selection lists to reduce payload size
pub async fn list_all_minimal(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<Json<Vec<ServerMinimalResponse>>, AppError> {
    let servers = ServerRepository::list_all_by_user(&state.db, auth_user.user_id)
        .await?;

    let response: Vec<ServerMinimalResponse> = servers
        .into_iter()
        .map(|s| ServerMinimalResponse {
            id: s.id,
            workspace_id: s.workspace_id,
            name: s.name,
        })
        .collect();

    Ok(Json(response))
}

/// List all servers with basic fields (id, workspace_id, name, status, runtime)
/// Use this for dashboard overview and logs page
pub async fn list_all_basic(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<Json<Vec<ServerBasicResponse>>, AppError> {
    let servers = ServerRepository::list_all_by_user(&state.db, auth_user.user_id)
        .await?;

    let response: Vec<ServerBasicResponse> = servers
        .into_iter()
        .map(|s| {
            let status = s.status();
            let runtime = s.runtime();
            ServerBasicResponse {
                id: s.id,
                workspace_id: s.workspace_id,
                name: s.name,
                status,
                runtime,
            }
        })
        .collect();

    Ok(Json(response))
}

/// List all servers with list-display fields
/// Use this for server list page
pub async fn list_all_list(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
) -> Result<Json<Vec<ServerListResponse>>, AppError> {
    let servers = ServerRepository::list_all_by_user(&state.db, auth_user.user_id)
        .await?;

    let response: Vec<ServerListResponse> = servers
        .into_iter()
        .map(|s| {
            let runtime = s.runtime();
            let visibility = s.visibility();
            let status = s.status();
            ServerListResponse {
                id: s.id,
                workspace_id: s.workspace_id,
                name: s.name,
                slug: s.slug,
                runtime,
                visibility,
                status,
                github_repo: s.github_repo,
                endpoint_url: s.endpoint_url,
            }
        })
        .collect();

    Ok(Json(response))
}

pub async fn list_minimal(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<ServerMinimalResponse>>, AppError> {
    workspace::require_member(&state.db, workspace_id, auth_user.user_id).await?;

    let servers = ServerRepository::list_by_workspace(&state.db, workspace_id, 1000, 0)
        .await?;

    let response: Vec<ServerMinimalResponse> = servers
        .into_iter()
        .map(|s| ServerMinimalResponse {
            id: s.id,
            workspace_id: s.workspace_id,
            name: s.name,
        })
        .collect();

    Ok(Json(response))
}

/// List servers in a workspace with basic fields (id, workspace_id, name, status, runtime)
/// Use this for dashboard overview and logs page
pub async fn list_basic(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<ServerBasicResponse>>, AppError> {
    workspace::require_member(&state.db, workspace_id, auth_user.user_id).await?;

    let servers = ServerRepository::list_by_workspace(&state.db, workspace_id, 1000, 0)
        .await?;

    let response: Vec<ServerBasicResponse> = servers
        .into_iter()
        .map(|s| {
            let status = s.status();
            let runtime = s.runtime();
            ServerBasicResponse {
                id: s.id,
                workspace_id: s.workspace_id,
                name: s.name,
                status,
                runtime,
            }
        })
        .collect();

    Ok(Json(response))
}

/// List servers in a workspace with list-display fields
/// Use this for server list page
pub async fn list_summary(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<ServerListResponse>>, AppError> {
    workspace::require_member(&state.db, workspace_id, auth_user.user_id).await?;

    let servers = ServerRepository::list_by_workspace(&state.db, workspace_id, 1000, 0)
        .await?;

    let response: Vec<ServerListResponse> = servers
        .into_iter()
        .map(|s| {
            let runtime = s.runtime();
            let visibility = s.visibility();
            let status = s.status();
            ServerListResponse {
                id: s.id,
                workspace_id: s.workspace_id,
                name: s.name,
                slug: s.slug,
                runtime,
                visibility,
                status,
                github_repo: s.github_repo,
                endpoint_url: s.endpoint_url,
            }
        })
        .collect();

    Ok(Json(response))
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<Vec<ServerResponse>>, AppError> {
    // Check membership
    workspace::require_member(&state.db, workspace_id, auth_user.user_id).await?;

    let servers = ServerRepository::list_by_workspace(
        &state.db,
        workspace_id,
        pagination.limit() as i64,
        pagination.offset() as i64,
    )
    .await?;

    let response: Vec<ServerResponse> = servers
        .into_iter()
        .map(|s| {
            let runtime = s.runtime();
            let visibility = s.visibility();
            let access_mode = s.access_mode();
            let transport = s.transport();
            let status = s.status();
            ServerResponse {
                id: s.id,
                workspace_id: s.workspace_id,
                name: s.name,
                slug: s.slug,
                description: s.description,
                github_repo: s.github_repo,
                github_branch: s.github_branch,
                runtime,
                visibility,
                access_mode,
                transport,
                status,
                endpoint_url: s.endpoint_url,
                region: s.region,
                root_directory: s.root_directory,
                mcp_path: s.mcp_path,
                entry_command: s.entry_command,
                build_command: s.build_command,
                auth_enabled: s.auth_enabled,
                memory_mb: s.memory_mb,
                port: s.port,
                tool_list_filter_by_scope: s.tool_list_filter_by_scope,
                tool_schema_slim: s.tool_schema_slim,
                tool_search_mode: s.tool_search_mode,
                tool_code_mode: s.tool_code_mode,
                upstream_oauth_provider: s.upstream_oauth_provider,
                upstream_oauth_scopes: s.upstream_oauth_scopes,
                upstream_oauth_authorization_url: s.upstream_oauth_authorization_url,
                upstream_oauth_token_url: s.upstream_oauth_token_url,
                upstream_oauth_client_id: s.upstream_oauth_client_id,
                created_at: s.created_at,
                updated_at: s.updated_at,
            }
        })
        .collect();

    Ok(Json(response))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<CreateServerRequest>,
) -> Result<Json<ServerResponse>, AppError> {
    // SECURITY: Validate input using validator crate
    use validator::Validate;
    if let Err(validation_errors) = body.validate() {
        let error_messages: Vec<String> = validation_errors
            .field_errors()
            .iter()
            .flat_map(|(field, errors)| {
                errors.iter().map(move |e| {
                    format!("{}: {}", field, e.message.as_ref().map(|m| m.to_string()).unwrap_or_else(|| e.code.to_string()))
                })
            })
            .collect();
        return Err(AppError::bad_request(
            "VALIDATION_ERROR",
            &error_messages.join(", "),
        ).with_details(json!({
            "errors": error_messages
        })));
    }

    // Validate runtime
    let runtime = body.runtime.clone().unwrap_or_default();
    if !matches!(runtime,
        mcp_common::types::Runtime::Node |
        mcp_common::types::Runtime::Python |
        mcp_common::types::Runtime::Go |
        mcp_common::types::Runtime::Rust |
        mcp_common::types::Runtime::Docker
    ) {
        return Err(AppError::bad_request(
            "INVALID_RUNTIME",
            &format!("Unsupported runtime: {:?}. Supported runtimes are: node, python, go, rust, docker", runtime),
        ).with_details(json!({
            "provided_runtime": format!("{:?}", runtime),
            "supported_runtimes": ["node", "python", "go", "rust", "docker"]
        })));
    }

    // Check membership and write permission
    workspace::require_write_access(&state.db, workspace_id, auth_user.user_id).await?;

    // Get workspace to check plan limits
    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching workspace: {}", e);
            AppError::internal("Failed to fetch workspace")
        })?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;

    // Check plan limits for server count
    let billing_plan = match workspace.plan.as_str() {
        "pro" => BillingPlan::Pro,
        "team" => BillingPlan::Team,
        "enterprise" => BillingPlan::Enterprise,
        _ => BillingPlan::Free,
    };
    let limits = billing_plan.limits();

    // Reject a memory size the plan can't grant before we do any work.
    check_memory_choice(body.memory_mb, &limits, &workspace.plan)?;

    let current_server_count = ServerRepository::count_by_workspace(&state.db, workspace_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error counting servers: {}", e);
            AppError::internal("Failed to check server count")
        })?;

    if current_server_count >= limits.max_servers as i64 {
        return Err(AppError::payment_required(
            "SERVER_LIMIT_REACHED",
            &format!(
                "You have reached the maximum number of servers ({}) for your {} plan. Please upgrade to create more servers.",
                limits.max_servers,
                workspace.plan
            ),
        ).with_details(json!({
            "current_count": current_server_count,
            "max_allowed": limits.max_servers,
            "plan": workspace.plan,
            "upgrade_url": "/dashboard/billing"
        })));
    }

    // Check if slug is already taken
    if let Some(existing) = ServerRepository::find_by_slug(&state.db, workspace_id, &body.slug)
        .await
        .map_err(|e| {
            tracing::error!("Database error checking slug: {}", e);
            AppError::internal("Failed to check server slug availability")
        })?
    {
        return Err(AppError::conflict(
            "SLUG_ALREADY_EXISTS",
            &format!("A server with slug '{}' already exists in this workspace", body.slug),
        ).with_details(json!({
            "conflicting_slug": body.slug,
            "existing_server_name": existing.name,
            "suggestion": format!("{}-2", body.slug)
        })));
    }

    // The server's public host is `<workspace_slug>--<server_slug>.<base_domain>`, a single
    // DNS label. Forbid `--` in the server slug (it is the workspace/server separator the
    // proxy splits on) and cap the combined label at the 63-char DNS-label limit so the
    // deployed subdomain is always valid and unambiguously routable.
    if body.slug.contains("--") {
        return Err(AppError::bad_request(
            "INVALID_SLUG",
            "Server slug cannot contain consecutive hyphens ('--').",
        ).with_details(json!({ "conflicting_slug": body.slug })));
    }
    if workspace.slug.len() + 2 + body.slug.len() > 63 {
        return Err(AppError::bad_request(
            "SLUG_TOO_LONG",
            "The combined workspace and server slug is too long for a subdomain (the '<workspace>--<server>' host label must be at most 63 characters).",
        ).with_details(json!({
            "workspace_slug": workspace.slug,
            "workspace_slug_len": workspace.slug.len(),
            "server_slug": body.slug,
            "server_slug_len": body.slug.len(),
            "max_label_len": 63
        })));
    }

    // Validate GitHub repo format (avoid Vec allocation)
    let (owner, repo) = {
        let mut parts = body.github_repo.split('/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(o), Some(r), None) if !o.is_empty() && !r.is_empty() => (o, r),
            _ => {
                return Err(AppError::bad_request(
                    "INVALID_GITHUB_REPO",
                    "GitHub repository must be in format 'owner/repo'",
                ).with_details(json!({
                    "provided_repo": body.github_repo,
                    "expected_format": "owner/repo",
                    "example": "octocat/my-mcp-server"
                })));
            }
        }
    };
    let branch = body.github_branch.clone().unwrap_or_else(|| "main".to_string());

    // Validate MCP repository structure
    if let (Some(github), Some(installation_id)) = (&state.github, body.github_installation_id) {
        let runtime_str = match &runtime {
            mcp_common::types::Runtime::Node => "node",
            mcp_common::types::Runtime::Python => "python",
            mcp_common::types::Runtime::Go => "go",
            mcp_common::types::Runtime::Rust => "rust",
            mcp_common::types::Runtime::Docker => "docker",
        };

        match github.validate_mcp_repository(
            installation_id,
            owner,
            repo,
            &branch,
            Some(runtime_str),
        ).await {
            Ok(validation) => {
                if !validation.is_valid {
                    return Err(AppError::bad_request(
                        "INVALID_MCP_REPOSITORY",
                        "Repository does not appear to be a valid MCP server",
                    ).with_details(json!({
                        "errors": validation.errors,
                        "warnings": validation.warnings,
                        "detected_runtime": validation.detected_runtime,
                        "expected_runtime": runtime_str,
                        "help": "Make sure your repository contains package.json (Node.js) or requirements.txt/pyproject.toml (Python) with MCP SDK dependencies"
                    })));
                }

                // Log warnings if any
                for warning in &validation.warnings {
                    tracing::warn!("MCP validation warning for {}/{}: {}", owner, repo, warning);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to validate MCP repository: {}", e);
                // Don't block creation, just log warning
            }
        }
    }

    let server = ServerRepository::create(
        &state.db,
        CreateServer {
            workspace_id,
            name: body.name.clone(),
            slug: body.slug.clone(),
            description: body.description.clone(),
            github_repo: body.github_repo.clone(),
            github_branch: body.github_branch.clone().unwrap_or_else(|| "main".to_string()),
            github_installation_id: body.github_installation_id,
            runtime,
            visibility: body.visibility.clone().unwrap_or_default(),
            access_mode: body.access_mode.clone().unwrap_or_default(),
            transport: body.transport.unwrap_or_default(),
            region: "iad".to_string(), // Fixed to US East (Virginia)
            root_directory: body.root_directory.clone().unwrap_or_default(),
            mcp_path: body.mcp_path.clone().unwrap_or_else(|| "/mcp".to_string()),
            entry_command: body.entry_command.clone(),
            build_command: body.build_command.clone(),
            auth_enabled: body.auth_enabled.unwrap_or(true),
            memory_mb: body.memory_mb,
            port: body.port,
            upstream_oauth_provider: body.upstream_oauth_provider.clone(),
            upstream_oauth_scopes: body.upstream_oauth_scopes.clone(),
            upstream_oauth_authorization_url: body.upstream_oauth_authorization_url.clone(),
            upstream_oauth_token_url: body.upstream_oauth_token_url.clone(),
            upstream_oauth_client_id: body.upstream_oauth_client_id.clone(),
            upstream_oauth_client_secret: body.upstream_oauth_client_secret.clone(),
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to create server: {}", e);
        let error_msg = e.to_string().to_lowercase();

        // Parse specific database errors and return user-friendly messages
        if error_msg.contains("duplicate key") {
            if error_msg.contains("slug") {
                return AppError::conflict(
                    "SLUG_ALREADY_EXISTS",
                    &format!("A server with slug '{}' already exists", body.slug),
                );
            }
            return AppError::conflict("DUPLICATE_ENTRY", "A server with these details already exists");
        }

        // Check constraint violations
        if error_msg.contains("chk_mcp_servers_slug") || (error_msg.contains("slug") && error_msg.contains("check")) {
            return AppError::bad_request(
                "INVALID_SLUG",
                "Server slug must contain only lowercase letters, numbers, and hyphens. It cannot start or end with a hyphen.",
            );
        }

        if error_msg.contains("chk_mcp_servers_github_repo") || (error_msg.contains("github_repo") && error_msg.contains("check")) {
            return AppError::bad_request(
                "INVALID_REPOSITORY",
                "Invalid repository format. Please use the format 'owner/repository' (e.g., 'username/my-repo').",
            );
        }

        if error_msg.contains("chk_mcp_servers_runtime") || (error_msg.contains("runtime") && error_msg.contains("check")) {
            return AppError::bad_request(
                "INVALID_RUNTIME",
                "Invalid runtime selected. Please choose from: Node.js, Python, Go, or Rust.",
            );
        }

        if error_msg.contains("chk_mcp_servers_transport") || (error_msg.contains("transport") && error_msg.contains("check")) {
            return AppError::bad_request(
                "INVALID_TRANSPORT",
                "Invalid transport type. Please choose SSE or STDIO.",
            );
        }

        if error_msg.contains("violates foreign key") || error_msg.contains("foreign key constraint") {
            return AppError::bad_request(
                "INVALID_WORKSPACE",
                "The specified workspace does not exist or you don't have access to it.",
            );
        }

        // Generic fallback - don't expose internal details
        AppError::internal("Unable to create the server. Please check your input and try again.")
    })?;

    // Create primary region entry in server_regions table
    ServerRegionRepository::create(
        &state.db,
        CreateServerRegion {
            server_id: server.id,
            region: server.region.clone(),
            is_primary: true,
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to create primary region for server: {}", e);
        AppError::internal("Failed to create server region. Please try again.")
    })?;

    // Persist any environment variables provided at creation time. This must happen
    // BEFORE the auto-deploy build job is enqueued below, otherwise the first build
    // would run without the secrets and the user would have to redeploy — exactly the
    // workflow this feature removes. Each value is encrypted at rest, same as the
    // dedicated secrets endpoint.
    if let Some(env_vars) = &body.env_vars {
        for env in env_vars {
            // Reuse the secrets endpoint's key validation (injection-safe).
            crate::routes::secrets::validate_secret_key(&env.key)
                .map_err(|(_, msg)| AppError::bad_request("INVALID_SECRET_KEY", &msg))?;

            let (encrypted_value, nonce) = state.crypto.encrypt_string(&env.value).map_err(|e| {
                tracing::error!("Failed to encrypt env var '{}' for server {}: {}", env.key, server.id, e);
                AppError::internal("Failed to encrypt environment variable")
            })?;

            SecretRepository::upsert(
                &state.db,
                CreateSecret {
                    server_id: server.id,
                    key: env.key.clone(),
                    encrypted_value,
                    nonce,
                },
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to store env var '{}' for server {}: {}", env.key, server.id, e);
                AppError::internal("Failed to store environment variable")
            })?;
        }
        tracing::info!("Provisioned {} env var(s) for server {}", env_vars.len(), server.id);
    }

    // NOTE: OAuth clients are intentionally NOT auto-created on server creation.
    // Users create an OAuth app explicitly from the OAuth apps page when they need
    // one, so enabling auth no longer silently provisions a Claude OAuth client.

    // Auto-deploy: Trigger initial deployment after server creation
    tracing::info!("Auto-deploying new server {}", server.id);

    // Get latest commit SHA from GitHub
    let commit_sha = if let (Some(github), Some(installation_id)) = (&state.github, server.github_installation_id) {
        match github.get_latest_commit(installation_id, owner, repo, &server.github_branch).await {
            Ok(commit) => commit.sha,
            Err(e) => {
                tracing::warn!("Failed to get commit SHA from GitHub: {}, using HEAD", e);
                "HEAD".to_string()
            }
        }
    } else {
        // No GitHub App - try to get commit via public API
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let url = format!(
            "https://api.github.com/repos/{}/{}/commits/{}",
            owner, repo, server.github_branch
        );
        match client
            .get(&url)
            .header("User-Agent", "MCP-Cloud")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                #[derive(serde::Deserialize)]
                struct CommitResponse { sha: String }
                resp.json::<CommitResponse>()
                    .await
                    .map(|c| c.sha)
                    .unwrap_or_else(|_| "HEAD".to_string())
            }
            _ => "HEAD".to_string(),
        }
    };

    // Create deployment record
    let deployment = DeploymentRepository::create(
        &state.db,
        CreateDeployment {
            server_id: server.id,
            commit_sha: commit_sha.clone(),
            deployed_by: Some(auth_user.user_id),
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to create auto-deployment: {}", e);
        AppError::internal("Server created but auto-deployment failed. Please deploy manually.")
    })?;

    // Increment deployment count cache
    let now = chrono::Utc::now();
    state.cache.increment_deployment_count(workspace_id, now.year(), now.month()).await;

    // Update server status to building
    ServerRepository::update_status(&state.db, server.id, mcp_common::types::ServerStatus::Building, None)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update server status: {}", e);
            AppError::internal("Server created but status update failed.")
        })?;

    // Enqueue build job
    let build_job = mcp_queue::BuildJob::from_server(
        &server,
        deployment.id,
        deployment.commit_sha.clone(),
        None,
    );

    state
        .job_queue
        .push_build_job(build_job)
        .await
        .map_err(|e| {
            tracing::error!("Failed to enqueue auto-deploy build job: {}", e);
            AppError::internal("Server created but build job failed to enqueue.")
        })?;

    tracing::info!("Auto-deploy build job enqueued for server {} (deployment {})", server.id, deployment.id);

    // Return server response with building status
    let runtime = server.runtime();
    let visibility = server.visibility();
    let access_mode = server.access_mode();
    let transport = server.transport();
    Ok(Json(ServerResponse {
        id: server.id,
        workspace_id: server.workspace_id,
        name: server.name,
        slug: server.slug,
        description: server.description,
        github_repo: server.github_repo,
        github_branch: server.github_branch,
        runtime,
        visibility,
        access_mode,
        transport,
        status: mcp_common::types::ServerStatus::Building, // Always building after creation
        endpoint_url: server.endpoint_url,
        region: server.region,
        root_directory: server.root_directory,
        mcp_path: server.mcp_path,
        entry_command: server.entry_command,
        build_command: server.build_command,
        auth_enabled: server.auth_enabled,
        memory_mb: server.memory_mb,
        port: server.port,
        tool_list_filter_by_scope: server.tool_list_filter_by_scope,
        tool_schema_slim: server.tool_schema_slim,
        tool_search_mode: server.tool_search_mode,
        tool_code_mode: server.tool_code_mode,
        upstream_oauth_provider: server.upstream_oauth_provider,
        upstream_oauth_scopes: server.upstream_oauth_scopes,
        upstream_oauth_authorization_url: server.upstream_oauth_authorization_url,
        upstream_oauth_token_url: server.upstream_oauth_token_url,
        upstream_oauth_client_id: server.upstream_oauth_client_id,
        created_at: server.created_at,
        updated_at: server.updated_at,
    }))
}

pub async fn get(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(path): Path<ServerPath>,
) -> Result<Json<ServerResponse>, AppError> {
    // Check membership
    workspace::require_member(&state.db, path.workspace_id, auth_user.user_id).await?;

    let server = ServerRepository::find_by_id(&state.db, path.server_id)
        .await?
        .ok_or_else(|| AppError::not_found("Server"))?;

    if server.workspace_id != path.workspace_id {
        return Err(AppError::not_found("Server"));
    }

    let runtime = server.runtime();
    let visibility = server.visibility();
    let access_mode = server.access_mode();
    let transport = server.transport();
    let status = server.status();
    Ok(Json(ServerResponse {
        id: server.id,
        workspace_id: server.workspace_id,
        name: server.name,
        slug: server.slug,
        description: server.description,
        github_repo: server.github_repo,
        github_branch: server.github_branch,
        runtime,
        visibility,
        access_mode,
        transport,
        status,
        endpoint_url: server.endpoint_url,
        region: server.region,
        root_directory: server.root_directory,
        mcp_path: server.mcp_path,
        entry_command: server.entry_command,
        build_command: server.build_command,
        auth_enabled: server.auth_enabled,
        memory_mb: server.memory_mb,
        port: server.port,
        tool_list_filter_by_scope: server.tool_list_filter_by_scope,
        tool_schema_slim: server.tool_schema_slim,
        tool_search_mode: server.tool_search_mode,
        tool_code_mode: server.tool_code_mode,
        upstream_oauth_provider: server.upstream_oauth_provider,
        upstream_oauth_scopes: server.upstream_oauth_scopes,
        upstream_oauth_authorization_url: server.upstream_oauth_authorization_url,
        upstream_oauth_token_url: server.upstream_oauth_token_url,
        upstream_oauth_client_id: server.upstream_oauth_client_id,
        created_at: server.created_at,
        updated_at: server.updated_at,
    }))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(path): Path<ServerPath>,
    Json(body): Json<UpdateServerRequest>,
) -> Result<Json<ServerResponse>, AppError> {
    // Check membership and write permission
    workspace::require_write_access(&state.db, path.workspace_id, auth_user.user_id).await?;

    // SECURITY: Validate input (e.g. port range) using the validator crate.
    use validator::Validate;
    if let Err(validation_errors) = body.validate() {
        let error_messages: Vec<String> = validation_errors
            .field_errors()
            .iter()
            .flat_map(|(field, errors)| {
                errors.iter().map(move |e| {
                    format!("{}: {}", field, e.message.as_ref().map(|m| m.to_string()).unwrap_or_else(|| e.code.to_string()))
                })
            })
            .collect();
        return Err(AppError::bad_request(
            "VALIDATION_ERROR",
            &error_messages.join(", "),
        ).with_details(json!({
            "errors": error_messages
        })));
    }

    // Verify server belongs to this workspace
    let existing = ServerRepository::find_by_id(&state.db, path.server_id)
        .await?
        .ok_or_else(|| AppError::not_found("Server"))?;

    if existing.workspace_id != path.workspace_id {
        return Err(AppError::not_found("Server"));
    }

    // Validate a memory change against the workspace plan before applying it.
    if body.memory_mb.is_some() {
        let workspace = WorkspaceRepository::find_by_id(&state.db, path.workspace_id)
            .await
            .map_err(|e| {
                tracing::error!("Database error fetching workspace: {}", e);
                AppError::internal("Failed to fetch workspace")
            })?
            .ok_or_else(|| AppError::not_found("Workspace not found"))?;
        let billing_plan = match workspace.plan.as_str() {
            "pro" => BillingPlan::Pro,
            "team" => BillingPlan::Team,
            "enterprise" => BillingPlan::Enterprise,
            _ => BillingPlan::Free,
        };
        check_memory_choice(body.memory_mb, &billing_plan.limits(), &workspace.plan)?;
    }

    tracing::info!(
        "Updating server {}: entry_command={:?}",
        path.server_id,
        body.entry_command
    );

    let server = ServerRepository::update(
        &state.db,
        path.server_id,
        UpdateServer {
            name: body.name,
            description: body.description,
            github_branch: body.github_branch,
            visibility: body.visibility,
            access_mode: body.access_mode,
            status: None,
            endpoint_url: None,
            region: None, // Region is fixed to iad
            root_directory: body.root_directory,
            mcp_path: body.mcp_path,
            entry_command: body.entry_command,
            build_command: body.build_command,
            auth_enabled: body.auth_enabled,
            memory_mb: body.memory_mb,
            port: body.port,
            tool_list_filter_by_scope: body.tool_list_filter_by_scope,
            tool_schema_slim: body.tool_schema_slim,
            tool_search_mode: body.tool_search_mode,
            tool_code_mode: body.tool_code_mode,
            upstream_oauth_provider: body.upstream_oauth_provider,
            upstream_oauth_scopes: body.upstream_oauth_scopes,
            upstream_oauth_authorization_url: body.upstream_oauth_authorization_url,
            upstream_oauth_token_url: body.upstream_oauth_token_url,
            upstream_oauth_client_id: body.upstream_oauth_client_id,
            upstream_oauth_client_secret: body.upstream_oauth_client_secret,
        },
    )
    .await?;

    let runtime = server.runtime();
    let visibility = server.visibility();
    let access_mode = server.access_mode();
    let transport = server.transport();
    let status = server.status();
    Ok(Json(ServerResponse {
        id: server.id,
        workspace_id: server.workspace_id,
        name: server.name,
        slug: server.slug,
        description: server.description,
        github_repo: server.github_repo,
        github_branch: server.github_branch,
        runtime,
        visibility,
        access_mode,
        transport,
        status,
        endpoint_url: server.endpoint_url,
        region: server.region,
        root_directory: server.root_directory,
        mcp_path: server.mcp_path,
        entry_command: server.entry_command,
        build_command: server.build_command,
        auth_enabled: server.auth_enabled,
        memory_mb: server.memory_mb,
        port: server.port,
        tool_list_filter_by_scope: server.tool_list_filter_by_scope,
        tool_schema_slim: server.tool_schema_slim,
        tool_search_mode: server.tool_search_mode,
        tool_code_mode: server.tool_code_mode,
        upstream_oauth_provider: server.upstream_oauth_provider,
        upstream_oauth_scopes: server.upstream_oauth_scopes,
        upstream_oauth_authorization_url: server.upstream_oauth_authorization_url,
        upstream_oauth_token_url: server.upstream_oauth_token_url,
        upstream_oauth_client_id: server.upstream_oauth_client_id,
        created_at: server.created_at,
        updated_at: server.updated_at,
    }))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(path): Path<ServerPath>,
) -> Result<StatusCode, AppError> {
    // Check membership and admin permission (only owner/admin can delete)
    workspace::require_admin(&state.db, path.workspace_id, auth_user.user_id).await?;

    // Verify server belongs to this workspace
    let existing = ServerRepository::find_by_id(&state.db, path.server_id)
        .await?
        .ok_or_else(|| AppError::not_found("Server"))?;

    if existing.workspace_id != path.workspace_id {
        return Err(AppError::not_found("Server"));
    }

    // Soft-delete: mark the row `deleting` instead of hard-deleting it up front. The old
    // flow deleted the DB row BEFORE the Fly teardown succeeded, so a transient Fly error
    // left a permanently orphaned (billable) app with no record. Now the row survives until
    // the builder confirms the app is gone (or a sweeper re-drives the teardown).
    ServerRepository::mark_deleting(&state.db, path.server_id).await?;

    // Tear down the Fly.io app via a durable, retrying job. Use the PERSISTED app name so
    // we never recompute it from a truncated UUID prefix (which collided across tenants and
    // could delete another tenant's app).
    let app_name = existing.fly_app_name.clone();

    let destroy_job = mcp_queue::DestroyJob {
        server_id: path.server_id,
        app_name: app_name.clone(),
    };
    if let Err(e) = state.job_queue.push_destroy_job(destroy_job).await {
        // Queue is down — fall back to an inline destroy so we still try right now.
        tracing::warn!("Failed to enqueue destroy job for {}: {}; trying inline", app_name, e);
        if let Some(ref fly_runtime) = state.fly_runtime {
            if let Err(e) = fly_runtime.destroy_app(&app_name).await {
                tracing::warn!(
                    "Inline destroy of Fly.io app {} for server {} also failed: {}",
                    app_name, path.server_id, e
                );
            }
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn deploy(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(path): Path<ServerPath>,
) -> Result<Json<mcp_common::types::DeploymentResponse>, AppError> {
    // Check membership and write permission
    workspace::require_write_access(&state.db, path.workspace_id, auth_user.user_id).await?;

    // Get workspace to check plan limits
    let workspace = WorkspaceRepository::find_by_id(&state.db, path.workspace_id)
        .await?
        .ok_or_else(|| AppError::not_found("Workspace"))?;

    // Check deployment limits for this month
    let billing_plan = match workspace.plan.as_str() {
        "pro" => BillingPlan::Pro,
        "team" => BillingPlan::Team,
        "enterprise" => BillingPlan::Enterprise,
        _ => BillingPlan::Free,
    };
    let limits = billing_plan.limits();

    // Get deployment count for current month (try cache first)
    let now = chrono::Utc::now();
    let year = now.year();
    let month = now.month();

    let deployments_this_month = if let Some(cached_count) = state.cache.get_deployment_count(path.workspace_id, year, month).await {
        cached_count
    } else {
        // Cache miss - fetch from database
        let month_start = chrono::NaiveDate::from_ymd_opt(year, month, 1)
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|dt| dt.and_utc())
            .unwrap_or(now);

        let count = mcp_db::DeploymentRepository::count_by_workspace_since(
            &state.db,
            path.workspace_id,
            month_start,
        )
        .await?;

        // Cache the count
        state.cache.set_deployment_count(path.workspace_id, year, month, count).await;
        count
    };

    if deployments_this_month >= limits.max_deployments_per_month as i64 {
        return Err(AppError::payment_required(
            "DEPLOYMENT_LIMIT_REACHED",
            &format!(
                "You have reached the maximum number of deployments ({}) for your {} plan this month. Please upgrade to deploy more.",
                limits.max_deployments_per_month,
                workspace.plan
            ),
        ).with_details(json!({
            "current_count": deployments_this_month,
            "max_allowed": limits.max_deployments_per_month,
            "plan": workspace.plan,
            "upgrade_url": "/dashboard/billing"
        })));
    }

    // Get server
    let server = ServerRepository::find_by_id(&state.db, path.server_id)
        .await?
        .ok_or_else(|| AppError::not_found("Server"))?;

    if server.workspace_id != path.workspace_id {
        return Err(AppError::not_found("Server"));
    }

    // Parse owner/repo from github_repo (avoid Vec allocation)
    let (owner, repo) = {
        let mut parts = server.github_repo.split('/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(o), Some(r), None) => (o, r),
            _ => return Err(AppError::bad_request("INVALID_GITHUB_REPO", "Invalid github_repo format")),
        }
    };

    // Get latest commit SHA from GitHub
    let commit_sha = if let (Some(github), Some(installation_id)) = (&state.github, server.github_installation_id) {
        match github.get_latest_commit(installation_id, owner, repo, &server.github_branch).await {
            Ok(commit) => commit.sha,
            Err(e) => {
                tracing::warn!("Failed to get commit SHA from GitHub: {}, using HEAD", e);
                "HEAD".to_string()
            }
        }
    } else {
        // No GitHub App - try to get commit via public API
        // SECURITY: Configure HTTP client with timeout and redirect policy
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let url = format!(
            "https://api.github.com/repos/{}/{}/commits/{}",
            owner, repo, server.github_branch
        );
        match client
            .get(&url)
            .header("User-Agent", "MCP-Cloud")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                #[derive(serde::Deserialize)]
                struct CommitResponse { sha: String }
                resp.json::<CommitResponse>()
                    .await
                    .map(|c| c.sha)
                    .unwrap_or_else(|_| "HEAD".to_string())
            }
            _ => "HEAD".to_string(),
        }
    };

    // Create deployment record
    let deployment = mcp_db::DeploymentRepository::create(
        &state.db,
        mcp_db::CreateDeployment {
            server_id: path.server_id,
            commit_sha: commit_sha.clone(),
            deployed_by: Some(auth_user.user_id),
        },
    )
    .await?;

    // Increment deployment count cache
    state.cache.increment_deployment_count(path.workspace_id, year, month).await;

    // Update server status
    ServerRepository::update_status(&state.db, path.server_id, mcp_common::types::ServerStatus::Building, None)
        .await?;

    // Enqueue build job
    tracing::info!(
        "Creating build job for server {}: entry_command={:?}",
        path.server_id,
        server.entry_command
    );

    let build_job = mcp_queue::BuildJob::from_server(
        &server,
        deployment.id,
        deployment.commit_sha.clone(),
        None,
    );

    if let Err(e) = state.job_queue.push_build_job(build_job).await {
        tracing::error!("Failed to enqueue build job for deployment {}: {:?}", deployment.id, e);
        return Err(AppError::internal(&format!("Failed to enqueue build job: {}", e)));
    }

    tracing::info!("Build job enqueued for deployment {}", deployment.id);

    let status = deployment.status();
    let build_duration_seconds = deployment.finished_at.map(|f| (f - deployment.started_at).num_seconds());
    Ok(Json(mcp_common::types::DeploymentResponse {
        id: deployment.id,
        server_id: deployment.server_id,
        version: deployment.version,
        commit_sha: deployment.commit_sha,
        status,
        error_message: deployment.error_message,
        build_logs: deployment.build_logs,
        started_at: deployment.started_at,
        finished_at: deployment.finished_at,
        created_at: deployment.started_at,
        deployed_at: deployment.finished_at,
        build_duration_seconds,
    }))
}

pub async fn stop(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(path): Path<ServerPath>,
) -> Result<Json<ServerResponse>, AppError> {
    // Check membership and permission
    let member = WorkspaceRepository::get_member(&state.db, path.workspace_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::forbidden("Not a member of this workspace"))?;

    if matches!(member.role(), mcp_common::types::WorkspaceRole::Viewer) {
        return Err(AppError::forbidden("Insufficient permissions"));
    }

    // Verify server belongs to workspace
    let server = ServerRepository::find_by_id(&state.db, path.server_id)
        .await?
        .ok_or_else(|| AppError::not_found("Server"))?;

    if server.workspace_id != path.workspace_id {
        return Err(AppError::not_found("Server"));
    }

    if !server.is_running() {
        return Err(AppError::bad_request("SERVER_NOT_RUNNING", "Server is not running"));
    }

    // Update server status to stopped
    ServerRepository::update_status(
        &state.db,
        path.server_id,
        mcp_common::types::ServerStatus::Stopped,
        None,
    )
    .await?;

    // Get updated server
    let server = ServerRepository::find_by_id(&state.db, path.server_id)
        .await?
        .ok_or_else(|| AppError::not_found("Server"))?;

    let runtime = server.runtime();
    let visibility = server.visibility();
    let access_mode = server.access_mode();
    let transport = server.transport();
    let status = server.status();
    Ok(Json(ServerResponse {
        id: server.id,
        workspace_id: server.workspace_id,
        name: server.name,
        slug: server.slug,
        description: server.description,
        github_repo: server.github_repo,
        github_branch: server.github_branch,
        runtime,
        visibility,
        access_mode,
        transport,
        status,
        endpoint_url: server.endpoint_url,
        region: server.region,
        root_directory: server.root_directory,
        mcp_path: server.mcp_path,
        entry_command: server.entry_command,
        build_command: server.build_command,
        auth_enabled: server.auth_enabled,
        memory_mb: server.memory_mb,
        port: server.port,
        tool_list_filter_by_scope: server.tool_list_filter_by_scope,
        tool_schema_slim: server.tool_schema_slim,
        tool_search_mode: server.tool_search_mode,
        tool_code_mode: server.tool_code_mode,
        upstream_oauth_provider: server.upstream_oauth_provider,
        upstream_oauth_scopes: server.upstream_oauth_scopes,
        upstream_oauth_authorization_url: server.upstream_oauth_authorization_url,
        upstream_oauth_token_url: server.upstream_oauth_token_url,
        upstream_oauth_client_id: server.upstream_oauth_client_id,
        created_at: server.created_at,
        updated_at: server.updated_at,
    }))
}

pub async fn restart(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(path): Path<ServerPath>,
) -> Result<Json<mcp_common::types::DeploymentResponse>, AppError> {
    // Check membership and permission
    let member = WorkspaceRepository::get_member(&state.db, path.workspace_id, auth_user.user_id)
        .await?
        .ok_or_else(|| AppError::forbidden("Not a member of this workspace"))?;

    if matches!(member.role(), mcp_common::types::WorkspaceRole::Viewer) {
        return Err(AppError::forbidden("Insufficient permissions"));
    }

    // Verify server belongs to workspace
    let server = ServerRepository::find_by_id(&state.db, path.server_id)
        .await?
        .ok_or_else(|| AppError::not_found("Server"))?;

    if server.workspace_id != path.workspace_id {
        return Err(AppError::not_found("Server"));
    }

    // For restart, we trigger a new deployment
    // This reuses the deploy logic
    deploy(State(state), auth_user, Path(path)).await
}

/// Get server metrics (CPU, memory, network)
pub async fn metrics(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(path): Path<ServerPath>,
) -> Result<Json<mcp_container::AppMetrics>, AppError> {
    // Check membership
    workspace::require_member(&state.db, path.workspace_id, auth_user.user_id).await?;

    // Get server
    let server = ServerRepository::find_by_id(&state.db, path.server_id)
        .await?
        .ok_or_else(|| AppError::not_found("Server"))?;

    if server.workspace_id != path.workspace_id {
        return Err(AppError::not_found("Server"));
    }

    // Get Fly.io runtime
    let fly_runtime = state.fly_runtime.as_ref().ok_or_else(|| {
        AppError::internal("Fly.io runtime not configured")
    })?;

    // Extract app_name from endpoint_url
    let app_name = server
        .endpoint_url
        .as_ref()
        .and_then(|url| {
            url.replace("https://", "")
                .replace("http://", "")
                .split('.')
                .next()
                .map(|s| s.to_string())
        })
        .ok_or_else(|| AppError::bad_request("NO_ENDPOINT", "Server has no endpoint URL"))?;

    // Get metrics from Fly.io
    let metrics = fly_runtime
        .get_metrics(&app_name)
        .await
        .map_err(|e| AppError::internal(&format!("Failed to get metrics: {}", e)))?;

    Ok(Json(metrics))
}

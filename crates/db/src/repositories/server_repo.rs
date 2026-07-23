use crate::models::{CreateServer, McpServer, UpdateServer};
use mcp_common::types::ServerStatus;
use mcp_common::Result;
use sqlx::PgPool;
use uuid::Uuid;

const SERVER_COLS: &str = "id, workspace_id, name, slug, description, github_repo, \
    github_branch, github_installation_id, runtime, visibility, access_mode, transport, \
    status, endpoint_url, rate_limit_per_minute, region, root_directory, mcp_path, \
    entry_command, build_command, auth_enabled, memory_mb, port, container_name, \
    tool_list_filter_by_scope, tool_schema_slim, tool_search_mode, tool_code_mode, \
    upstream_oauth_provider, upstream_oauth_scopes, upstream_oauth_authorization_url, \
    upstream_oauth_token_url, upstream_oauth_client_id, upstream_oauth_client_secret, \
    created_at, updated_at";

const SERVER_COLS_S: &str = "s.id, s.workspace_id, s.name, s.slug, s.description, s.github_repo, \
    s.github_branch, s.github_installation_id, s.runtime, s.visibility, s.access_mode, s.transport, \
    s.status, s.endpoint_url, s.rate_limit_per_minute, s.region, s.root_directory, s.mcp_path, \
    s.entry_command, s.build_command, s.auth_enabled, s.memory_mb, s.port, s.container_name, \
    s.tool_list_filter_by_scope, s.tool_schema_slim, s.tool_search_mode, s.tool_code_mode, \
    s.upstream_oauth_provider, s.upstream_oauth_scopes, s.upstream_oauth_authorization_url, \
    s.upstream_oauth_token_url, s.upstream_oauth_client_id, s.upstream_oauth_client_secret, \
    s.created_at, s.updated_at";

pub struct ServerRepository;

impl ServerRepository {
    /// Maximum servers to return in a single query
    const MAX_SERVERS_PER_QUERY: i64 = 200;
    pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<McpServer>> {
        let server = sqlx::query_as::<_, McpServer>(
            &format!("SELECT {SERVER_COLS} FROM mcp_servers WHERE id = $1"),
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(server)
    }

    pub async fn find_by_slug(
        pool: &PgPool,
        workspace_id: Uuid,
        slug: &str,
    ) -> Result<Option<McpServer>> {
        let server = sqlx::query_as::<_, McpServer>(
            &format!("SELECT {SERVER_COLS} FROM mcp_servers WHERE workspace_id = $1 AND slug = $2"),
        )
        .bind(workspace_id)
        .bind(slug)
        .fetch_optional(pool)
        .await?;

        Ok(server)
    }

    /// Resolve a running server by its workspace slug + server slug.
    ///
    /// The proxy's public host is `<workspace_slug>--<server_slug>.<base_domain>`.
    /// A server slug is only unique *within a workspace* (`UNIQUE (workspace_id, slug)`),
    /// so slug alone is ambiguous across workspaces; the workspace slug — which IS globally
    /// unique (`workspaces.slug UNIQUE`) — disambiguates, making this lookup deterministic.
    /// Access control is handled downstream by API key / OAuth validation.
    pub async fn find_by_workspace_and_server_slug(
        pool: &PgPool,
        workspace_slug: &str,
        server_slug: &str,
    ) -> Result<Option<McpServer>> {
        let server = sqlx::query_as::<_, McpServer>(
            &format!(
                "SELECT {SERVER_COLS_S} FROM mcp_servers s \
                 JOIN workspaces w ON w.id = s.workspace_id \
                 WHERE w.slug = $1 AND s.slug = $2 AND s.status = 'running'"
            ),
        )
        .bind(workspace_slug)
        .bind(server_slug)
        .fetch_optional(pool)
        .await?;

        Ok(server)
    }

    pub async fn list_by_workspace(
        pool: &PgPool,
        workspace_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<McpServer>> {
        let servers = sqlx::query_as::<_, McpServer>(
            &format!(
                "SELECT {SERVER_COLS} FROM mcp_servers \
                 WHERE workspace_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
            ),
        )
        .bind(workspace_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok(servers)
    }

    /// All servers currently marked `running`, across every workspace. Used by the
    /// usage sampler to know which apps to poll for active machine time.
    pub async fn list_running(pool: &PgPool) -> Result<Vec<McpServer>> {
        let servers = sqlx::query_as::<_, McpServer>(
            &format!("SELECT {SERVER_COLS} FROM mcp_servers WHERE status = 'running' LIMIT 5000"),
        )
        .fetch_all(pool)
        .await?;

        Ok(servers)
    }

    pub async fn count_by_workspace(pool: &PgPool, workspace_id: Uuid) -> Result<i64> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM mcp_servers WHERE workspace_id = $1
            "#,
        )
        .bind(workspace_id)
        .fetch_one(pool)
        .await?;

        Ok(count.0)
    }

    pub async fn create(pool: &PgPool, data: CreateServer) -> Result<McpServer> {
        let runtime_str = data.runtime.to_string();
        let visibility_str = data.visibility.to_string();
        let access_mode_str = data.access_mode.to_string();
        let transport_str = data.transport.to_string();

        // Generate the id in Rust (rather than relying on the DB default) so we can derive
        // the collision-free container name from the SAME id and persist both atomically.
        let id = Uuid::new_v4();
        let container_name = McpServer::new_container_name(id);

        let server = sqlx::query_as::<_, McpServer>(
            &format!(
                "INSERT INTO mcp_servers ( \
                    workspace_id, name, slug, description, github_repo, github_branch, \
                    github_installation_id, runtime, visibility, access_mode, transport, region, \
                    root_directory, mcp_path, entry_command, auth_enabled, build_command, memory_mb, \
                    id, container_name, port, upstream_oauth_provider, upstream_oauth_scopes, \
                    upstream_oauth_authorization_url, upstream_oauth_token_url, \
                    upstream_oauth_client_id, upstream_oauth_client_secret \
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, \
                    $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27) \
                RETURNING {SERVER_COLS}"
            ),
        )
        .bind(data.workspace_id)
        .bind(&data.name)
        .bind(&data.slug)
        .bind(&data.description)
        .bind(&data.github_repo)
        .bind(&data.github_branch)
        .bind(data.github_installation_id)
        .bind(runtime_str)
        .bind(visibility_str)
        .bind(access_mode_str)
        .bind(transport_str)
        .bind(&data.region)
        .bind(&data.root_directory)
        .bind(&data.mcp_path)
        .bind(&data.entry_command)
        .bind(data.auth_enabled)
        .bind(&data.build_command)
        .bind(data.memory_mb)
        .bind(id)
        .bind(&container_name)
        .bind(data.port)
        .bind(&data.upstream_oauth_provider)
        .bind(&data.upstream_oauth_scopes)
        .bind(&data.upstream_oauth_authorization_url)
        .bind(&data.upstream_oauth_token_url)
        .bind(&data.upstream_oauth_client_id)
        .bind(&data.upstream_oauth_client_secret)
        .fetch_one(pool)
        .await?;

        Ok(server)
    }

    pub async fn update(pool: &PgPool, id: Uuid, data: UpdateServer) -> Result<McpServer> {
        let access_mode_str = data.access_mode.map(|a| a.to_string());
        let status_str = data.status.map(|s| s.to_string());
        let visibility_str = data.visibility.map(|v| v.to_string());

        let server = sqlx::query_as::<_, McpServer>(
            &format!(
                "UPDATE mcp_servers SET \
                    name = COALESCE($2, name), \
                    description = COALESCE($3, description), \
                    github_branch = COALESCE($4, github_branch), \
                    visibility = COALESCE($5, visibility), \
                    access_mode = COALESCE($6, access_mode), \
                    status = COALESCE($7, status), \
                    endpoint_url = COALESCE($8, endpoint_url), \
                    region = COALESCE($9, region), \
                    root_directory = COALESCE($10, root_directory), \
                    mcp_path = COALESCE($11, mcp_path), \
                    entry_command = COALESCE($12, entry_command), \
                    auth_enabled = COALESCE($13, auth_enabled), \
                    build_command = COALESCE($14, build_command), \
                    memory_mb = COALESCE($15, memory_mb), \
                    port = COALESCE($16, port), \
                    tool_list_filter_by_scope = COALESCE($17, tool_list_filter_by_scope), \
                    tool_schema_slim = COALESCE($18, tool_schema_slim), \
                    tool_search_mode = COALESCE($19, tool_search_mode), \
                    tool_code_mode = COALESCE($20, tool_code_mode), \
                    upstream_oauth_provider = COALESCE($21, upstream_oauth_provider), \
                    upstream_oauth_scopes = COALESCE($22, upstream_oauth_scopes), \
                    upstream_oauth_authorization_url = COALESCE($23, upstream_oauth_authorization_url), \
                    upstream_oauth_token_url = COALESCE($24, upstream_oauth_token_url), \
                    upstream_oauth_client_id = COALESCE($25, upstream_oauth_client_id), \
                    upstream_oauth_client_secret = COALESCE($26, upstream_oauth_client_secret), \
                    updated_at = NOW() \
                WHERE id = $1 RETURNING {SERVER_COLS}"
            ),
        )
        .bind(id)
        .bind(&data.name)
        .bind(&data.description)
        .bind(&data.github_branch)
        .bind(visibility_str)
        .bind(access_mode_str)
        .bind(status_str)
        .bind(&data.endpoint_url)
        .bind(&data.region)
        .bind(&data.root_directory)
        .bind(&data.mcp_path)
        .bind(&data.entry_command)
        .bind(data.auth_enabled)
        .bind(&data.build_command)
        .bind(data.memory_mb)
        .bind(data.port)
        .bind(data.tool_list_filter_by_scope)
        .bind(data.tool_schema_slim)
        .bind(data.tool_search_mode)
        .bind(data.tool_code_mode)
        .bind(&data.upstream_oauth_provider)
        .bind(&data.upstream_oauth_scopes)
        .bind(&data.upstream_oauth_authorization_url)
        .bind(&data.upstream_oauth_token_url)
        .bind(&data.upstream_oauth_client_id)
        .bind(&data.upstream_oauth_client_secret)
        .fetch_one(pool)
        .await?;

        Ok(server)
    }

    pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM mcp_servers WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Soft-delete: mark the server `deleting` so its Fly app teardown can be confirmed
    /// before the row is hard-deleted. The row stays so the sweeper can re-drive teardown
    /// if the worker dies mid-destroy (otherwise a transient failure orphans the Fly app).
    pub async fn mark_deleting(pool: &PgPool, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE mcp_servers
            SET status = 'deleting', updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Servers stuck in `deleting` for longer than `older_than_minutes` — their destroy job
    /// likely never confirmed (worker crash, transient Fly error). The sweeper re-drives the
    /// teardown (idempotent: destroying a missing app is a no-op) and hard-deletes on success.
    pub async fn list_deleting_older_than(
        pool: &PgPool,
        older_than_minutes: i64,
    ) -> Result<Vec<McpServer>> {
        let servers = sqlx::query_as::<_, McpServer>(
            &format!(
                "SELECT {SERVER_COLS} FROM mcp_servers \
                 WHERE status = 'deleting' AND updated_at < NOW() - ($1 * interval '1 minute') \
                 LIMIT 1000"
            ),
        )
        .bind(older_than_minutes)
        .fetch_all(pool)
        .await?;

        Ok(servers)
    }

    pub async fn update_status(
        pool: &PgPool,
        id: Uuid,
        status: ServerStatus,
        endpoint_url: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE mcp_servers
            SET status = $2, endpoint_url = COALESCE($3, endpoint_url), updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(status.to_string())
        .bind(endpoint_url)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// List all servers for a user across all workspaces (prevents N+1)
    /// Note: For users with many servers, use list_all_by_user_paginated instead
    pub async fn list_all_by_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<McpServer>> {
        Self::list_all_by_user_paginated(pool, user_id, Self::MAX_SERVERS_PER_QUERY, 0).await
    }

    /// List servers for a user with pagination support
    /// Returns (servers, has_more) where has_more indicates if there are more pages
    pub async fn list_all_by_user_paginated(
        pool: &PgPool,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<McpServer>> {
        // Cap limit to prevent abuse
        let effective_limit = limit.min(Self::MAX_SERVERS_PER_QUERY);

        let servers = sqlx::query_as::<_, McpServer>(
            &format!(
                "SELECT DISTINCT {SERVER_COLS_S} FROM mcp_servers s \
                 INNER JOIN workspace_members wm ON s.workspace_id = wm.workspace_id \
                 WHERE wm.user_id = $1 ORDER BY s.created_at DESC LIMIT $2 OFFSET $3"
            ),
        )
        .bind(user_id)
        .bind(effective_limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok(servers)
    }

    /// Count total servers for a user across all workspaces
    pub async fn count_all_by_user(pool: &PgPool, user_id: Uuid) -> Result<i64> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(DISTINCT s.id)
            FROM mcp_servers s
            INNER JOIN workspace_members wm ON s.workspace_id = wm.workspace_id
            WHERE wm.user_id = $1
            "#,
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?;

        Ok(count.0)
    }

    /// Check if a user has access to a server (optimized single query with JOIN)
    /// Returns true if the server exists, belongs to the workspace, and the user is a member
    pub async fn check_user_access(
        pool: &PgPool,
        server_id: Uuid,
        workspace_id: Uuid,
        user_id: Uuid,
    ) -> Result<bool> {
        let result: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT 1
            FROM mcp_servers s
            INNER JOIN workspace_members wm ON s.workspace_id = wm.workspace_id
            WHERE s.id = $1 AND s.workspace_id = $2 AND wm.user_id = $3
            LIMIT 1
            "#,
        )
        .bind(server_id)
        .bind(workspace_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;

        Ok(result.is_some())
    }

    /// Get platform-wide statistics for public display
    pub async fn get_platform_stats(pool: &PgPool) -> Result<PlatformStats> {
        let row: (i64, i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*) as total_servers,
                COUNT(*) FILTER (WHERE status = 'running') as running_servers,
                COUNT(DISTINCT workspace_id) as total_workspaces
            FROM mcp_servers
            "#,
        )
        .fetch_one(pool)
        .await?;

        Ok(PlatformStats {
            total_servers: row.0,
            running_servers: row.1,
            total_workspaces: row.2,
        })
    }

    /// List running servers that have an upstream OAuth provider configured.
    /// Used by the background token refresh job.
    pub async fn list_running_with_upstream_oauth(pool: &PgPool) -> Result<Vec<McpServer>> {
        let servers = sqlx::query_as::<_, McpServer>(
            &format!(
                "SELECT {SERVER_COLS} FROM mcp_servers \
                 WHERE status = 'running' AND upstream_oauth_provider IS NOT NULL LIMIT 5000"
            ),
        )
        .fetch_all(pool)
        .await?;

        Ok(servers)
    }
}

/// Platform-wide statistics
#[derive(Debug, Clone)]
pub struct PlatformStats {
    pub total_servers: i64,
    pub running_servers: i64,
    pub total_workspaces: i64,
}

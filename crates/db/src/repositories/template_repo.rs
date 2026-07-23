use crate::models::{CreateServerTemplate, ServerTemplate};
use mcp_common::Result;
use sqlx::PgPool;
use uuid::Uuid;

pub struct ServerTemplateRepository;

/// Columns shared by all SELECT queries on server_templates.
const COLS: &str = "
    st.id, st.server_id, st.owner_id, st.workspace_id, st.name, st.description,
    st.github_repo, st.github_branch, st.runtime, st.transport, st.mcp_path,
    st.entry_command, st.build_command, st.root_directory, st.auth_enabled,
    st.memory_mb, st.port, st.upstream_oauth_provider, st.upstream_oauth_scopes,
    st.required_env_var_keys, st.use_count, st.created_at, st.updated_at
";

impl ServerTemplateRepository {
    /// List published templates whose backing server is still public.
    /// Secondary sort by id ensures stable pagination.
    pub async fn list_public(pool: &PgPool, limit: i64, offset: i64) -> Result<Vec<ServerTemplate>> {
        let query = format!(
            r#"
            SELECT {COLS}
            FROM server_templates st
            JOIN mcp_servers s ON s.id = st.server_id
            WHERE s.visibility = 'public'
            ORDER BY st.created_at DESC, st.id DESC
            LIMIT $1 OFFSET $2
            "#
        );
        let templates = sqlx::query_as::<_, ServerTemplate>(&query)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?;
        Ok(templates)
    }

    /// Fetch a single template by its own ID, only if the backing server is still public.
    pub async fn find_public_by_id(pool: &PgPool, id: Uuid) -> Result<Option<ServerTemplate>> {
        let query = format!(
            r#"
            SELECT {COLS}
            FROM server_templates st
            JOIN mcp_servers s ON s.id = st.server_id
            WHERE st.id = $1 AND s.visibility = 'public'
            "#
        );
        let template = sqlx::query_as::<_, ServerTemplate>(&query)
            .bind(id)
            .fetch_optional(pool)
            .await?;
        Ok(template)
    }

    /// Fetch a template by its ID regardless of server visibility (used internally / auth'd endpoints).
    pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<ServerTemplate>> {
        let query = format!(
            r#"
            SELECT {COLS}
            FROM server_templates st
            WHERE st.id = $1
            "#
        );
        let template = sqlx::query_as::<_, ServerTemplate>(&query)
            .bind(id)
            .fetch_optional(pool)
            .await?;
        Ok(template)
    }

    pub async fn find_by_server_id(
        pool: &PgPool,
        server_id: Uuid,
    ) -> Result<Option<ServerTemplate>> {
        let query = format!(
            r#"
            SELECT {COLS}
            FROM server_templates st
            WHERE st.server_id = $1
            "#
        );
        let template = sqlx::query_as::<_, ServerTemplate>(&query)
            .bind(server_id)
            .fetch_optional(pool)
            .await?;
        Ok(template)
    }

    pub async fn upsert(pool: &PgPool, data: CreateServerTemplate) -> Result<ServerTemplate> {
        let template = sqlx::query_as::<_, ServerTemplate>(
            r#"
            INSERT INTO server_templates (
                server_id, owner_id, workspace_id, name, description,
                github_repo, github_branch, runtime, transport, mcp_path,
                entry_command, build_command, root_directory, auth_enabled,
                memory_mb, port, upstream_oauth_provider, upstream_oauth_scopes,
                required_env_var_keys
            )
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)
            ON CONFLICT (server_id) DO UPDATE SET
                name                    = EXCLUDED.name,
                description             = EXCLUDED.description,
                github_repo             = EXCLUDED.github_repo,
                github_branch           = EXCLUDED.github_branch,
                runtime                 = EXCLUDED.runtime,
                transport               = EXCLUDED.transport,
                mcp_path                = EXCLUDED.mcp_path,
                entry_command           = EXCLUDED.entry_command,
                build_command           = EXCLUDED.build_command,
                root_directory          = EXCLUDED.root_directory,
                auth_enabled            = EXCLUDED.auth_enabled,
                memory_mb               = EXCLUDED.memory_mb,
                port                    = EXCLUDED.port,
                upstream_oauth_provider = EXCLUDED.upstream_oauth_provider,
                upstream_oauth_scopes   = EXCLUDED.upstream_oauth_scopes,
                required_env_var_keys   = EXCLUDED.required_env_var_keys,
                updated_at              = NOW()
            RETURNING
                id, server_id, owner_id, workspace_id, name, description,
                github_repo, github_branch, runtime, transport, mcp_path,
                entry_command, build_command, root_directory, auth_enabled,
                memory_mb, port, upstream_oauth_provider, upstream_oauth_scopes,
                required_env_var_keys, use_count, created_at, updated_at
            "#,
        )
        .bind(data.server_id)
        .bind(data.owner_id)
        .bind(data.workspace_id)
        .bind(&data.name)
        .bind(&data.description)
        .bind(&data.github_repo)
        .bind(&data.github_branch)
        .bind(&data.runtime)
        .bind(&data.transport)
        .bind(&data.mcp_path)
        .bind(&data.entry_command)
        .bind(&data.build_command)
        .bind(&data.root_directory)
        .bind(data.auth_enabled)
        .bind(data.memory_mb)
        .bind(data.port)
        .bind(&data.upstream_oauth_provider)
        .bind(&data.upstream_oauth_scopes)
        .bind(&data.required_env_var_keys)
        .fetch_one(pool)
        .await?;

        Ok(template)
    }

    pub async fn delete_by_server_id(pool: &PgPool, server_id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM server_templates WHERE server_id = $1")
            .bind(server_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Increment use_count only if the template actually exists. Returns true when a row was updated.
    pub async fn increment_use_count(pool: &PgPool, id: Uuid) -> Result<bool> {
        let result =
            sqlx::query("UPDATE server_templates SET use_count = use_count + 1 WHERE id = $1")
                .bind(id)
                .execute(pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }
}

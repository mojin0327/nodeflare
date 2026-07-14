use crate::models::{CreateTool, Tool, UpdateTool, UpsertTool};
use mcp_common::Result;
use pgvector::Vector;
use sqlx::PgPool;
use uuid::Uuid;

pub struct ToolRepository;

impl ToolRepository {
    pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Tool>> {
        let tool = sqlx::query_as::<_, Tool>(
            r#"
            SELECT id, server_id, name, description, input_schema, enabled,
                   permission_level, rate_limit_per_minute, created_at, updated_at
            FROM tools
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(tool)
    }

    pub async fn find_by_name(
        pool: &PgPool,
        server_id: Uuid,
        name: &str,
    ) -> Result<Option<Tool>> {
        let tool = sqlx::query_as::<_, Tool>(
            r#"
            SELECT id, server_id, name, description, input_schema, enabled,
                   permission_level, rate_limit_per_minute, created_at, updated_at
            FROM tools
            WHERE server_id = $1 AND name = $2
            "#,
        )
        .bind(server_id)
        .bind(name)
        .fetch_optional(pool)
        .await?;

        Ok(tool)
    }

    /// Maximum tools per server to prevent resource exhaustion
    const MAX_TOOLS_PER_SERVER: i64 = 500;

    pub async fn list_by_server(pool: &PgPool, server_id: Uuid) -> Result<Vec<Tool>> {
        let tools = sqlx::query_as::<_, Tool>(
            r#"
            SELECT id, server_id, name, description, input_schema, enabled,
                   permission_level, rate_limit_per_minute, created_at, updated_at
            FROM tools
            WHERE server_id = $1
            ORDER BY name
            LIMIT $2
            "#,
        )
        .bind(server_id)
        .bind(Self::MAX_TOOLS_PER_SERVER)
        .fetch_all(pool)
        .await?;

        Ok(tools)
    }

    pub async fn list_enabled_by_server(pool: &PgPool, server_id: Uuid) -> Result<Vec<Tool>> {
        let tools = sqlx::query_as::<_, Tool>(
            r#"
            SELECT id, server_id, name, description, input_schema, enabled,
                   permission_level, rate_limit_per_minute, created_at, updated_at
            FROM tools
            WHERE server_id = $1 AND enabled = true
            ORDER BY name
            LIMIT $2
            "#,
        )
        .bind(server_id)
        .bind(Self::MAX_TOOLS_PER_SERVER)
        .fetch_all(pool)
        .await?;

        Ok(tools)
    }

    pub async fn create(pool: &PgPool, data: CreateTool) -> Result<Tool> {
        let tool = sqlx::query_as::<_, Tool>(
            r#"
            INSERT INTO tools (server_id, name, description, input_schema)
            VALUES ($1, $2, $3, $4)
            RETURNING id, server_id, name, description, input_schema, enabled,
                      permission_level, rate_limit_per_minute, created_at, updated_at
            "#,
        )
        .bind(data.server_id)
        .bind(&data.name)
        .bind(&data.description)
        .bind(&data.input_schema)
        .fetch_one(pool)
        .await?;

        Ok(tool)
    }

    pub async fn upsert(pool: &PgPool, data: UpsertTool) -> Result<Tool> {
        let tool = sqlx::query_as::<_, Tool>(
            r#"
            INSERT INTO tools (server_id, name, description, input_schema)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (server_id, name) DO UPDATE SET
                description = EXCLUDED.description,
                input_schema = EXCLUDED.input_schema,
                updated_at = NOW()
            RETURNING id, server_id, name, description, input_schema, enabled,
                      permission_level, rate_limit_per_minute, created_at, updated_at
            "#,
        )
        .bind(data.server_id)
        .bind(&data.name)
        .bind(&data.description)
        .bind(&data.input_schema)
        .fetch_one(pool)
        .await?;

        Ok(tool)
    }

    pub async fn update(pool: &PgPool, id: Uuid, data: UpdateTool) -> Result<Tool> {
        let permission_str = data.permission_level.map(|p| p.to_string());

        let tool = sqlx::query_as::<_, Tool>(
            r#"
            UPDATE tools
            SET
                description = COALESCE($2, description),
                enabled = COALESCE($3, enabled),
                permission_level = COALESCE($4, permission_level),
                rate_limit_per_minute = COALESCE($5, rate_limit_per_minute),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, server_id, name, description, input_schema, enabled,
                      permission_level, rate_limit_per_minute, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&data.description)
        .bind(data.enabled)
        .bind(permission_str)
        .bind(data.rate_limit_per_minute)
        .fetch_one(pool)
        .await?;

        Ok(tool)
    }

    pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM tools WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn delete_by_server(pool: &PgPool, server_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM tools WHERE server_id = $1")
            .bind(server_id)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Sync tools using batch operations to prevent N+1 queries
    /// Limits the number of tools to prevent resource exhaustion
    pub async fn sync_tools(
        pool: &PgPool,
        server_id: Uuid,
        tools: Vec<UpsertTool>,
    ) -> Result<Vec<Tool>> {
        // Limit tools to prevent DoS
        let tools: Vec<UpsertTool> = tools.into_iter().take(Self::MAX_TOOLS_PER_SERVER as usize).collect();

        if tools.is_empty() {
            // Delete all tools for this server
            sqlx::query("DELETE FROM tools WHERE server_id = $1")
                .bind(server_id)
                .execute(pool)
                .await?;
            return Ok(Vec::new());
        }

        // Use a transaction for atomicity
        let mut tx = pool.begin().await?;

        // Collect new tool names
        let new_names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();

        // Batch delete tools that no longer exist (single query)
        sqlx::query(
            r#"
            DELETE FROM tools
            WHERE server_id = $1 AND name != ALL($2)
            "#,
        )
        .bind(server_id)
        .bind(&new_names)
        .execute(&mut *tx)
        .await?;

        // Batch upsert using UNNEST for efficiency (single query)
        let names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        let descriptions: Vec<Option<String>> = tools.iter().map(|t| t.description.clone()).collect();
        let input_schemas: Vec<Option<serde_json::Value>> = tools.iter().map(|t| t.input_schema.clone()).collect();

        let result = sqlx::query_as::<_, Tool>(
            r#"
            INSERT INTO tools (server_id, name, description, input_schema)
            SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::jsonb[])
            ON CONFLICT (server_id, name) DO UPDATE SET
                description = EXCLUDED.description,
                input_schema = EXCLUDED.input_schema,
                -- Invalidate the stored embedding when the description changes so the
                -- proxy re-embeds it; keep it otherwise to avoid needless re-embedding.
                embedding = CASE
                    WHEN tools.description IS DISTINCT FROM EXCLUDED.description THEN NULL
                    ELSE tools.embedding
                END,
                updated_at = NOW()
            RETURNING id, server_id, name, description, input_schema, enabled,
                      permission_level, rate_limit_per_minute, created_at, updated_at
            "#,
        )
        .bind(server_id)
        .bind(&names)
        .bind(&descriptions)
        .bind(&input_schemas)
        .fetch_all(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(result)
    }

    /// Store a tool's embedding vector (search mode / semantic search).
    pub async fn set_embedding(pool: &PgPool, tool_id: Uuid, embedding: Vec<f32>) -> Result<()> {
        sqlx::query("UPDATE tools SET embedding = $2 WHERE id = $1")
            .bind(tool_id)
            .bind(Vector::from(embedding))
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Tools in a server that have no embedding yet (newly observed or description
    /// changed). The proxy backfills these in the background.
    pub async fn list_missing_embeddings(pool: &PgPool, server_id: Uuid) -> Result<Vec<Tool>> {
        let tools = sqlx::query_as::<_, Tool>(
            r#"
            SELECT id, server_id, name, description, input_schema, enabled,
                   permission_level, rate_limit_per_minute, created_at, updated_at
            FROM tools
            WHERE server_id = $1 AND embedding IS NULL
            ORDER BY name
            LIMIT $2
            "#,
        )
        .bind(server_id)
        .bind(Self::MAX_TOOLS_PER_SERVER)
        .fetch_all(pool)
        .await?;

        Ok(tools)
    }

    /// Semantic search: tools in a server ordered by cosine distance to `embedding`.
    /// Only tools that already have an embedding participate.
    pub async fn search_by_embedding(
        pool: &PgPool,
        server_id: Uuid,
        embedding: Vec<f32>,
        limit: i64,
    ) -> Result<Vec<Tool>> {
        let tools = sqlx::query_as::<_, Tool>(
            r#"
            SELECT id, server_id, name, description, input_schema, enabled,
                   permission_level, rate_limit_per_minute, created_at, updated_at
            FROM tools
            WHERE server_id = $1 AND embedding IS NOT NULL
            ORDER BY embedding <=> $2
            LIMIT $3
            "#,
        )
        .bind(server_id)
        .bind(Vector::from(embedding))
        .bind(limit)
        .fetch_all(pool)
        .await?;

        Ok(tools)
    }
}

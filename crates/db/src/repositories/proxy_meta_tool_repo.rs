use crate::models::ProxyMetaTool;
use mcp_common::Result;
use sqlx::PgPool;

pub struct ProxyMetaToolRepository;

impl ProxyMetaToolRepository {
    /// Returns all enabled meta-tool definitions ordered by sort_order.
    pub async fn list_enabled(pool: &PgPool) -> Result<Vec<ProxyMetaTool>> {
        let rows = sqlx::query_as::<_, ProxyMetaTool>(
            r#"
            SELECT handler_type, name, description, input_schema, is_enabled, sort_order
            FROM proxy_meta_tools
            WHERE is_enabled = TRUE
            ORDER BY sort_order ASC
            "#,
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }
}

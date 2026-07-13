use crate::models::UpstreamOAuthProvider;
use mcp_common::Result;
use sqlx::PgPool;

pub struct UpstreamOAuthProviderRepository;

impl UpstreamOAuthProviderRepository {
    pub async fn list_enabled(pool: &PgPool) -> Result<Vec<UpstreamOAuthProvider>> {
        let rows = sqlx::query_as::<_, UpstreamOAuthProvider>(
            r#"
            SELECT id, name, display_name, authorization_url, token_url, default_scopes,
                   is_managed, is_enabled, sort_order, created_at, updated_at
            FROM upstream_oauth_providers
            WHERE is_enabled = TRUE
            ORDER BY sort_order ASC
            "#,
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }

    pub async fn find_by_name(pool: &PgPool, name: &str) -> Result<Option<UpstreamOAuthProvider>> {
        let row = sqlx::query_as::<_, UpstreamOAuthProvider>(
            r#"
            SELECT id, name, display_name, authorization_url, token_url, default_scopes,
                   is_managed, is_enabled, sort_order, created_at, updated_at
            FROM upstream_oauth_providers
            WHERE name = $1
            "#,
        )
        .bind(name)
        .fetch_optional(pool)
        .await?;
        Ok(row)
    }
}

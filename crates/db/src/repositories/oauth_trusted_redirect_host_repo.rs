use mcp_common::Result;
use sqlx::PgPool;

pub struct OAuthTrustedRedirectHostRepository;

impl OAuthTrustedRedirectHostRepository {
    /// Returns all enabled trusted redirect hostnames.
    pub async fn list_enabled_hostnames(pool: &PgPool) -> Result<Vec<String>> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT hostname FROM oauth_trusted_redirect_hosts WHERE is_enabled = TRUE",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }
}

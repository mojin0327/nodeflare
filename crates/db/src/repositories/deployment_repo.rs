use crate::models::{CreateDeployment, Deployment, UpdateDeployment};
use chrono::{DateTime, Utc};
use mcp_common::Result;
use sqlx::PgPool;
use uuid::Uuid;

pub struct DeploymentRepository;

impl DeploymentRepository {
    pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Deployment>> {
        let deployment = sqlx::query_as::<_, Deployment>(
            r#"
            SELECT id, server_id, version, commit_sha, status, build_logs,
                   error_message, started_at, finished_at, deployed_by
            FROM deployments
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(deployment)
    }

    pub async fn find_latest_by_server(pool: &PgPool, server_id: Uuid) -> Result<Option<Deployment>> {
        let deployment = sqlx::query_as::<_, Deployment>(
            r#"
            SELECT id, server_id, version, commit_sha, status, build_logs,
                   error_message, started_at, finished_at, deployed_by
            FROM deployments
            WHERE server_id = $1
            ORDER BY version DESC
            LIMIT 1
            "#,
        )
        .bind(server_id)
        .fetch_optional(pool)
        .await?;

        Ok(deployment)
    }

    pub async fn list_by_server(
        pool: &PgPool,
        server_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Deployment>> {
        let deployments = sqlx::query_as::<_, Deployment>(
            r#"
            SELECT id, server_id, version, commit_sha, status, build_logs,
                   error_message, started_at, finished_at, deployed_by
            FROM deployments
            WHERE server_id = $1
            ORDER BY version DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(server_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok(deployments)
    }

    pub async fn create(pool: &PgPool, data: CreateDeployment) -> Result<Deployment> {
        // Single query: compute next version and insert in one atomic operation
        // This eliminates the race condition and reduces round trips from 2 to 1
        let deployment = sqlx::query_as::<_, Deployment>(
            r#"
            INSERT INTO deployments (server_id, version, commit_sha, deployed_by)
            SELECT $1, COALESCE(MAX(version), 0) + 1, $2, $3
            FROM deployments
            WHERE server_id = $1
            RETURNING id, server_id, version, commit_sha, status, build_logs,
                      error_message, started_at, finished_at, deployed_by
            "#,
        )
        .bind(data.server_id)
        .bind(&data.commit_sha)
        .bind(data.deployed_by)
        .fetch_one(pool)
        .await?;

        Ok(deployment)
    }

    pub async fn update(pool: &PgPool, id: Uuid, data: UpdateDeployment) -> Result<Deployment> {
        let status_str = data.status.map(|s| s.to_string());

        let deployment = sqlx::query_as::<_, Deployment>(
            r#"
            UPDATE deployments
            SET
                status = COALESCE($2, status),
                build_logs = COALESCE($3, build_logs),
                error_message = COALESCE($4, error_message),
                finished_at = COALESCE($5, finished_at)
            WHERE id = $1
            RETURNING id, server_id, version, commit_sha, status, build_logs,
                      error_message, started_at, finished_at, deployed_by
            "#,
        )
        .bind(id)
        .bind(status_str)
        .bind(&data.build_logs)
        .bind(&data.error_message)
        .bind(data.finished_at)
        .fetch_one(pool)
        .await?;

        Ok(deployment)
    }

    /// Maximum size for build_logs in bytes (default: 5MB)
    const MAX_BUILD_LOG_SIZE: i32 = 5 * 1024 * 1024;

    /// Append a log line to deployment build_logs with size limit protection
    /// If the log exceeds MAX_BUILD_LOG_SIZE, older logs are truncated
    pub async fn append_log(pool: &PgPool, id: Uuid, log_line: &str) -> Result<()> {
        // Use a single atomic UPDATE that checks and truncates if needed
        // This prevents race conditions and ensures logs don't exceed limit
        sqlx::query(
            r#"
            UPDATE deployments
            SET build_logs = CASE
                WHEN LENGTH(COALESCE(build_logs, '')) + LENGTH($2) > $3 THEN
                    -- Truncate from the beginning, keeping last portion + new line
                    '... [earlier logs truncated]\n' ||
                    RIGHT(COALESCE(build_logs, ''), $3 - LENGTH($2) - 50) || $2 || E'\n'
                ELSE
                    COALESCE(build_logs, '') || $2 || E'\n'
            END
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(log_line)
        .bind(Self::MAX_BUILD_LOG_SIZE)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Count deployments for a workspace since a given date (for monthly limits)
    pub async fn count_by_workspace_since(
        pool: &PgPool,
        workspace_id: Uuid,
        since: DateTime<Utc>,
    ) -> Result<i64> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM deployments d
            INNER JOIN mcp_servers s ON d.server_id = s.id
            WHERE s.workspace_id = $1 AND d.started_at >= $2
            "#,
        )
        .bind(workspace_id)
        .bind(since)
        .fetch_one(pool)
        .await?;

        Ok(count.0)
    }

    /// Check if a user has access to a deployment (optimized single query with JOIN)
    /// Returns true if the deployment exists and the user is a member of the associated workspace
    pub async fn check_user_access(pool: &PgPool, deployment_id: Uuid, user_id: Uuid) -> Result<bool> {
        let result: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT 1
            FROM deployments d
            INNER JOIN mcp_servers s ON d.server_id = s.id
            INNER JOIN workspace_members wm ON s.workspace_id = wm.workspace_id
            WHERE d.id = $1 AND wm.user_id = $2
            LIMIT 1
            "#,
        )
        .bind(deployment_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;

        Ok(result.is_some())
    }

    /// Mark stuck deployments as failed (timeout)
    /// Returns the number of deployments marked as failed
    pub async fn timeout_stuck_deployments(
        pool: &PgPool,
        timeout_minutes: i64,
    ) -> Result<u64> {
        let result = sqlx::query(
            r#"
            UPDATE deployments
            SET status = 'failed',
                error_message = 'Deployment timed out',
                finished_at = NOW()
            WHERE status IN ('pending', 'building', 'pushing', 'deploying')
              AND started_at < NOW() - ($1 || ' minutes')::INTERVAL
            "#,
        )
        .bind(timeout_minutes)
        .execute(pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Check if there's a succeeded deployment with version greater than the given version
    /// Used to prevent race conditions where an old failed deployment overwrites a newer success
    pub async fn has_succeeded_deployment_after(
        pool: &PgPool,
        server_id: Uuid,
        version: i32,
    ) -> Result<bool> {
        let result: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT 1
            FROM deployments
            WHERE server_id = $1
              AND version > $2
              AND status = 'succeeded'
            LIMIT 1
            "#,
        )
        .bind(server_id)
        .bind(version)
        .fetch_optional(pool)
        .await?;

        Ok(result.is_some())
    }
}

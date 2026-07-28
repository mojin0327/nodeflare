use anyhow::Result;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{
    CreateServerRegion, RegionStatus, RegionUsage, ServerRegion,
    UpdateServerRegion,
};

/// Helper function to get the first day of a month (safe, no panic)
fn first_of_month(year: i32, month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, 1)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or(NaiveDate::MIN))
}

/// Helper function to get the first day of the next month (safe, no panic)
fn first_of_next_month(year: i32, month: u32) -> NaiveDate {
    if month == 12 {
        first_of_month(year + 1, 1)
    } else {
        first_of_month(year, month + 1)
    }
}

pub struct ServerRegionRepository;

impl ServerRegionRepository {
    /// List all regions for a server
    pub async fn list_by_server(pool: &PgPool, server_id: Uuid) -> Result<Vec<ServerRegion>> {
        let regions = sqlx::query_as::<_, ServerRegion>(
            r#"
            SELECT id, server_id, region, is_primary, machine_id, status,
                   endpoint_url, cancel_at, created_at, updated_at
            FROM server_regions
            WHERE server_id = $1
            ORDER BY is_primary DESC, region ASC
            "#,
        )
        .bind(server_id)
        .fetch_all(pool)
        .await?;

        Ok(regions)
    }

    /// Get a specific region for a server
    pub async fn find_by_server_and_region(
        pool: &PgPool,
        server_id: Uuid,
        region: &str,
    ) -> Result<Option<ServerRegion>> {
        let region = sqlx::query_as::<_, ServerRegion>(
            r#"
            SELECT id, server_id, region, is_primary, machine_id, status,
                   endpoint_url, cancel_at, created_at, updated_at
            FROM server_regions
            WHERE server_id = $1 AND region = $2
            "#,
        )
        .bind(server_id)
        .bind(region)
        .fetch_optional(pool)
        .await?;

        Ok(region)
    }

    /// Get primary region for a server
    pub async fn find_primary(pool: &PgPool, server_id: Uuid) -> Result<Option<ServerRegion>> {
        let region = sqlx::query_as::<_, ServerRegion>(
            r#"
            SELECT id, server_id, region, is_primary, machine_id, status,
                   endpoint_url, cancel_at, created_at, updated_at
            FROM server_regions
            WHERE server_id = $1 AND is_primary = true
            "#,
        )
        .bind(server_id)
        .fetch_optional(pool)
        .await?;

        Ok(region)
    }

    /// Add a new region to a server
    pub async fn create(pool: &PgPool, data: CreateServerRegion) -> Result<ServerRegion> {
        let region = sqlx::query_as::<_, ServerRegion>(
            r#"
            INSERT INTO server_regions (server_id, region, is_primary, status)
            VALUES ($1, $2, $3, 'pending')
            RETURNING id, server_id, region, is_primary, machine_id, status,
                      endpoint_url, cancel_at, created_at, updated_at
            "#,
        )
        .bind(data.server_id)
        .bind(&data.region)
        .bind(data.is_primary)
        .fetch_one(pool)
        .await?;

        Ok(region)
    }

    /// Update a server region
    pub async fn update(
        pool: &PgPool,
        server_id: Uuid,
        region: &str,
        data: UpdateServerRegion,
    ) -> Result<Option<ServerRegion>> {
        let status_str = data.status.map(|s| s.as_str().to_string());

        let region = sqlx::query_as::<_, ServerRegion>(
            r#"
            UPDATE server_regions
            SET machine_id = COALESCE($3, machine_id),
                status = COALESCE($4, status),
                endpoint_url = COALESCE($5, endpoint_url),
                updated_at = NOW()
            WHERE server_id = $1 AND region = $2
            RETURNING id, server_id, region, is_primary, machine_id, status,
                      endpoint_url, cancel_at, created_at, updated_at
            "#,
        )
        .bind(server_id)
        .bind(region)
        .bind(&data.machine_id)
        .bind(&status_str)
        .bind(&data.endpoint_url)
        .fetch_optional(pool)
        .await?;

        Ok(region)
    }

    /// Update region status
    pub async fn update_status(
        pool: &PgPool,
        server_id: Uuid,
        region: &str,
        status: RegionStatus,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE server_regions
            SET status = $3, updated_at = NOW()
            WHERE server_id = $1 AND region = $2
            "#,
        )
        .bind(server_id)
        .bind(region)
        .bind(status.as_str())
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Delete a region from a server
    pub async fn delete(pool: &PgPool, server_id: Uuid, region: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM server_regions
            WHERE server_id = $1 AND region = $2 AND is_primary = false
            "#,
        )
        .bind(server_id)
        .bind(region)
        .execute(pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Count additional (non-primary) regions for a server
    pub async fn count_additional_regions(pool: &PgPool, server_id: Uuid) -> Result<i64> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM server_regions
            WHERE server_id = $1 AND is_primary = false
            "#,
        )
        .bind(server_id)
        .fetch_one(pool)
        .await?;

        Ok(count.0)
    }

    /// Count all additional regions for a workspace (for billing)
    pub async fn count_workspace_additional_regions(
        pool: &PgPool,
        workspace_id: Uuid,
    ) -> Result<i64> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM server_regions sr
            JOIN mcp_servers s ON sr.server_id = s.id
            WHERE s.workspace_id = $1 AND sr.is_primary = false AND sr.status = 'running'
            "#,
        )
        .bind(workspace_id)
        .fetch_one(pool)
        .await?;

        Ok(count.0)
    }

    /// List all running regions that need billing
    pub async fn list_running_additional_regions(
        pool: &PgPool,
        workspace_id: Uuid,
    ) -> Result<Vec<ServerRegion>> {
        let regions = sqlx::query_as::<_, ServerRegion>(
            r#"
            SELECT sr.id, sr.server_id, sr.region, sr.is_primary, sr.machine_id,
                   sr.status, sr.endpoint_url, sr.cancel_at, sr.created_at, sr.updated_at
            FROM server_regions sr
            JOIN mcp_servers s ON sr.server_id = s.id
            WHERE s.workspace_id = $1 AND sr.is_primary = false AND sr.status = 'running'
            "#,
        )
        .bind(workspace_id)
        .fetch_all(pool)
        .await?;

        Ok(regions)
    }

    /// Mark a region for cancellation at period end
    pub async fn mark_for_cancellation(
        pool: &PgPool,
        server_id: Uuid,
        region: &str,
        cancel_at: DateTime<Utc>,
    ) -> Result<Option<ServerRegion>> {
        let region = sqlx::query_as::<_, ServerRegion>(
            r#"
            UPDATE server_regions
            SET cancel_at = $3, updated_at = NOW()
            WHERE server_id = $1 AND region = $2
            RETURNING id, server_id, region, is_primary, machine_id, status,
                      endpoint_url, cancel_at, created_at, updated_at
            "#,
        )
        .bind(server_id)
        .bind(region)
        .bind(cancel_at)
        .fetch_optional(pool)
        .await?;

        Ok(region)
    }

    /// Clear cancellation mark (if user re-subscribes)
    pub async fn clear_cancellation(
        pool: &PgPool,
        server_id: Uuid,
        region: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE server_regions
            SET cancel_at = NULL, updated_at = NOW()
            WHERE server_id = $1 AND region = $2
            "#,
        )
        .bind(server_id)
        .bind(region)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// List regions that are past their cancellation date and should be deleted
    pub async fn list_regions_to_delete(pool: &PgPool) -> Result<Vec<ServerRegion>> {
        let regions = sqlx::query_as::<_, ServerRegion>(
            r#"
            SELECT id, server_id, region, is_primary, machine_id, status,
                   endpoint_url, cancel_at, created_at, updated_at
            FROM server_regions
            WHERE cancel_at IS NOT NULL AND cancel_at <= NOW()
            "#,
        )
        .fetch_all(pool)
        .await?;

        Ok(regions)
    }
}

pub struct RegionUsageRepository;

impl RegionUsageRepository {
    /// Get or create usage record for the current period
    pub async fn get_or_create_current(
        pool: &PgPool,
        workspace_id: Uuid,
        server_id: Uuid,
        region: &str,
    ) -> Result<RegionUsage> {
        let now = Utc::now().date_naive();
        let period_start = first_of_month(now.year(), now.month());
        let period_end = first_of_next_month(now.year(), now.month());

        // Try to get existing
        let existing = sqlx::query_as::<_, RegionUsage>(
            r#"
            SELECT id, workspace_id, server_id, region, period_start, period_end,
                   active_hours, gb_minutes, reported_to_stripe, stripe_usage_record_id,
                   created_at, updated_at
            FROM region_usage
            WHERE server_id = $1 AND region = $2 AND period_start = $3
            "#,
        )
        .bind(server_id)
        .bind(region)
        .bind(period_start)
        .fetch_optional(pool)
        .await?;

        if let Some(usage) = existing {
            return Ok(usage);
        }

        // Create new
        let usage = sqlx::query_as::<_, RegionUsage>(
            r#"
            INSERT INTO region_usage (workspace_id, server_id, region, period_start, period_end)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, workspace_id, server_id, region, period_start, period_end,
                      active_hours, gb_minutes, reported_to_stripe, stripe_usage_record_id,
                      created_at, updated_at
            "#,
        )
        .bind(workspace_id)
        .bind(server_id)
        .bind(region)
        .bind(period_start)
        .bind(period_end)
        .fetch_one(pool)
        .await?;

        Ok(usage)
    }

    /// Increment active hours for a region
    pub async fn increment_hours(
        pool: &PgPool,
        server_id: Uuid,
        region: &str,
        hours: i32,
    ) -> Result<()> {
        let now = Utc::now().date_naive();
        let period_start = first_of_month(now.year(), now.month());

        sqlx::query(
            r#"
            UPDATE region_usage
            SET active_hours = active_hours + $4, updated_at = NOW()
            WHERE server_id = $1 AND region = $2 AND period_start = $3
            "#,
        )
        .bind(server_id)
        .bind(region)
        .bind(period_start)
        .bind(hours)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Add memory-weighted active minutes (memory_gb * minutes) to the current
    /// period's row, creating the row first so the accumulation can't silently no-op.
    pub async fn add_gb_minutes(
        pool: &PgPool,
        workspace_id: Uuid,
        server_id: Uuid,
        region: &str,
        gb_minutes: f64,
    ) -> Result<()> {
        Self::get_or_create_current(pool, workspace_id, server_id, region).await?;

        let now = Utc::now().date_naive();
        let period_start = first_of_month(now.year(), now.month());

        sqlx::query(
            r#"
            UPDATE region_usage
            SET gb_minutes = gb_minutes + $4, updated_at = NOW()
            WHERE server_id = $1 AND region = $2 AND period_start = $3
            "#,
        )
        .bind(server_id)
        .bind(region)
        .bind(period_start)
        .bind(gb_minutes)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Current-period usage rows for a workspace (for the usage dashboard).
    pub async fn list_current_period(pool: &PgPool, workspace_id: Uuid) -> Result<Vec<RegionUsage>> {
        let now = Utc::now().date_naive();
        let period_start = first_of_month(now.year(), now.month());
        let records = sqlx::query_as::<_, RegionUsage>(
            r#"
            SELECT id, workspace_id, server_id, region, period_start, period_end,
                   active_hours, gb_minutes, reported_to_stripe, stripe_usage_record_id,
                   created_at, updated_at
            FROM region_usage
            WHERE workspace_id = $1 AND period_start = $2
            "#,
        )
        .bind(workspace_id)
        .bind(period_start)
        .fetch_all(pool)
        .await?;

        Ok(records)
    }

    /// Get unreported usage records for billing
    pub async fn list_unreported(pool: &PgPool, workspace_id: Uuid) -> Result<Vec<RegionUsage>> {
        let records = sqlx::query_as::<_, RegionUsage>(
            r#"
            SELECT id, workspace_id, server_id, region, period_start, period_end,
                   active_hours, gb_minutes, reported_to_stripe, stripe_usage_record_id,
                   created_at, updated_at
            FROM region_usage
            WHERE workspace_id = $1 AND reported_to_stripe = false AND gb_minutes > 0
            "#,
        )
        .bind(workspace_id)
        .fetch_all(pool)
        .await?;

        Ok(records)
    }

    /// Workspace ids that have unreported usage to bill (for the reporting job to iterate).
    pub async fn list_unreported_workspaces(pool: &PgPool) -> Result<Vec<Uuid>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            r#"
            SELECT DISTINCT workspace_id
            FROM region_usage
            WHERE reported_to_stripe = false AND gb_minutes > 0
            "#,
        )
        .fetch_all(pool)
        .await?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Mark usage as reported to Stripe
    pub async fn mark_reported(
        pool: &PgPool,
        usage_id: Uuid,
        stripe_usage_record_id: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE region_usage
            SET reported_to_stripe = true, stripe_usage_record_id = $2, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(usage_id)
        .bind(stripe_usage_record_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Mark a batch of usage records as reported in a single UPDATE.
    pub async fn mark_reported_batch(
        pool: &PgPool,
        usage_ids: &[Uuid],
        stripe_usage_record_id: &str,
    ) -> Result<()> {
        if usage_ids.is_empty() {
            return Ok(());
        }
        sqlx::query(
            r#"
            UPDATE region_usage
            SET reported_to_stripe = true, stripe_usage_record_id = $2, updated_at = NOW()
            WHERE id = ANY($1)
            "#,
        )
        .bind(usage_ids)
        .bind(stripe_usage_record_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Get total additional region count for current period
    pub async fn get_current_period_region_count(
        pool: &PgPool,
        workspace_id: Uuid,
    ) -> Result<i64> {
        let now = Utc::now().date_naive();
        let period_start = first_of_month(now.year(), now.month());

        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(DISTINCT (server_id, region))
            FROM region_usage
            WHERE workspace_id = $1 AND period_start = $2
            "#,
        )
        .bind(workspace_id)
        .bind(period_start)
        .fetch_one(pool)
        .await?;

        Ok(count.0)
    }
}

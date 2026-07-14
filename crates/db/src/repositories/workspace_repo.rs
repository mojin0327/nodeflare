use crate::models::{CreateWorkspace, MemberWithUser, UpdateWorkspace, Workspace, WorkspaceMember, WorkspaceWithRole};
use mcp_common::types::{Plan, WorkspaceRole};
use mcp_common::Result;
use sqlx::PgPool;
use uuid::Uuid;

const WORKSPACE_COLS: &str = "id, name, slug, plan, owner_id, stripe_customer_id, \
    stripe_subscription_id, stripe_region_subscription_item_id, subscription_status, \
    current_period_end, auto_email_invoices, created_at, updated_at";

pub struct WorkspaceRepository;

impl WorkspaceRepository {
    /// Maximum workspaces a user can belong to
    const MAX_WORKSPACES_PER_USER: i64 = 50;
    /// Maximum members per workspace
    const MAX_MEMBERS_PER_WORKSPACE: i64 = 200;

    pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Workspace>> {
        let workspace = sqlx::query_as::<_, Workspace>(
            &format!("SELECT {WORKSPACE_COLS} FROM workspaces WHERE id = $1"),
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(workspace)
    }

    pub async fn find_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Workspace>> {
        let workspace = sqlx::query_as::<_, Workspace>(
            &format!("SELECT {WORKSPACE_COLS} FROM workspaces WHERE slug = $1"),
        )
        .bind(slug)
        .fetch_optional(pool)
        .await?;

        Ok(workspace)
    }

    pub async fn list_by_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<WorkspaceWithRole>> {
        let workspaces = sqlx::query_as::<_, WorkspaceWithRole>(
            r#"
            SELECT w.id, w.name, w.slug, w.plan, w.owner_id, w.stripe_customer_id,
                   w.stripe_subscription_id, w.stripe_region_subscription_item_id,
                   w.subscription_status, w.current_period_end, w.auto_email_invoices,
                   w.created_at, w.updated_at, wm.role
            FROM workspaces w
            INNER JOIN workspace_members wm ON w.id = wm.workspace_id
            WHERE wm.user_id = $1
            ORDER BY w.created_at DESC
            LIMIT $2
            "#,
        )
        .bind(user_id)
        .bind(Self::MAX_WORKSPACES_PER_USER)
        .fetch_all(pool)
        .await?;

        Ok(workspaces)
    }

    pub async fn create(pool: &PgPool, data: CreateWorkspace) -> Result<Workspace> {
        let mut tx = pool.begin().await?;

        let workspace = sqlx::query_as::<_, Workspace>(
            &format!(
                "INSERT INTO workspaces (name, slug, owner_id) VALUES ($1, $2, $3) \
                 RETURNING {WORKSPACE_COLS}"
            ),
        )
        .bind(&data.name)
        .bind(&data.slug)
        .bind(data.owner_id)
        .fetch_one(&mut *tx)
        .await?;

        // Add owner as workspace member
        sqlx::query(
            r#"
            INSERT INTO workspace_members (workspace_id, user_id, role)
            VALUES ($1, $2, 'owner')
            "#,
        )
        .bind(workspace.id)
        .bind(data.owner_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(workspace)
    }

    pub async fn update(pool: &PgPool, id: Uuid, data: UpdateWorkspace) -> Result<Workspace> {
        let plan_str = data.plan.as_ref().map(|p| p.as_str());

        let workspace = sqlx::query_as::<_, Workspace>(
            &format!(
                "UPDATE workspaces SET name = COALESCE($2, name), plan = COALESCE($3, plan), \
                 updated_at = NOW() WHERE id = $1 RETURNING {WORKSPACE_COLS}"
            ),
        )
        .bind(id)
        .bind(&data.name)
        .bind(plan_str)
        .fetch_one(pool)
        .await?;

        Ok(workspace)
    }

    /// Update Stripe customer ID only
    pub async fn update_stripe_customer(
        pool: &PgPool,
        id: Uuid,
        customer_id: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE workspaces
            SET
                stripe_customer_id = $2,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(customer_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Update Stripe customer and subscription IDs
    pub async fn update_stripe_ids(
        pool: &PgPool,
        id: Uuid,
        customer_id: &str,
        subscription_id: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE workspaces
            SET
                stripe_customer_id = $2,
                stripe_subscription_id = $3,
                subscription_status = 'active',
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(customer_id)
        .bind(subscription_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Update workspace plan
    pub async fn update_plan(pool: &PgPool, id: Uuid, plan: Plan) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE workspaces
            SET
                plan = $2,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(plan.as_str())
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Clear Stripe subscription (on cancellation)
    pub async fn clear_stripe_subscription(pool: &PgPool, id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE workspaces
            SET
                stripe_subscription_id = NULL,
                subscription_status = 'cancelled',
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Update subscription status
    pub async fn update_subscription_status(
        pool: &PgPool,
        id: Uuid,
        status: &str,
        period_end: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE workspaces
            SET
                subscription_status = $2,
                current_period_end = $3,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(status)
        .bind(period_end)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Find workspace by Stripe customer ID
    pub async fn find_by_stripe_customer(pool: &PgPool, customer_id: &str) -> Result<Option<Workspace>> {
        let workspace = sqlx::query_as::<_, Workspace>(
            &format!("SELECT {WORKSPACE_COLS} FROM workspaces WHERE stripe_customer_id = $1"),
        )
        .bind(customer_id)
        .fetch_optional(pool)
        .await?;

        Ok(workspace)
    }

    /// Find workspace by Stripe subscription ID
    pub async fn find_by_stripe_subscription(pool: &PgPool, subscription_id: &str) -> Result<Option<Workspace>> {
        let workspace = sqlx::query_as::<_, Workspace>(
            &format!("SELECT {WORKSPACE_COLS} FROM workspaces WHERE stripe_subscription_id = $1"),
        )
        .bind(subscription_id)
        .fetch_optional(pool)
        .await?;

        Ok(workspace)
    }

    pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM workspaces WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn get_member(
        pool: &PgPool,
        workspace_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<WorkspaceMember>> {
        let member = sqlx::query_as::<_, WorkspaceMember>(
            r#"
            SELECT workspace_id, user_id, role, created_at
            FROM workspace_members
            WHERE workspace_id = $1 AND user_id = $2
            "#,
        )
        .bind(workspace_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;

        Ok(member)
    }

    pub async fn add_member(
        pool: &PgPool,
        workspace_id: Uuid,
        user_id: Uuid,
        role: WorkspaceRole,
    ) -> Result<WorkspaceMember> {
        let member = sqlx::query_as::<_, WorkspaceMember>(
            r#"
            INSERT INTO workspace_members (workspace_id, user_id, role)
            VALUES ($1, $2, $3)
            RETURNING workspace_id, user_id, role, created_at
            "#,
        )
        .bind(workspace_id)
        .bind(user_id)
        .bind(role.as_str())
        .fetch_one(pool)
        .await?;

        Ok(member)
    }

    pub async fn remove_member(pool: &PgPool, workspace_id: Uuid, user_id: Uuid) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM workspace_members
            WHERE workspace_id = $1 AND user_id = $2
            "#,
        )
        .bind(workspace_id)
        .bind(user_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Update member role atomically (prevents race conditions)
    pub async fn update_member_role(
        pool: &PgPool,
        workspace_id: Uuid,
        user_id: Uuid,
        role: WorkspaceRole,
    ) -> Result<Option<WorkspaceMember>> {
        let member = sqlx::query_as::<_, WorkspaceMember>(
            r#"
            UPDATE workspace_members
            SET role = $3
            WHERE workspace_id = $1 AND user_id = $2
            RETURNING workspace_id, user_id, role, created_at
            "#,
        )
        .bind(workspace_id)
        .bind(user_id)
        .bind(role.as_str())
        .fetch_optional(pool)
        .await?;

        Ok(member)
    }

    /// Count members in a workspace (more efficient than fetching all members)
    pub async fn count_members(pool: &PgPool, workspace_id: Uuid) -> Result<i64> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM workspace_members
            WHERE workspace_id = $1
            "#,
        )
        .bind(workspace_id)
        .fetch_one(pool)
        .await?;

        Ok(count.0)
    }

    pub async fn list_members(pool: &PgPool, workspace_id: Uuid) -> Result<Vec<WorkspaceMember>> {
        let members = sqlx::query_as::<_, WorkspaceMember>(
            r#"
            SELECT workspace_id, user_id, role, created_at
            FROM workspace_members
            WHERE workspace_id = $1
            ORDER BY created_at
            LIMIT $2
            "#,
        )
        .bind(workspace_id)
        .bind(Self::MAX_MEMBERS_PER_WORKSPACE)
        .fetch_all(pool)
        .await?;

        Ok(members)
    }

    /// List members with user details in a single query (prevents N+1)
    pub async fn list_members_with_users(pool: &PgPool, workspace_id: Uuid) -> Result<Vec<MemberWithUser>> {
        let members = sqlx::query_as::<_, MemberWithUser>(
            r#"
            SELECT
                u.id as user_id,
                u.email,
                u.name,
                u.avatar_url,
                wm.role,
                wm.created_at as member_created_at
            FROM workspace_members wm
            INNER JOIN users u ON wm.user_id = u.id
            WHERE wm.workspace_id = $1
            ORDER BY wm.created_at
            LIMIT $2
            "#,
        )
        .bind(workspace_id)
        .bind(Self::MAX_MEMBERS_PER_WORKSPACE)
        .fetch_all(pool)
        .await?;

        Ok(members)
    }

    /// Update region subscription item ID
    pub async fn update_region_subscription_item(
        pool: &PgPool,
        id: Uuid,
        subscription_item_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE workspaces
            SET
                stripe_region_subscription_item_id = $2,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(subscription_item_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn list_owned_by_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<Workspace>> {
        let workspaces = sqlx::query_as::<_, Workspace>(
            &format!(
                "SELECT {WORKSPACE_COLS} FROM workspaces WHERE owner_id = $1 \
                 ORDER BY created_at DESC LIMIT $2"
            ),
        )
        .bind(user_id)
        .bind(Self::MAX_WORKSPACES_PER_USER)
        .fetch_all(pool)
        .await?;

        Ok(workspaces)
    }

    /// Update billing settings
    pub async fn update_billing_settings(
        pool: &PgPool,
        id: Uuid,
        auto_email_invoices: bool,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE workspaces
            SET
                auto_email_invoices = $2,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(auto_email_invoices)
        .execute(pool)
        .await?;

        Ok(())
    }
}

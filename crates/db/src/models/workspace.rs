use chrono::{DateTime, Utc};
use mcp_common::types::{Plan, WorkspaceRole};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub plan: String,
    pub owner_id: Uuid,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub stripe_region_subscription_item_id: Option<String>,
    pub subscription_status: Option<String>,
    pub current_period_end: Option<DateTime<Utc>>,
    pub auto_email_invoices: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Workspace {
    pub fn plan(&self) -> Plan {
        Plan::from_str(&self.plan)
    }
}

#[derive(Debug, Clone)]
pub struct CreateWorkspace {
    pub name: String,
    pub slug: String,
    pub owner_id: Uuid,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateWorkspace {
    pub name: Option<String>,
    pub plan: Option<Plan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WorkspaceMember {
    pub workspace_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

impl WorkspaceMember {
    pub fn role(&self) -> WorkspaceRole {
        WorkspaceRole::from_str(&self.role)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WorkspaceWithRole {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub plan: String,
    pub owner_id: Uuid,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub stripe_region_subscription_item_id: Option<String>,
    pub subscription_status: Option<String>,
    pub current_period_end: Option<DateTime<Utc>>,
    pub auto_email_invoices: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub role: String,
}

impl WorkspaceWithRole {
    pub fn plan(&self) -> Plan {
        Plan::from_str(&self.plan)
    }

    pub fn role(&self) -> WorkspaceRole {
        WorkspaceRole::from_str(&self.role)
    }
}

/// Member with user details (for N+1 problem prevention)
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MemberWithUser {
    // User fields
    pub user_id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    // Member fields
    pub role: String,
    pub member_created_at: DateTime<Utc>,
}

impl MemberWithUser {
    pub fn role(&self) -> WorkspaceRole {
        WorkspaceRole::from_str(&self.role)
    }
}

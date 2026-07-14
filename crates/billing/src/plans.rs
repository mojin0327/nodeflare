use serde::{Deserialize, Serialize};

/// Subscription plan types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    Free,
    Pro,
    Team,
    Enterprise,
}

impl Default for Plan {
    fn default() -> Self {
        Self::Free
    }
}

impl std::fmt::Display for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Plan::Free => write!(f, "free"),
            Plan::Pro => write!(f, "pro"),
            Plan::Team => write!(f, "team"),
            Plan::Enterprise => write!(f, "enterprise"),
        }
    }
}

impl Plan {
    /// Parse a plan string stored in the database into a `Plan`. Unrecognised strings
    /// default to `Free` so that a new tier can be added without breaking existing rows.
    pub fn from_str(s: &str) -> Self {
        match s {
            "pro" => Plan::Pro,
            "team" => Plan::Team,
            "enterprise" => Plan::Enterprise,
            _ => Plan::Free,
        }
    }

    pub fn from_stripe_price_id(price_id: &str) -> Option<Self> {
        // These should match your Stripe price IDs
        // In production, load these from environment or config
        match price_id {
            id if id.starts_with("price_pro_") => Some(Plan::Pro),
            id if id.starts_with("price_team_") => Some(Plan::Team),
            id if id.starts_with("price_enterprise_") => Some(Plan::Enterprise),
            _ => None,
        }
    }

    pub fn limits(&self) -> PlanLimits {
        PLANS.iter()
            .find(|p| p.plan == *self)
            .map(|p| p.limits.clone())
            .unwrap_or_default()
    }
}

/// Allowed per-server memory sizes (MB), smallest first. The Free plan is capped at
/// the smallest entry; paid plans raise the ceiling via [`PlanLimits::max_memory_mb`].
pub const MEMORY_LADDER_MB: &[u32] = &[256, 512, 1024, 2048];

/// Plan limits and quotas
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanLimits {
    /// Maximum number of servers
    pub max_servers: u32,
    /// Maximum number of deployments per month
    pub max_deployments_per_month: u32,
    /// Maximum number of API requests per month
    pub max_requests_per_month: u64,
    /// Maximum number of team members (for Team/Enterprise)
    pub max_team_members: u32,
    /// Maximum machine memory a server may be assigned, in MB. Acts as the ceiling
    /// for the user-selectable memory ladder ([`MEMORY_LADDER_MB`]).
    pub max_memory_mb: u32,
    /// Log retention in days
    pub log_retention_days: u32,
    /// Custom domain support
    pub custom_domains: bool,
    /// Priority support
    pub priority_support: bool,
    /// SSO support
    pub sso_enabled: bool,
}

impl Default for PlanLimits {
    fn default() -> Self {
        // Free tier limits
        Self {
            max_servers: 3,
            max_deployments_per_month: 50,
            max_requests_per_month: 10_000,
            max_team_members: 1,
            max_memory_mb: 256,
            log_retention_days: 7,
            custom_domains: false,
            priority_support: false,
            sso_enabled: false,
        }
    }
}

/// Plan definition with pricing info
#[derive(Debug, Clone, Serialize)]
pub struct PlanDefinition {
    pub plan: Plan,
    pub name: &'static str,
    pub description: &'static str,
    /// Price in JPY per month. None means "contact us" pricing.
    pub price_monthly_jpy: Option<u32>,
    /// Price in JPY per year. None means "contact us" pricing.
    pub price_yearly_jpy: Option<u32>,
    #[serde(skip)]
    pub stripe_price_id_monthly: Option<&'static str>,
    #[serde(skip)]
    pub stripe_price_id_yearly: Option<&'static str>,
    pub limits: PlanLimits,
    #[serde(serialize_with = "serialize_features")]
    pub features: &'static [&'static str],
}

fn serialize_features<S>(features: &&'static [&'static str], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(features.len()))?;
    for feature in *features {
        seq.serialize_element(feature)?;
    }
    seq.end()
}

/// All available plans
pub static PLANS: &[PlanDefinition] = &[
    PlanDefinition {
        plan: Plan::Free,
        name: "Free",
        description: "Perfect for getting started",
        price_monthly_jpy: Some(0),
        price_yearly_jpy: Some(0),
        stripe_price_id_monthly: None,
        stripe_price_id_yearly: None,
        limits: PlanLimits {
            max_servers: 3,
            max_deployments_per_month: 50,
            max_requests_per_month: 10_000,
            max_team_members: 1,
            max_memory_mb: 256,
            log_retention_days: 7,
            custom_domains: false,
            priority_support: false,
            sso_enabled: false,
        },
        features: &[
            "3 MCP servers",
            "50 deployments/month",
            "10K API requests/month",
            "7-day log retention",
            "Community support",
        ],
    },
    PlanDefinition {
        plan: Plan::Pro,
        name: "Pro",
        description: "For professional developers",
        price_monthly_jpy: Some(2980),
        price_yearly_jpy: Some(29800),
        stripe_price_id_monthly: None, // Set via STRIPE_PRICE_PRO_MONTHLY env
        stripe_price_id_yearly: None,  // Set via STRIPE_PRICE_PRO_YEARLY env
        limits: PlanLimits {
            max_servers: 20,
            max_deployments_per_month: 500,
            max_requests_per_month: 500_000,
            max_team_members: 1,
            max_memory_mb: 2048,
            log_retention_days: 30,
            custom_domains: true,
            priority_support: false,
            sso_enabled: false,
        },
        features: &[
            "20 MCP servers",
            "500 deployments/month",
            "500K API requests/month",
            "30-day log retention",
            "Custom domains",
            "Email support",
        ],
    },
    PlanDefinition {
        plan: Plan::Team,
        name: "Team",
        description: "For growing teams",
        price_monthly_jpy: Some(9800),
        price_yearly_jpy: Some(98000),
        stripe_price_id_monthly: None, // Set via STRIPE_PRICE_TEAM_MONTHLY env
        stripe_price_id_yearly: None,  // Set via STRIPE_PRICE_TEAM_YEARLY env
        limits: PlanLimits {
            max_servers: 100,
            max_deployments_per_month: 2000,
            max_requests_per_month: 5_000_000,
            max_team_members: 10,
            max_memory_mb: 2048,
            log_retention_days: 90,
            custom_domains: true,
            priority_support: true,
            sso_enabled: false,
        },
        features: &[
            "100 MCP servers",
            "2000 deployments/month",
            "5M API requests/month",
            "90-day log retention",
            "Custom domains",
            "Up to 10 team members",
            "Priority support",
        ],
    },
    PlanDefinition {
        plan: Plan::Enterprise,
        name: "Enterprise",
        description: "For large organizations",
        price_monthly_jpy: None, // Contact us for pricing
        price_yearly_jpy: None,  // Contact us for pricing
        stripe_price_id_monthly: None, // Set via STRIPE_PRICE_ENTERPRISE_MONTHLY env
        stripe_price_id_yearly: None,  // Set via STRIPE_PRICE_ENTERPRISE_YEARLY env
        // Scalability: Using very high but finite limits to prevent DDoS/resource exhaustion
        // These are practical limits that no legitimate customer would exceed
        // Can be customized per-customer via database overrides
        limits: PlanLimits {
            max_servers: 10_000,              // 10K servers max
            max_deployments_per_month: 100_000, // 100K deployments/month
            max_requests_per_month: 1_000_000_000, // 1B requests/month
            max_team_members: 1_000,          // 1K team members
            max_memory_mb: 2048,              // ceiling = ladder max (raise once metered)
            log_retention_days: 365,
            custom_domains: true,
            priority_support: true,
            sso_enabled: true,
        },
        features: &[
            "Up to 10,000 MCP servers",
            "Up to 100K deployments/month",
            "Up to 1B API requests/month",
            "1-year log retention",
            "Custom domains",
            "Up to 1,000 team members",
            "SSO/SAML",
            "Dedicated support",
            "SLA",
        ],
    },
];

/// Why a user-requested per-server memory size was rejected.
#[derive(Debug, Clone, PartialEq)]
pub enum MemoryChoiceError {
    /// Not one of the allowed [`MEMORY_LADDER_MB`] sizes.
    NotOnLadder,
    /// Exceeds the plan's ceiling. Carries the ceiling for the error message.
    ExceedsPlan { ceiling_mb: u32 },
}

/// Validate a user-selected memory size (MB) against the allowed ladder and the
/// plan ceiling. `None` means "auto" and is always allowed.
pub fn validate_memory_choice(
    memory_mb: Option<u32>,
    limits: &PlanLimits,
) -> Result<(), MemoryChoiceError> {
    let Some(mb) = memory_mb else {
        return Ok(());
    };
    if !MEMORY_LADDER_MB.contains(&mb) {
        return Err(MemoryChoiceError::NotOnLadder);
    }
    if mb > limits.max_memory_mb {
        return Err(MemoryChoiceError::ExceedsPlan {
            ceiling_mb: limits.max_memory_mb,
        });
    }
    Ok(())
}

/// Get plan definition by plan type
pub fn get_plan_definition(plan: Plan) -> Option<&'static PlanDefinition> {
    PLANS.iter().find(|p| p.plan == plan)
}

/// Get plan by Stripe price ID
pub fn get_plan_by_price_id(price_id: &str) -> Option<Plan> {
    // Check environment variables for price IDs
    if let Ok(pro_monthly) = std::env::var("STRIPE_PRICE_PRO_MONTHLY") {
        if price_id == pro_monthly {
            return Some(Plan::Pro);
        }
    }
    if let Ok(pro_yearly) = std::env::var("STRIPE_PRICE_PRO_YEARLY") {
        if price_id == pro_yearly {
            return Some(Plan::Pro);
        }
    }
    if let Ok(team_monthly) = std::env::var("STRIPE_PRICE_TEAM_MONTHLY") {
        if price_id == team_monthly {
            return Some(Plan::Team);
        }
    }
    if let Ok(team_yearly) = std::env::var("STRIPE_PRICE_TEAM_YEARLY") {
        if price_id == team_yearly {
            return Some(Plan::Team);
        }
    }
    if let Ok(enterprise_monthly) = std::env::var("STRIPE_PRICE_ENTERPRISE_MONTHLY") {
        if price_id == enterprise_monthly {
            return Some(Plan::Enterprise);
        }
    }
    if let Ok(enterprise_yearly) = std::env::var("STRIPE_PRICE_ENTERPRISE_YEARLY") {
        if price_id == enterprise_yearly {
            return Some(Plan::Enterprise);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_plan_only_allows_smallest_memory() {
        let free = Plan::Free.limits();
        assert_eq!(free.max_memory_mb, MEMORY_LADDER_MB[0]);
        // None (auto) is always fine.
        assert!(validate_memory_choice(None, &free).is_ok());
        // The smallest size is allowed; anything larger is rejected with the ceiling.
        assert!(validate_memory_choice(Some(256), &free).is_ok());
        assert_eq!(
            validate_memory_choice(Some(512), &free),
            Err(MemoryChoiceError::ExceedsPlan { ceiling_mb: 256 })
        );
    }

    #[test]
    fn pro_plan_allows_up_to_2gb() {
        let pro = Plan::Pro.limits();
        assert_eq!(pro.max_memory_mb, 2048);
        for &mb in MEMORY_LADDER_MB {
            assert!(validate_memory_choice(Some(mb), &pro).is_ok(), "{mb} should fit Pro");
        }
    }

    #[test]
    fn off_ladder_sizes_are_rejected() {
        let pro = Plan::Pro.limits();
        // 768 is between rungs and 4096 is above the ladder entirely.
        assert_eq!(validate_memory_choice(Some(768), &pro), Err(MemoryChoiceError::NotOnLadder));
        assert_eq!(validate_memory_choice(Some(4096), &pro), Err(MemoryChoiceError::NotOnLadder));
    }
}

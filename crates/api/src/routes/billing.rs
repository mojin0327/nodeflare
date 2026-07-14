use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use fred::interfaces::{KeysInterface, LuaInterface};
use mcp_billing::{PaymentMethodDetails, Plan as BillingPlan, PLANS};
use mcp_db::{RegionUsageRepository, ServerRepository, WorkspaceRepository};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{db_error, internal_error};
use crate::extractors::{workspace, AuthUser};
use crate::middleware::rate_limit::extract_client_ip;
use crate::state::AppState;
use crate::routes::helpers::ERR_WORKSPACE_NOT_FOUND;

// Lock timeout for checkout creation (prevents race conditions)
const CHECKOUT_LOCK_TTL_SECS: i64 = 30;

// Webhook rate limiting constants
const WEBHOOK_RATE_LIMIT_PREFIX: &str = "webhook_rate:stripe:";
const WEBHOOK_RATE_LIMIT_MAX: i64 = 100; // Max 100 requests per minute
const WEBHOOK_RATE_LIMIT_WINDOW_SECS: i64 = 60;

/// Get available plans
pub async fn list_plans() -> Json<Vec<PlanResponse>> {
    let plans: Vec<PlanResponse> = PLANS
        .iter()
        .map(|p| PlanResponse {
            plan: p.plan.to_string(),
            name: p.name.to_string(),
            description: p.description.to_string(),
            price_monthly_jpy: p.price_monthly_jpy,
            price_yearly_jpy: p.price_yearly_jpy,
            features: p.features.iter().map(|s| s.to_string()).collect(),
            limits: PlanLimitsResponse {
                max_servers: p.limits.max_servers,
                max_deployments_per_month: p.limits.max_deployments_per_month,
                max_requests_per_month: p.limits.max_requests_per_month,
                max_team_members: p.limits.max_team_members,
                max_memory_mb: p.limits.max_memory_mb,
                log_retention_days: p.limits.log_retention_days,
                custom_domains: p.limits.custom_domains,
                priority_support: p.limits.priority_support,
                sso_enabled: p.limits.sso_enabled,
            },
        })
        .collect();

    Json(plans)
}

/// Get current subscription status for a workspace
/// Stripe replies 400 "No such customer" / resource_missing when a workspace's stored
/// stripe_customer_id doesn't exist in the current account — e.g. a plan set manually in
/// the DB, a stale id, or one from a different Stripe account. Treat that as "no billing
/// data" so the billing page still loads instead of 500-ing.
fn is_missing_stripe_resource<E: std::fmt::Display>(error: &E) -> bool {
    let s = error.to_string().to_lowercase();
    s.contains("no such customer") || s.contains("resource_missing")
}

pub async fn get_subscription(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<SubscriptionResponse>, (StatusCode, String)> {
    // Verify user has access
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    let mut cancel_at_period_end = false;
    let mut current_period_start: Option<i64> = None;

    if let (Some(billing), Some(subscription_id)) = (state.billing.as_ref(), workspace.stripe_subscription_id.as_ref()) {
        if let Ok(subscription) = billing.get_subscription(subscription_id).await {
            cancel_at_period_end = subscription.cancel_at_period_end;
            current_period_start = Some(subscription.current_period_start);
        }
    }

    Ok(Json(SubscriptionResponse {
        plan: workspace.plan.clone(),
        status: workspace.subscription_status.unwrap_or_else(|| "none".to_string()),
        stripe_customer_id: workspace.stripe_customer_id,
        stripe_subscription_id: workspace.stripe_subscription_id,
        current_period_start,
        current_period_end: workspace.current_period_end.map(|d| d.timestamp()),
        cancel_at_period_end,
    }))
}

/// Current-month memory-time usage for a workspace (GB-hours per server + total).
/// Usage billing applies to paid plans; Free is shown for transparency but billed flat.
pub async fn get_usage(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<UsageResponse>, (StatusCode, String)> {
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    let rows = RegionUsageRepository::list_current_period(&state.db, workspace_id)
        .await
        .map_err(db_error)?;

    // Sum gb_minutes per server (a server may span regions), then convert to GB-hours.
    let mut per_server: std::collections::HashMap<Uuid, f64> = std::collections::HashMap::new();
    for r in &rows {
        *per_server.entry(r.server_id).or_insert(0.0) += r.gb_minutes;
    }

    // Label each server. Missing names (deleted servers) still count toward the total.
    let names: std::collections::HashMap<Uuid, String> =
        ServerRepository::list_by_workspace(&state.db, workspace_id, 200, 0)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|s| (s.id, s.name))
            .collect();

    let servers: Vec<ServerUsage> = per_server
        .iter()
        .map(|(id, gb_minutes)| ServerUsage {
            server_id: *id,
            name: names.get(id).cloned().unwrap_or_else(|| "(deleted)".to_string()),
            gb_hours: gb_minutes / 60.0,
        })
        .collect();

    let total_gb_hours: f64 = per_server.values().sum::<f64>() / 60.0;

    // Display-only estimate. Real charges come later via Stripe metered usage; until a
    // rate is configured this is 0 and nothing is billed for usage.
    let rate = std::env::var("USAGE_RATE_JPY_PER_GB_HOUR")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    let billing_plan = BillingPlan::from_str(&workspace.plan);
    // Free plan is flat (capped, not metered): never show a usage charge.
    let metered = billing_plan != BillingPlan::Free;
    let estimated_cost_jpy = if metered { total_gb_hours * rate } else { 0.0 };

    Ok(Json(UsageResponse {
        plan: workspace.plan,
        metered,
        total_gb_hours,
        rate_jpy_per_gb_hour: rate,
        estimated_cost_jpy,
        servers,
    }))
}

/// Create a checkout session
pub async fn create_checkout(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<CreateCheckoutRequest>,
) -> Result<Json<CheckoutResponse>, (StatusCode, String)> {
    // Validate plan is one of the allowed values (server-side validation)
    if !matches!(body.plan.as_str(), "pro" | "team" | "enterprise") {
        return Err((StatusCode::BAD_REQUEST, "Invalid plan. Must be one of: pro, team, enterprise".to_string()));
    }

    // Verify user is owner/admin
    workspace::require_admin(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    // Prevent duplicate subscriptions - check if already has active subscription
    if workspace.stripe_subscription_id.is_some() {
        return Err((StatusCode::CONFLICT, "Workspace already has an active subscription. Please cancel the existing subscription first or use the billing portal to change plans.".to_string()));
    }

    // Acquire lock to prevent race condition (two simultaneous checkout requests)
    let lock_key = format!("checkout_lock:{}", workspace_id);
    let lock_acquired: bool = state.redis
        .set(
            &lock_key,
            "1",
            Some(fred::types::Expiration::EX(CHECKOUT_LOCK_TTL_SECS)),
            Some(fred::types::SetOptions::NX), // Only set if not exists
            false,
        )
        .await
        .unwrap_or(false);

    if !lock_acquired {
        return Err((StatusCode::TOO_MANY_REQUESTS, "A checkout is already in progress for this workspace. Please wait a moment and try again.".to_string()));
    }

    // Lock will auto-expire after CHECKOUT_LOCK_TTL_SECS
    // This prevents race conditions while allowing retries after timeout

    let billing = state.billing.as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Billing not configured".to_string()))?;

    // Get or create Stripe customer
    let customer_id = if let Some(id) = workspace.stripe_customer_id.clone() {
        id
    } else {
        // Get user info to create customer
        let user = mcp_db::UserRepository::find_by_id(&state.db, auth_user.user_id)
            .await
            .map_err(db_error)?
            .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

        let customer = billing
            .create_customer(&user.email, &user.name, auth_user.user_id)
            .await
            .map_err(db_error)?;

        let customer_id = customer.id.to_string();

        // Save customer ID to workspace
        WorkspaceRepository::update_stripe_customer(&state.db, workspace_id, &customer_id)
            .await
            .map_err(db_error)?;

        customer_id
    };

    // Get price ID based on plan and interval (server-side only - never trust client price_id)
    let price_id = get_price_id(&body.plan, body.yearly)
        .ok_or((StatusCode::BAD_REQUEST, "Price not configured for this plan".to_string()))?;

    // Create checkout session
    let session = billing
        .create_checkout_session(&customer_id, &price_id, workspace_id)
        .await
        .map_err(db_error)?;

    // Ensure checkout URL is present
    let checkout_url = session.url
        .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "Stripe did not return a checkout URL".to_string()))?;

    Ok(Json(CheckoutResponse {
        checkout_url,
        session_id: session.id.to_string(),
    }))
}

/// Change subscription plan (upgrade or downgrade)
/// For reactivation (same plan, clearing cancel), uses direct API
/// For actual plan changes, redirects to Stripe Customer Portal
pub async fn change_plan(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<CreateCheckoutRequest>,
) -> Result<Json<ChangePlanResponse>, (StatusCode, String)> {
    // Validate plan
    if !matches!(body.plan.as_str(), "pro" | "team" | "enterprise") {
        return Err((StatusCode::BAD_REQUEST, "Invalid plan".to_string()));
    }

    // Verify user is owner/admin
    workspace::require_admin(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    // Must have an existing subscription to change plan
    let subscription_id = workspace
        .stripe_subscription_id
        .clone()
        .ok_or((StatusCode::BAD_REQUEST, "No active subscription. Please subscribe first.".to_string()))?;

    let billing = state.billing.as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Billing not configured".to_string()))?;

    // Get current subscription to check if this is a reactivation or plan change
    let current_subscription = billing
        .get_subscription(&subscription_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get subscription: {}", e)))?;

    let current_plan = billing.get_plan_from_subscription(&current_subscription);
    let is_reactivation = body.plan == current_plan.to_string() && current_subscription.cancel_at_period_end;

    if is_reactivation {
        // Reactivation: just clear cancel_at_period_end using direct API
        let new_price_id = get_price_id(&body.plan, body.yearly)
            .ok_or((StatusCode::BAD_REQUEST, "Price not configured for this plan".to_string()))?;

        let subscription = billing
            .update_subscription_plan(&subscription_id, &new_price_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to reactivate: {}", e)))?;

        tracing::info!(
            "Reactivated workspace {} subscription {} (cleared cancel_at_period_end)",
            workspace_id,
            subscription_id
        );

        Ok(Json(ChangePlanResponse {
            plan: body.plan,
            status: subscription.status.to_string(),
            portal_url: None,
        }))
    } else {
        // Plan change: redirect to Stripe Customer Portal
        let customer_id = workspace
            .stripe_customer_id
            .ok_or((StatusCode::BAD_REQUEST, "No billing setup for this workspace".to_string()))?;

        let session = billing
            .create_portal_session(&customer_id)
            .await
            .map_err(db_error)?;

        tracing::info!(
            "Redirecting workspace {} to Stripe Portal for plan change",
            workspace_id
        );

        Ok(Json(ChangePlanResponse {
            plan: body.plan,
            status: "redirect".to_string(),
            portal_url: Some(session.url),
        }))
    }
}

#[derive(Debug, Serialize)]
pub struct ChangePlanResponse {
    pub plan: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portal_url: Option<String>,
}

/// Cancel subscription
pub async fn cancel_subscription(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<CancelResponse>, (StatusCode, String)> {
    // Verify user is owner/admin
    workspace::require_admin(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    let subscription_id = workspace
        .stripe_subscription_id
        .ok_or((StatusCode::BAD_REQUEST, "No active subscription to cancel".to_string()))?;

    let billing = state.billing.as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Billing not configured".to_string()))?;

    // First, check current subscription status
    let current_subscription = billing
        .get_subscription(&subscription_id)
        .await
        .map_err(|e| {
            tracing::warn!("Failed to get subscription {}: {}", subscription_id, e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to get subscription: {}", e))
        })?;

    // If already canceled, update DB and return success (idempotent)
    if current_subscription.status.to_string() == "canceled" {
        tracing::info!(
            "Subscription {} is already canceled, updating DB (workspace {})",
            subscription_id,
            workspace_id
        );

        // Clear subscription from DB since it's fully canceled
        WorkspaceRepository::clear_stripe_subscription(&state.db, workspace_id)
            .await
            .map_err(db_error)?;

        // Downgrade to free plan
        WorkspaceRepository::update_plan(&state.db, workspace_id, mcp_common::types::Plan::Free)
            .await
            .map_err(db_error)?;

        return Ok(Json(CancelResponse {
            status: "canceled".to_string(),
            cancel_at_period_end: false,
            current_period_end: None,
        }));
    }

    // If already set to cancel at period end, just return current state
    if current_subscription.cancel_at_period_end {
        tracing::info!(
            "Subscription {} is already scheduled for cancellation (workspace {})",
            subscription_id,
            workspace_id
        );

        return Ok(Json(CancelResponse {
            status: current_subscription.status.to_string(),
            cancel_at_period_end: true,
            current_period_end: Some(current_subscription.current_period_end),
        }));
    }

    // Cancel subscription at period end in Stripe
    let subscription = billing
        .cancel_subscription(&subscription_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to cancel subscription: {}", e)))?;

    // Update database immediately to reflect cancel_at_period_end status
    // This ensures the UI shows the cancellation status right away
    if subscription.cancel_at_period_end {
        let period_end = chrono::DateTime::from_timestamp(subscription.current_period_end, 0);
        WorkspaceRepository::update_subscription_status(
            &state.db,
            workspace_id,
            "active", // Still active until period end
            period_end,
        )
        .await
        .map_err(db_error)?;
    }

    tracing::info!(
        "Subscription {} scheduled for cancellation at period end (workspace {})",
        subscription_id,
        workspace_id
    );

    Ok(Json(CancelResponse {
        status: subscription.status.to_string(),
        cancel_at_period_end: subscription.cancel_at_period_end,
        current_period_end: Some(subscription.current_period_end),
    }))
}

/// Create a customer portal session
pub async fn create_portal_session(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<PortalResponse>, (StatusCode, String)> {
    // Verify user is owner/admin
    workspace::require_admin(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    let customer_id = workspace
        .stripe_customer_id
        .ok_or((StatusCode::BAD_REQUEST, "No billing setup for this workspace".to_string()))?;

    let billing = state.billing.as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Billing not configured".to_string()))?;

    let session = billing
        .create_portal_session(&customer_id)
        .await
        .map_err(db_error)?;

    Ok(Json(PortalResponse {
        portal_url: session.url,
    }))
}

/// Handle Stripe webhooks
pub async fn handle_webhook(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, (StatusCode, String)> {
    // SECURITY: Rate limit webhook requests to prevent abuse
    let ip = extract_client_ip(&headers, &addr);
    let rate_key = format!("{}{}", WEBHOOK_RATE_LIMIT_PREFIX, ip);

    let lua_script = r#"
        local current = redis.call('INCR', KEYS[1])
        if current == 1 then
            redis.call('EXPIRE', KEYS[1], ARGV[1])
        end
        return current
    "#;

    let result: Result<i64, _> = LuaInterface::eval(
        &state.redis,
        lua_script,
        vec![rate_key],
        vec![WEBHOOK_RATE_LIMIT_WINDOW_SECS.to_string()],
    )
    .await;

    if let Ok(count) = result {
        if count > WEBHOOK_RATE_LIMIT_MAX {
            tracing::warn!("Stripe webhook rate limit exceeded for IP: {}", ip);
            return Err((StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded".to_string()));
        }
    }

    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::BAD_REQUEST, "Missing signature".to_string()))?;

    let webhook_handler = state.webhook_handler.as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Webhook handler not configured".to_string()))?;

    let event = webhook_handler
        .verify_event(&body, signature)
        .map_err(|e| {
            tracing::warn!("Webhook signature verification failed: {}", e);
            (StatusCode::BAD_REQUEST, "Invalid webhook signature".to_string())
        })?;

    webhook_handler
        .handle_event(event)
        .await
        .map_err(|e| internal_error("Webhook handler error", e))?;

    Ok(StatusCode::OK)
}

// Helper function to get price ID
fn get_price_id(plan: &str, yearly: bool) -> Option<String> {
    let env_key = match (plan, yearly) {
        ("pro", false) => "STRIPE_PRICE_PRO_MONTHLY",
        ("pro", true) => "STRIPE_PRICE_PRO_YEARLY",
        ("team", false) => "STRIPE_PRICE_TEAM_MONTHLY",
        ("team", true) => "STRIPE_PRICE_TEAM_YEARLY",
        ("enterprise", false) => "STRIPE_PRICE_ENTERPRISE_MONTHLY",
        ("enterprise", true) => "STRIPE_PRICE_ENTERPRISE_YEARLY",
        _ => return None,
    };

    std::env::var(env_key).ok()
}

// Request/Response types
#[derive(Debug, Serialize)]
pub struct PlanResponse {
    pub plan: String,
    pub name: String,
    pub description: String,
    /// Price in JPY per month. None means "contact us" pricing.
    pub price_monthly_jpy: Option<u32>,
    /// Price in JPY per year. None means "contact us" pricing.
    pub price_yearly_jpy: Option<u32>,
    pub features: Vec<String>,
    pub limits: PlanLimitsResponse,
}

#[derive(Debug, Serialize)]
pub struct PlanLimitsResponse {
    pub max_servers: u32,
    pub max_deployments_per_month: u32,
    pub max_requests_per_month: u64,
    pub max_team_members: u32,
    pub max_memory_mb: u32,
    pub log_retention_days: u32,
    pub custom_domains: bool,
    pub priority_support: bool,
    pub sso_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct ServerUsage {
    pub server_id: Uuid,
    pub name: String,
    pub gb_hours: f64,
}

#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub plan: String,
    /// Whether this plan is usage-billed (paid) vs flat-capped (Free).
    pub metered: bool,
    pub total_gb_hours: f64,
    pub rate_jpy_per_gb_hour: f64,
    pub estimated_cost_jpy: f64,
    pub servers: Vec<ServerUsage>,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionResponse {
    pub plan: String,
    pub status: String,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub current_period_start: Option<i64>,
    pub current_period_end: Option<i64>,
    pub cancel_at_period_end: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateCheckoutRequest {
    pub plan: String,
    #[serde(default)]
    pub yearly: bool,
}

#[derive(Debug, Serialize)]
pub struct CheckoutResponse {
    pub checkout_url: String,
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct PortalResponse {
    pub portal_url: String,
}

#[derive(Debug, Serialize)]
pub struct CancelResponse {
    pub status: String,
    pub cancel_at_period_end: bool,
    pub current_period_end: Option<i64>,
}

/// Get billing settings for a workspace
pub async fn get_billing_settings(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<BillingSettingsResponse>, (StatusCode, String)> {
    // Verify user has access
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    Ok(Json(BillingSettingsResponse {
        auto_email_invoices: workspace.auto_email_invoices.unwrap_or(true),
    }))
}

/// Update billing settings for a workspace
pub async fn update_billing_settings(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<UpdateBillingSettingsRequest>,
) -> Result<Json<BillingSettingsResponse>, (StatusCode, String)> {
    workspace::require_admin(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    WorkspaceRepository::update_billing_settings(&state.db, workspace_id, body.auto_email_invoices)
        .await
        .map_err(db_error)?;

    Ok(Json(BillingSettingsResponse {
        auto_email_invoices: body.auto_email_invoices,
    }))
}

#[derive(Debug, Serialize)]
pub struct BillingSettingsResponse {
    pub auto_email_invoices: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBillingSettingsRequest {
    pub auto_email_invoices: bool,
}

#[derive(Debug, Serialize)]
pub struct InvoiceResponse {
    pub id: String,
    pub number: Option<String>,
    pub status: Option<String>,
    pub amount_due: i64,
    pub amount_paid: i64,
    pub currency: String,
    pub created: i64,
    pub hosted_invoice_url: Option<String>,
    pub invoice_pdf: Option<String>,
}

/// Get payment method for a workspace
pub async fn get_payment_method(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<PaymentMethodResponse>, (StatusCode, String)> {
    // Verify user has access
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    let customer_id = match workspace.stripe_customer_id {
        Some(id) => id,
        None => return Ok(Json(PaymentMethodResponse { payment_method: None })),
    };

    let billing = state.billing.as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Billing not configured".to_string()))?;

    // Get payment methods for customer using Stripe API
    let payment_method = billing
        .get_default_payment_method(&customer_id)
        .await
        .ok()
        .flatten();

    Ok(Json(PaymentMethodResponse { payment_method }))
}

#[derive(Debug, Serialize)]
pub struct PaymentMethodResponse {
    pub payment_method: Option<PaymentMethodDetails>,
}

/// List invoices for a workspace
pub async fn list_invoices(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<InvoiceResponse>>, (StatusCode, String)> {
    // Verify user has access
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    // Return empty list if no stripe customer or billing not configured
    let Some(customer_id) = workspace.stripe_customer_id else {
        return Ok(Json(vec![]));
    };

    let Some(billing) = state.billing.as_ref() else {
        return Ok(Json(vec![]));
    };

    let invoices = match billing.list_invoices(&customer_id, 100).await {
        Ok(v) => v,
        // Unknown/stale customer → no invoices, not a hard error.
        Err(e) if is_missing_stripe_resource(&e) => return Ok(Json(vec![])),
        Err(e) => return Err(db_error(e)),
    };

    let response: Vec<InvoiceResponse> = invoices
        .into_iter()
        .map(|inv| InvoiceResponse {
            id: inv.id.to_string(),
            number: inv.number,
            status: inv.status.map(|s| s.to_string()),
            amount_due: inv.amount_due.unwrap_or(0),
            amount_paid: inv.amount_paid.unwrap_or(0),
            currency: inv.currency.map(|c| c.to_string()).unwrap_or_else(|| "jpy".to_string()),
            created: inv.created.unwrap_or(0),
            hosted_invoice_url: inv.hosted_invoice_url,
            invoice_pdf: inv.invoice_pdf,
        })
        .collect();

    Ok(Json(response))
}

#[derive(Debug, Serialize)]
pub struct SubscriptionHistoryItem {
    pub id: String,
    pub plan: String,
    pub status: String,
    pub current_period_start: i64,
    pub current_period_end: i64,
    pub canceled_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub cancel_at_period_end: bool,
}

/// List subscription history for a workspace (including canceled)
pub async fn list_subscription_history(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path(workspace_id): Path<Uuid>,
) -> Result<Json<Vec<SubscriptionHistoryItem>>, (StatusCode, String)> {
    // Verify user has access
    workspace::require_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(|e| (e.status, e.body.error.message))?;

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_WORKSPACE_NOT_FOUND.to_string()))?;

    // Return empty list if no stripe customer or billing not configured
    let Some(customer_id) = workspace.stripe_customer_id else {
        return Ok(Json(vec![]));
    };

    let Some(billing) = state.billing.as_ref() else {
        return Ok(Json(vec![]));
    };

    let subscriptions = match billing.list_subscriptions(&customer_id).await {
        Ok(v) => v,
        // Unknown/stale customer → no subscription history, not a hard error.
        Err(e) if is_missing_stripe_resource(&e) => return Ok(Json(vec![])),
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to list subscriptions: {}", e),
            ))
        }
    };

    let response: Vec<SubscriptionHistoryItem> = subscriptions
        .into_iter()
        .map(|sub| {
            let plan = billing.get_plan_from_subscription(&sub);

            SubscriptionHistoryItem {
                id: sub.id.to_string(),
                plan: plan.to_string(),
                status: sub.status.to_string(),
                current_period_start: sub.current_period_start,
                current_period_end: sub.current_period_end,
                canceled_at: sub.canceled_at,
                ended_at: sub.ended_at,
                cancel_at_period_end: sub.cancel_at_period_end,
            }
        })
        .collect();

    Ok(Json(response))
}

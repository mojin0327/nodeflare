pub mod auth;
pub mod billing;
pub mod console;
pub mod contact;
pub mod github;
pub mod health;
pub mod inspect;
pub mod openapi;
pub mod servers;
pub mod wireguard;
pub mod workspaces;
pub mod members;
pub mod tools;
pub mod deployments;
pub mod access_tokens;
pub mod secrets;
pub mod logs;
pub mod ws;
pub mod announcements;
pub mod user_preferences;
pub mod notifications;
pub mod webhooks;
pub mod test;
pub mod oauth;
pub mod stats;
pub mod github_accounts;

use axum::{routing::{get, post, patch, delete}, Router};
use std::sync::Arc;
use crate::state::AppState;

pub fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        // Auth - GitHub OAuth
        .route("/auth/github", get(auth::github_login))
        .route("/auth/github/callback", get(auth::github_callback))
        // Auth - Google OAuth
        .route("/auth/google", get(auth::google_login))
        .route("/auth/google/callback", get(auth::google_callback))
        // Auth - Email/Password
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/verify-email", get(auth::verify_email))
        .route("/auth/forgot-password", post(auth::forgot_password))
        .route("/auth/reset-password", post(auth::reset_password))
        .route("/auth/resend-verification", post(auth::resend_verification))
        // Auth - Common
        .route("/auth/refresh", post(auth::refresh_token))
        .route("/auth/me", get(auth::get_current_user))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/profile", patch(auth::update_profile))
        .route("/auth/account", delete(auth::delete_account))
        .route("/auth/ws-token", get(auth::ws_token))
        // GitHub
        .route("/github/repos", get(github::list_repositories))
        .route("/github/branches", get(github::list_branches))
        // GitHub Account Linking (callback handled by /auth/github/callback)
        .route("/github/accounts", get(github_accounts::list_accounts))
        .route("/github/accounts/link", get(github_accounts::link_account))
        .route("/github/accounts/:account_id", delete(github_accounts::unlink_account))
        .route("/github/accounts/:account_id/primary", post(github_accounts::set_primary))
        // Workspaces
        .route("/workspaces", get(workspaces::list).post(workspaces::create))
        .route(
            "/workspaces/:workspace_id",
            get(workspaces::get)
                .patch(workspaces::update)
                .delete(workspaces::delete),
        )
        // Workspace Members
        .route(
            "/workspaces/:workspace_id/members",
            get(members::list).post(members::add),
        )
        .route(
            "/workspaces/:workspace_id/members/:user_id",
            patch(members::update).delete(members::remove),
        )
        // Non-scoped aggregate lists (all of a user's workspaces). DEPRECATED: kept only
        // for backward compat during the workspace-scoped migration so the currently
        // deployed frontend keeps working; remove once the scoped frontend is live.
        .route("/servers/inspect", post(inspect::inspect))
        .route("/servers", get(servers::list_all))
        .route("/servers/minimal", get(servers::list_all_minimal))
        .route("/servers/basic", get(servers::list_all_basic))
        .route("/servers/list", get(servers::list_all_list))
        // Servers (workspace scoped)
        .route("/workspaces/:workspace_id/servers", get(servers::list).post(servers::create))
        .route("/workspaces/:workspace_id/servers/minimal", get(servers::list_minimal))
        .route("/workspaces/:workspace_id/servers/basic", get(servers::list_basic))
        .route("/workspaces/:workspace_id/servers/summary", get(servers::list_summary))
        .route(
            "/workspaces/:workspace_id/servers/:server_id",
            get(servers::get)
                .patch(servers::update)
                .delete(servers::delete),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/deploy",
            post(servers::deploy),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/stop",
            post(servers::stop),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/restart",
            post(servers::restart),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/metrics",
            get(servers::metrics),
        )
        // Tools
        .route(
            "/workspaces/:workspace_id/servers/:server_id/tools",
            get(tools::list),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/tools/:tool_id",
            patch(tools::update),
        )
        // Deployments
        .route(
            "/workspaces/:workspace_id/servers/:server_id/deployments",
            get(deployments::list),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/deployments/:deployment_id",
            get(deployments::get),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/deployments/:deployment_id/logs",
            get(deployments::get_logs),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/deployments/:deployment_id/rollback",
            post(deployments::rollback),
        )
        .route(
            "/workspaces/:workspace_id/deployments/usage",
            get(deployments::usage),
        )
        // Access Tokens
        .route(
            "/workspaces/:workspace_id/access-tokens",
            get(access_tokens::list).post(access_tokens::create),
        )
        .route(
            "/workspaces/:workspace_id/access-tokens/:key_id",
            delete(access_tokens::delete),
        )
        // Secrets
        .route(
            "/workspaces/:workspace_id/servers/:server_id/secrets",
            get(secrets::list).post(secrets::set),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/secrets/:key",
            delete(secrets::delete),
        )
        // Logs
        .route(
            "/workspaces/:workspace_id/servers/:server_id/logs",
            get(logs::list),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/stats",
            get(logs::stats),
        )
        // Batch stats for dashboard (single request for all servers)
        .route(
            "/workspaces/:workspace_id/stats",
            get(logs::batch_stats),
        )
        // Webhooks
        .route(
            "/workspaces/:workspace_id/servers/:server_id/webhooks",
            get(webhooks::list).post(webhooks::create),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/webhooks/:webhook_id",
            patch(webhooks::update).delete(webhooks::delete),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/webhooks/:webhook_id/test",
            post(webhooks::test),
        )
        // Billing
        .route("/billing/plans", get(billing::list_plans))
        .route(
            "/workspaces/:workspace_id/billing/subscription",
            get(billing::get_subscription),
        )
        .route(
            "/workspaces/:workspace_id/billing/usage",
            get(billing::get_usage),
        )
        .route(
            "/workspaces/:workspace_id/billing/checkout",
            post(billing::create_checkout),
        )
        .route(
            "/workspaces/:workspace_id/billing/change-plan",
            post(billing::change_plan),
        )
        .route(
            "/workspaces/:workspace_id/billing/portal",
            post(billing::create_portal_session),
        )
        .route(
            "/workspaces/:workspace_id/billing/cancel",
            post(billing::cancel_subscription),
        )
        .route(
            "/workspaces/:workspace_id/billing/invoices",
            get(billing::list_invoices),
        )
        .route(
            "/workspaces/:workspace_id/billing/subscriptions",
            get(billing::list_subscription_history),
        )
        .route(
            "/workspaces/:workspace_id/billing/payment-method",
            get(billing::get_payment_method),
        )
        .route(
            "/workspaces/:workspace_id/billing/settings",
            get(billing::get_billing_settings).patch(billing::update_billing_settings),
        )
        // Stripe webhook (no auth required)
        .route("/webhooks/stripe", post(billing::handle_webhook))
        // Public stats (no auth required)
        .route("/public/stats", get(stats::get_stats))
        // Contact (no auth required)
        .route("/contact", post(contact::submit_contact))
        // Announcements (public list, admin for CRUD)
        .route("/announcements", get(announcements::list).post(announcements::create))
        .route("/announcements/all", get(announcements::list_all))
        .route("/announcements/:id", patch(announcements::update).delete(announcements::delete))
        // User Preferences
        .route(
            "/user/preferences",
            get(user_preferences::get_preferences).patch(user_preferences::update_preferences),
        )
        // User Notifications
        .route(
            "/user/notifications",
            get(notifications::get_settings).patch(notifications::update_settings),
        )
        // Console (Machine Exec)
        .route(
            "/workspaces/:workspace_id/servers/:server_id/console/exec",
            post(console::exec_command),
        )
        // WireGuard VPN
        .route(
            "/workspaces/:workspace_id/wireguard",
            get(wireguard::list_wireguard_peers).post(wireguard::create_wireguard_peer),
        )
        .route(
            "/workspaces/:workspace_id/wireguard/:peer_name",
            delete(wireguard::delete_wireguard_peer),
        )
        // MCP Server Testing
        .route(
            "/workspaces/:workspace_id/servers/:server_id/test",
            get(test::health_check),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/test/execute",
            post(test::execute_tool),
        )
        // OAuth Client Management
        .route(
            "/workspaces/:workspace_id/oauth-apps",
            get(oauth::list_clients).post(oauth::create_client),
        )
        .route(
            "/workspaces/:workspace_id/oauth-apps/:client_id",
            delete(oauth::delete_client),
        )
        // Server OAuth (auto-generated per server)
        .route(
            "/workspaces/:workspace_id/servers/:server_id/oauth",
            get(oauth::get_server_oauth),
        )
        .route(
            "/workspaces/:workspace_id/servers/:server_id/oauth/regenerate",
            post(oauth::regenerate_server_oauth_secret),
        )
        // OAuth Authorization Code (called from frontend for logged-in users)
        .route("/oauth/client-info", get(oauth::client_info))
        .route("/oauth/authorize-code", post(oauth::authorize_code))
}

/// WebSocket router for real-time updates
pub fn ws_router() -> Router<Arc<AppState>> {
    Router::new()
        // Deployment status updates
        .route(
            "/deployments/:deployment_id",
            get(ws::deployment_ws),
        )
        // Build logs streaming
        .route(
            "/deployments/:deployment_id/logs",
            get(ws::build_logs_ws),
        )
        // Server status updates
        .route(
            "/workspaces/:workspace_id/servers/:server_id/status",
            get(ws::server_status_ws),
        )
        // Server logs streaming
        .route(
            "/workspaces/:workspace_id/servers/:server_id/logs",
            get(ws::server_logs_ws),
        )
}

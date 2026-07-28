use anyhow::Result;
use axum::{
    http::{header, HeaderName, HeaderValue, Method},
    middleware as axum_middleware,
    routing::get,
    Router,
};
use fred::interfaces::ClientLike;
use mcp_common::AppConfig;
use std::{net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod cache;
mod error;
mod extractors;
mod middleware;
mod redis_subscriber;
mod routes;
mod state;
mod ws_manager;

use middleware::rate_limit_middleware;
use state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    // Install rustls crypto provider (required for TLS connections)
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mcp_api=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = AppConfig::from_env()?;
    tracing::info!("Starting MCP Cloud API server");

    // Create database pool
    let db_pool = mcp_db::create_pool(&config).await?;

    // Create Redis client with reconnection policy
    let redis_config = fred::types::RedisConfig::from_url(&config.redis.url)?;
    let reconnect_policy = fred::types::ReconnectPolicy::new_exponential(0, 1, 30_000, 2);
    let redis = fred::prelude::RedisClient::new(redis_config, None, None, Some(reconnect_policy));
    redis.connect();
    redis.wait_for_connect().await?;
    tracing::info!("Connected to Redis");

    // Create job queue for background tasks
    let job_queue = Arc::new(mcp_queue::JobQueue::connect(&config.redis.url).await?);
    tracing::info!("Connected to job queue");

    // Start job queue keep-alive task to prevent Upstash idle timeout
    job_queue.clone().start_keepalive_task();
    tracing::info!("Job queue keep-alive task started");

    // Create GitHub App client (optional)
    let github = mcp_github::GitHubApp::new(&config).ok();
    if github.is_some() {
        tracing::info!("GitHub App initialized");
    } else {
        tracing::warn!("GitHub App not configured - private repos will not be accessible");
    }

    // Create app state
    let state = Arc::new(AppState::new(config.clone(), db_pool.clone(), redis, job_queue, github));

    // Start WsManager cleanup task
    let ws_manager_arc = Arc::new(state.ws_manager.clone());
    ws_manager_arc.clone().start_cleanup_task();
    tracing::info!("WsManager cleanup task started");

    // Start Redis subscriber for WebSocket events
    redis_subscriber::start_redis_subscriber(
        &config.redis.url,
        ws_manager_arc,
    )
    .await;
    tracing::info!("Redis subscriber started for WebSocket events");

    // Start request_logs cleanup task
    start_request_logs_cleanup_task(db_pool.clone());
    start_usage_sampler_task(db_pool.clone());
    start_usage_reporting_task(db_pool.clone(), state.billing.clone());
    tracing::info!("Request logs cleanup task started");

    // Start deployment timeout task
    start_deployment_timeout_task(db_pool.clone());
    tracing::info!("Deployment timeout task started");

    // Start upstream OAuth token refresh task
    start_upstream_token_refresh_task(db_pool.clone(), state.http.clone());
    tracing::info!("Upstream OAuth token refresh task started");

    // Build router
    let app = create_router(state);

    // Start server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("API server listening on {}", addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// Start background task to clean up old request_logs
fn start_request_logs_cleanup_task(db_pool: mcp_db::DbPool) {
    use chrono::{Duration, Utc};
    use mcp_db::RequestLogRepository;

    // Get retention days from env (default: 30 days)
    let retention_days: i64 = std::env::var("REQUEST_LOGS_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    // Get cleanup interval from env (default: 1 hour)
    let cleanup_interval_secs: u64 = std::env::var("REQUEST_LOGS_CLEANUP_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(cleanup_interval_secs));
        loop {
            interval.tick().await;
            let cutoff = Utc::now() - Duration::days(retention_days);
            match RequestLogRepository::delete_old_logs(&db_pool, cutoff).await {
                Ok(deleted) => {
                    if deleted > 0 {
                        tracing::info!("Cleaned up {} old request logs (older than {} days)", deleted, retention_days);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to clean up old request logs: {}", e);
                }
            }
        }
    });
}

/// Start background task to timeout stuck deployments
fn start_deployment_timeout_task(db_pool: mcp_db::DbPool) {
    use mcp_db::DeploymentRepository;

    // Get timeout in minutes from env (default: 15 minutes)
    let timeout_minutes: i64 = std::env::var("DEPLOYMENT_TIMEOUT_MINUTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);

    // Check every minute
    let check_interval_secs: u64 = std::env::var("DEPLOYMENT_TIMEOUT_CHECK_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(check_interval_secs));
        loop {
            interval.tick().await;
            match DeploymentRepository::timeout_stuck_deployments(&db_pool, timeout_minutes).await {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!("Timed out {} stuck deployments (older than {} minutes)", count, timeout_minutes);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to timeout stuck deployments: {}", e);
                }
            }
        }
    });
}

/// Hourly background job: refresh upstream OAuth tokens (Google Drive, GitHub, etc.)
/// that are within 5 days of expiry. Failures are logged but do not stop the loop.
fn start_upstream_token_refresh_task(db_pool: mcp_db::DbPool, http: reqwest::Client) {
    use mcp_db::ServerRepository;
    use crate::routes::oauth::{refresh_upstream_token, resolve_credentials};

    let refresh_interval_secs: u64 = std::env::var("UPSTREAM_TOKEN_REFRESH_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600); // default: 1 hour

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(refresh_interval_secs));
        loop {
            interval.tick().await;

            let servers = match ServerRepository::list_running_with_upstream_oauth(&db_pool).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Upstream token refresh: failed to list servers: {}", e);
                    continue;
                }
            };

            // We need a Redis client for token reads/writes.
            // Re-connect cheaply using the same URL from env.
            let redis_url = match std::env::var("REDIS_URL") {
                Ok(u) => u,
                Err(_) => {
                    tracing::warn!("Upstream token refresh: REDIS_URL not set, skipping");
                    continue;
                }
            };
            let redis_client = match redis::Client::open(redis_url.as_str()) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Upstream token refresh: redis client error: {}", e);
                    continue;
                }
            };
            let mut conn = match redis_client.get_multiplexed_async_connection().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Upstream token refresh: redis connect error: {}", e);
                    continue;
                }
            };

            let now = chrono::Utc::now().timestamp();
            const REFRESH_BEFORE_SECS: i64 = 5 * 24 * 3600; // 5 days

            for server in servers {
                let provider = match server.upstream_oauth_provider.as_deref() {
                    Some(p) => p.to_string(),
                    None => continue,
                };

                let redis_key = format!("mcp:oauth:{}", server.id);
                let raw: Option<String> = match redis::AsyncCommands::get(&mut conn, &redis_key).await {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let raw = match raw {
                    Some(r) => r,
                    None => continue, // no token stored yet; nothing to refresh
                };

                #[derive(serde::Deserialize)]
                struct StoredToken {
                    refresh_token: String,
                    expires_at: i64,
                }

                let stored: StoredToken = match serde_json::from_str(&raw) {
                    Ok(t) => t,
                    Err(_) => continue,
                };

                // Skip if not near expiry
                if stored.expires_at - now > REFRESH_BEFORE_SECS {
                    continue;
                }

                tracing::info!(
                    "Upstream token refresh: refreshing {} token for server {}",
                    provider, server.id
                );

                let creds = match resolve_credentials(&db_pool, &provider, &server).await {
                    Ok(c) => c,
                    Err((_, e)) => {
                        tracing::warn!("Upstream token refresh: failed to resolve credentials for {} server {}: {}", provider, server.id, e);
                        continue;
                    }
                };

                match refresh_upstream_token(&http, &creds.token_url, &creds.client_id, &creds.client_secret, &stored.refresh_token).await {
                    Ok((new_access, new_refresh, expires_in)) => {
                        let new_expires_at = now + expires_in;
                        let new_refresh_token = new_refresh.unwrap_or(stored.refresh_token);
                        let payload = serde_json::json!({
                            "access_token": new_access,
                            "refresh_token": new_refresh_token,
                            "expires_at": new_expires_at,
                        })
                        .to_string();
                        let ttl = expires_in + 90 * 24 * 3600;
                        let _: Result<(), _> = redis::AsyncCommands::set_ex(
                            &mut conn,
                            &redis_key,
                            payload,
                            ttl as u64,
                        )
                        .await;
                        tracing::info!(
                            "Upstream token refresh: refreshed {} for server {}",
                            provider, server.id
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Upstream token refresh: failed to refresh {} for server {}: {}",
                            provider, server.id, e
                        );
                    }
                }
            }
        }
    });
}

/// Background task: every interval, sample each running server's container memory usage
/// and accrue memory-weighted active time (GB-minutes) for usage billing.
fn start_usage_sampler_task(db_pool: mcp_db::DbPool) {
    use mcp_db::{RegionUsageRepository, ServerRepository};

    let interval_secs: u64 = std::env::var("USAGE_SAMPLE_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);

    let builder_url = std::env::var("BUILDER_INTERNAL_URL")
        .unwrap_or_else(|_| "http://localhost:8083".to_string());
    let builder_token = std::env::var("BUILDER_TOKEN").unwrap_or_default();

    let interval_minutes = interval_secs as f64 / 60.0;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;

            let servers = match ServerRepository::list_running(&db_pool).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("usage sampler: failed to list running servers: {}", e);
                    continue;
                }
            };

            for server in servers {
                // Verify container is running via Builder stats endpoint
                let stats_url = format!("{}/internal/stats", builder_url);
                let mut req = http.get(&stats_url);
                if !builder_token.is_empty() {
                    req = req.bearer_auth(&builder_token);
                }
                let running = match req.send().await {
                    Ok(resp) => resp.json::<serde_json::Value>().await
                        .map(|v| v.get("containers").and_then(|c| c.as_str())
                            .map(|s| s.contains(&server.container_name))
                            .unwrap_or(false))
                        .unwrap_or(false),
                    Err(_) => false,
                };

                if !running {
                    continue;
                }

                // Accrue usage based on configured memory
                let memory_mb = server.memory_mb.unwrap_or(256) as f64;
                let gb_minutes = (memory_mb / 1024.0) * interval_minutes;

                if let Err(e) = RegionUsageRepository::add_gb_minutes(
                    &db_pool,
                    server.workspace_id,
                    server.id,
                    &server.region,
                    gb_minutes,
                )
                .await
                {
                    tracing::error!("usage sampler: failed to accrue usage for {}: {}", server.id, e);
                }
            }
        }
    });
}

/// Background task: report accumulated GB-hours to Stripe as metered usage for paid
/// workspaces, then mark those rows reported. Dormant unless a Stripe usage price
/// (`STRIPE_PRICE_PRO_USAGE`) and a billing client are configured — so it never bills
/// until you intentionally enable it.
fn start_usage_reporting_task(db_pool: mcp_db::DbPool, billing: Option<mcp_billing::BillingService>) {
    use mcp_db::{RegionUsageRepository, WorkspaceRepository};

    let usage_price = match std::env::var("STRIPE_PRICE_PRO_USAGE") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            tracing::info!("Usage reporting disabled: STRIPE_PRICE_PRO_USAGE not set");
            return;
        }
    };
    let Some(billing) = billing else {
        tracing::warn!("Usage reporting disabled: billing not configured");
        return;
    };

    // The Billing Meter's event_name (usage is reported per-customer to this meter).
    // Must match the meter backing STRIPE_PRICE_PRO_USAGE in Stripe.
    let meter_event = std::env::var("STRIPE_USAGE_METER_EVENT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "nodeflare_gb_hours".to_string());

    // Report hourly by default. Stripe aggregates increments within the period.
    let interval_secs: u64 = std::env::var("USAGE_REPORT_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;

            let workspaces = match RegionUsageRepository::list_unreported_workspaces(&db_pool).await {
                Ok(w) => w,
                Err(e) => {
                    tracing::error!("usage reporting: failed to list workspaces: {}", e);
                    continue;
                }
            };

            for ws_id in workspaces {
                let Ok(Some(ws)) = WorkspaceRepository::find_by_id(&db_pool, ws_id).await else {
                    continue;
                };
                // Only paid plans are usage-billed; Free is flat-capped.
                if ws.plan == "free" {
                    continue;
                }
                let Some(subscription_id) = ws.stripe_subscription_id.as_deref() else {
                    continue;
                };
                let Some(customer_id) = ws.stripe_customer_id.as_deref() else {
                    continue;
                };

                // Gate: only bill workspaces actually subscribed to the usage price. Existing
                // Pro subs that predate usage billing have no such item, so we skip them and
                // keep accumulating (they're never charged until the item is added).
                match billing
                    .find_metered_subscription_item(subscription_id, &usage_price)
                    .await
                {
                    Ok(Some(_)) => {} // subscribed to the usage price → bill it
                    Ok(None) => continue, // subscription has no usage item yet
                    Err(e) => {
                        tracing::warn!("usage reporting: lookup item failed for {}: {}", ws_id, e);
                        continue;
                    }
                };

                let rows = match RegionUsageRepository::list_unreported(&db_pool, ws_id).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("usage reporting: list_unreported failed: {}", e);
                        continue;
                    }
                };
                let gb_hours: f64 = rows.iter().map(|r| r.gb_minutes).sum::<f64>() / 60.0;
                let quantity = gb_hours.round() as u64;
                // Below 1 GB-hour: leave unreported so it accumulates (avoids reporting 0).
                if quantity == 0 {
                    continue;
                }

                // Deterministic per-batch identifier so a crash between reporting and marking
                // rows reported can't double-bill: the same unreported batch yields the same id.
                let batch_id = rows.iter().map(|r| r.id).max().unwrap();
                let identifier = format!("nf-{}-{}", ws_id, batch_id);

                let now_ts = chrono::Utc::now().timestamp();
                match billing
                    .report_meter_usage(customer_id, &meter_event, quantity, now_ts, &identifier)
                    .await
                {
                    Ok(record_id) => {
                        for r in &rows {
                            if let Err(e) =
                                RegionUsageRepository::mark_reported(&db_pool, r.id, &record_id).await
                            {
                                tracing::error!("usage reporting: mark_reported failed: {}", e);
                            }
                        }
                        tracing::info!("Reported {} GB-hours for workspace {}", quantity, ws_id);
                    }
                    Err(e) => {
                        tracing::error!("usage reporting: report_meter_usage failed for {}: {}", ws_id, e);
                    }
                }
            }
        }
    });
}

fn create_router(state: Arc<AppState>) -> Router {
    // CORS configuration for frontend
    let frontend_url = state.config.server.frontend_url.clone();
    let cors = CorsLayer::new()
        .allow_origin(
            frontend_url
                .parse::<HeaderValue>()
                .unwrap_or_else(|_| HeaderValue::from_static("http://localhost:3000")),
        )
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::ORIGIN,
            HeaderName::from_static("x-requested-with"),
        ])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(3600));

    // SECURITY NOTE: OAuth endpoints use permissive CORS intentionally.
    // This is required by OAuth 2.0 spec since clients can be on any domain.
    // Security is enforced at the application level through:
    // - Client authentication (client_id/secret) for confidential clients
    // - PKCE (Proof Key for Code Exchange) for public clients
    // - Rate limiting on dynamic client registration (10/hour per IP)
    // - Redirect URI validation to prevent authorization code theft
    // Do NOT use allow_credentials(true) with Any origin.
    let oauth_cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
        ])
        .max_age(std::time::Duration::from_secs(3600));

    // Check if rate limiting is enabled (default: true in production)
    let rate_limit_enabled = std::env::var("RATE_LIMIT_ENABLED")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(true);

    // Get request body limit from env (default: 1MB)
    let body_limit: usize = std::env::var("REQUEST_BODY_LIMIT_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1024 * 1024); // 1MB default

    // Main router with frontend CORS
    let main_router = Router::new()
        // Health check (no rate limiting)
        .route("/health", get(routes::health::health_check))
        .route("/ready", get(routes::health::readiness_check))
        // API v1 with rate limiting
        .nest("/api/v1", routes::api_router())
        // WebSocket endpoints (rate limiting handled at connection level)
        .nest("/ws", routes::ws_router())
        // OpenAPI docs
        .merge(routes::openapi::openapi_router())
        .layer(cors);

    // OAuth routes with permissive CORS (external clients like Claude need to access these)
    let oauth_router = Router::new()
        .route("/.well-known/oauth-authorization-server", get(routes::oauth::authorization_server_metadata))
        .route("/oauth/authorize", get(routes::oauth::authorize))
        .route("/oauth/token", axum::routing::post(routes::oauth::token))
        .route("/oauth/register", axum::routing::post(routes::oauth::register_client))
        .layer(oauth_cors);

    // Internal routes: accessible only via Fly 6PN, authenticated by NODEFLARE_SERVER_TOKEN.
    // No CORS needed — called server-to-server by hosted MCP apps over the private network.
    let internal_router = Router::new()
        .route(
            "/internal/mcp-token/:server_id",
            axum::routing::get(routes::mcp_tokens::get_token)
                .put(routes::mcp_tokens::put_token),
        );

    // Merge routers
    let router = Router::new()
        .merge(main_router)
        .merge(oauth_router)
        .merge(internal_router);

    // Apply rate limiting middleware conditionally
    let router = if rate_limit_enabled {
        router.layer(axum_middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
    } else {
        router
    };

    router
        // Middleware (applied to all routes)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(RequestBodyLimitLayer::new(body_limit))
        // Security headers
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-xss-protection"),
            HeaderValue::from_static("1; mode=block"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
        ))
        // HSTS - Enforce HTTPS for 1 year, including subdomains
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"),
        ))
        // Content-Security-Policy - Restrict resource loading
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'self'; \
                 script-src 'self' 'unsafe-inline' 'unsafe-eval'; \
                 style-src 'self' 'unsafe-inline'; \
                 img-src 'self' data: https:; \
                 font-src 'self' data:; \
                 connect-src 'self' wss: https:; \
                 frame-ancestors 'none'; \
                 base-uri 'self'; \
                 form-action 'self'"
            ),
        ))
        .with_state(state)
}

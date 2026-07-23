use fred::prelude::RedisClient;
use mcp_auth::{CryptoService, JwtService};
use mcp_billing::{BillingService, WebhookHandler};
use mcp_common::AppConfig;
use mcp_db::DbPool;
use mcp_email::EmailService;
use mcp_github::GitHubApp;
use mcp_queue::JobQueue;
use std::sync::Arc;

use crate::cache::ApiCache;
use crate::ws_manager::WsManager;

pub struct AppState {
    pub config: AppConfig,
    pub db: DbPool,
    pub redis: RedisClient,
    pub jwt: JwtService,
    pub crypto: CryptoService,
    pub job_queue: Arc<JobQueue>,
    pub github: Option<GitHubApp>,
    pub ws_manager: WsManager,
    pub billing: Option<BillingService>,
    pub webhook_handler: Option<WebhookHandler>,
    pub email: Option<EmailService>,
    /// URL of the Builder's internal HTTP API (for exec/stats/metrics from API → Builder)
    pub builder_internal_url: String,
    pub cache: ApiCache,
    /// Shared, connection-pooled HTTP client (keep-alive + HTTP/2). Reused for outbound
    /// calls like the repo-inspect GitHub requests so each request avoids a cold TLS
    /// handshake and can multiplex parallel fetches over one connection.
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        db: DbPool,
        redis: RedisClient,
        job_queue: Arc<JobQueue>,
        github: Option<GitHubApp>,
    ) -> Self {
        let jwt = JwtService::new(&config);

        // Initialize API cache
        let cache = ApiCache::new(redis.clone());

        // SECURITY: ENCRYPTION_KEY must be set in production
        // In development, generate a key if not set (with warning)
        let encryption_key = match std::env::var("ENCRYPTION_KEY") {
            Ok(key) => key,
            Err(_) => {
                if std::env::var("ENVIRONMENT").as_deref() == Ok("production") {
                    panic!("ENCRYPTION_KEY must be set in production environment");
                }
                tracing::warn!(
                    "ENCRYPTION_KEY not set - generating temporary key. \
                     This is only acceptable for development!"
                );
                CryptoService::generate_key()
                    .expect("Failed to generate encryption key - system RNG unavailable")
            }
        };
        let crypto = CryptoService::from_hex(&encryption_key)
            .expect("Invalid encryption key format");

        let ws_manager = WsManager::new();

        // Initialize Stripe billing (optional)
        let (billing, webhook_handler) = match (
            std::env::var("STRIPE_SECRET_KEY"),
            std::env::var("STRIPE_WEBHOOK_SECRET"),
        ) {
            (Ok(secret_key), Ok(webhook_secret)) => {
                let base_url = std::env::var("APP_URL")
                    .unwrap_or_else(|_| "http://localhost:3000".to_string());
                let billing = BillingService::new(&secret_key, &base_url);
                let webhook = WebhookHandler::new(
                    &webhook_secret,
                    db.clone(),
                    &secret_key,
                );
                tracing::info!("Stripe billing initialized");
                (Some(billing), Some(webhook))
            }
            _ => {
                tracing::warn!("Stripe not configured - billing features disabled");
                (None, None)
            }
        };

        // Initialize Resend email service (optional)
        let email = match EmailService::from_env() {
            Ok(service) => {
                tracing::info!("Resend email service initialized");
                Some(service)
            }
            Err(e) => {
                tracing::warn!("Email service not configured: {} - email features disabled", e);
                None
            }
        };

        let builder_internal_url = std::env::var("BUILDER_INTERNAL_URL")
            .unwrap_or_else(|_| "http://localhost:8083".to_string());

        // Shared outbound HTTP client: pooled keep-alive connections so repeated/parallel
        // GitHub calls (repo inspect) skip the TLS handshake and reuse one HTTP/2 conn.
        let http = reqwest::Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(16)
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("NodeFlare/1.0")
            .build()
            .unwrap_or_default();

        Self {
            config,
            db,
            redis,
            jwt,
            crypto,
            job_queue,
            github,
            ws_manager,
            billing,
            webhook_handler,
            email,
            builder_internal_url,
            cache,
            http,
        }
    }
}

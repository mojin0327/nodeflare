use anyhow::Result;
use apalis::prelude::*;
use apalis_redis::RedisStorage;
use axum::{routing::{get, post}, Router};
use mcp_auth::CryptoService;
use mcp_common::{types::LogStream, AppConfig, EventPublisher};
use mcp_db::{CreateSecret, DeploymentRepository, ErrorHintRepository, NotificationSettingsRepository, RegionStatus, SecretRepository, ServerRegionRepository, ServerRepository, UpdateDeployment, UpdateServerRegion, UserPreferencesRepository, UserRepository, WorkspaceRepository};
use mcp_email::EmailService;
use mcp_github::GitHubApp;
use mcp_queue::{BuildJob, DeployJob, DestroyJob, JobQueue};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod containerd;
mod firecracker;
mod flyctl;
mod fork_socket;
mod tap;

#[allow(dead_code)]
mod docker;

/// Send deployment notification email to workspace owner
async fn send_deploy_notification(
    ctx: &BuilderContext,
    server_id: uuid::Uuid,
    success: bool,
    error_message: Option<&str>,
) {
    let email_service = match &ctx.email {
        Some(s) => s,
        None => return,
    };

    // Get server -> workspace -> owner user. Log lookup failures rather than silently
    // dropping the notification, so a misconfigured row doesn't fail invisibly.
    let server = match ServerRepository::find_by_id(&ctx.db, server_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!("Deploy notification: failed to load server {}: {}", server_id, e);
            return;
        }
    };

    let workspace = match WorkspaceRepository::find_by_id(&ctx.db, server.workspace_id).await {
        Ok(Some(w)) => w,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!("Deploy notification: failed to load workspace {}: {}", server.workspace_id, e);
            return;
        }
    };

    let owner = match UserRepository::find_by_id(&ctx.db, workspace.owner_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!("Deploy notification: failed to load owner {}: {}", workspace.owner_id, e);
            return;
        }
    };

    // Check notification settings
    let settings = match NotificationSettingsRepository::get_or_create(&ctx.db, owner.id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Deploy notification: failed to load settings for {}: {}", owner.id, e);
            return;
        }
    };

    let app_url = std::env::var("APP_URL").unwrap_or_else(|_| "https://mcpcloud.dev".to_string());

    if success && settings.email_deploy_success {
        let deploy_url = format!("{}/dashboard/servers/{}", app_url, server_id);
        if let Err(e) = email_service.send_deploy_success(&owner.email, &server.name, &deploy_url).await {
            tracing::error!("Failed to send deploy success email: {}", e);
        }
    } else if !success && settings.email_deploy_failure {
        let logs_url = format!("{}/dashboard/servers/{}/logs", app_url, server_id);
        let error_msg = error_message.unwrap_or("Unknown error");
        if let Err(e) = email_service.send_deploy_failure(&owner.email, &server.name, error_msg, &logs_url).await {
            tracing::error!("Failed to send deploy failure email: {}", e);
        }
    }
}

/// Get user's locale preference from server_id
async fn get_user_locale_for_server(pool: &mcp_db::DbPool, server_id: uuid::Uuid) -> String {
    // Get server -> workspace -> owner -> preferences -> locale
    let locale = async {
        let server = ServerRepository::find_by_id(pool, server_id).await.ok()??;
        let workspace = WorkspaceRepository::find_by_id(pool, server.workspace_id).await.ok()??;
        let locale = UserPreferencesRepository::get_locale(pool, workspace.owner_id).await.ok()?;
        Some(locale)
    }.await;

    locale.unwrap_or_else(|| "en".to_string())
}

/// The memory ceiling (MB) the server's workspace plan allows. Looked up at deploy
/// time so a plan downgrade is respected. Falls back to the Free ceiling (smallest)
/// if the plan can't be resolved — never hand out more memory than we can confirm.
async fn plan_memory_ceiling_mb(pool: &mcp_db::DbPool, server_id: uuid::Uuid) -> u64 {
    use mcp_billing::Plan;
    let ceiling = async {
        let server = ServerRepository::find_by_id(pool, server_id).await.ok()??;
        let workspace = WorkspaceRepository::find_by_id(pool, server.workspace_id).await.ok()??;
        let plan = match workspace.plan.as_str() {
            "pro" => Plan::Pro,
            "team" => Plan::Team,
            "enterprise" => Plan::Enterprise,
            _ => Plan::Free,
        };
        Some(plan.limits().max_memory_mb as u64)
    }
    .await;
    ceiling.unwrap_or_else(|| mcp_billing::PlanLimits::default().max_memory_mb as u64)
}

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::collections::HashMap;
use tokio::sync::RwLock;

/// Container names accepted by the internal HTTP API: non-empty, ≤64 chars,
/// alphanumeric + hyphens only.  Prevents regex metacharacters from reaching
/// pkill/pgrep -f and path traversal in /proc/<pid>/status lookups.
fn is_valid_container_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// XOR-fold constant-time bearer token comparison.
/// Prevents statistical timing attacks on BUILDER_TOKEN from a co-located
/// process (e.g. a compromised Firecracker VM on the same host).
fn bearer_token_eq(presented: Option<&str>, expected: &str) -> bool {
    let presented = match presented {
        Some(s) => s,
        None => return false,
    };
    let expected_full = format!("Bearer {}", expected);
    let a = presented.as_bytes();
    let b = expected_full.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Per-process monotonic counter combined with a millisecond timestamp to give
/// fork names that are unique even under concurrent and same-millisecond requests.
static FORK_SEQ: AtomicU64 = AtomicU64::new(0);

/// Preprocessed error hint for efficient matching
/// Scalability: Keywords are pre-lowercased to avoid repeated string operations
#[derive(Clone, Debug)]
struct PreprocessedErrorHint {
    /// Original hint data
    hint_message: String,
    /// Pre-lowercased keywords for efficient matching
    keywords_lower: Vec<String>,
}

impl From<mcp_db::ErrorHint> for PreprocessedErrorHint {
    fn from(hint: mcp_db::ErrorHint) -> Self {
        Self {
            hint_message: hint.hint_message,
            // Pre-process keywords to lowercase once
            keywords_lower: hint.keywords.iter().map(|k| k.to_lowercase()).collect(),
        }
    }
}

/// Cache for error hints - keyed by locale, stores (preprocessed hints, last_updated)
static ERROR_HINTS_CACHE: OnceLock<RwLock<HashMap<String, (Vec<PreprocessedErrorHint>, std::time::Instant)>>> = OnceLock::new();

/// Cache TTL for error hints (5 minutes)
const ERROR_HINTS_CACHE_TTL_SECS: u64 = 300;

/// Maximum cache entries to prevent unbounded memory growth
const MAX_CACHE_LOCALES: usize = 100;

/// Get cached error hints for a locale, fetching from DB if needed
async fn get_cached_error_hints(pool: &mcp_db::DbPool, locale: &str) -> Vec<PreprocessedErrorHint> {
    let cache = ERROR_HINTS_CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    let now = std::time::Instant::now();

    // Check if we have valid cached hints
    {
        let cache_read = cache.read().await;
        if let Some((hints, last_updated)) = cache_read.get(locale) {
            if now.duration_since(*last_updated).as_secs() < ERROR_HINTS_CACHE_TTL_SECS {
                return hints.clone();
            }
        }
    }

    // Cache miss or expired - fetch from database
    match ErrorHintRepository::list_active(pool, locale).await {
        Ok(hints) => {
            // Pre-process hints for efficient matching
            let preprocessed: Vec<PreprocessedErrorHint> = hints.into_iter().map(|h| h.into()).collect();

            let mut cache_write = cache.write().await;

            // Scalability: Limit cache size to prevent unbounded memory growth
            if cache_write.len() >= MAX_CACHE_LOCALES {
                // Remove oldest entry (simple LRU approximation)
                if let Some(oldest_key) = cache_write
                    .iter()
                    .min_by_key(|(_, (_, instant))| *instant)
                    .map(|(k, _)| k.clone())
                {
                    cache_write.remove(&oldest_key);
                }
            }

            cache_write.insert(locale.to_string(), (preprocessed.clone(), now));
            preprocessed
        }
        Err(e) => {
            tracing::warn!("Failed to fetch error hints from database: {}", e);
            Vec::new()
        }
    }
}

/// Analyze error message and return user-friendly hints using cached hints
/// Scalability: Keywords are pre-lowercased in cache, error message is lowercased once
async fn analyze_error_for_hints(pool: &mcp_db::DbPool, error_msg: &str, locale: &str) -> Option<String> {
    // Lowercase error message once (not per-keyword)
    let error_lower = error_msg.to_lowercase();

    // First try to find a hint in the user's locale (using cache)
    let hints = get_cached_error_hints(pool, locale).await;

    for hint in &hints {
        // Keywords are already lowercased in cache
        let all_match = hint.keywords_lower.iter().all(|keyword| {
            error_lower.contains(keyword)
        });

        if all_match {
            return Some(format!("\n\n{}", hint.hint_message));
        }
    }

    // If no match found and locale is not 'en', fall back to English
    if locale != "en" {
        let en_hints = get_cached_error_hints(pool, "en").await;

        for hint in en_hints {
            // Keywords are already lowercased in cache
            let all_match = hint.keywords_lower.iter().all(|keyword| {
                error_lower.contains(keyword)
            });

            if all_match {
                return Some(format!("\n\n{}", hint.hint_message));
            }
        }
    }

    None
}

struct BuilderContext {
    config: AppConfig,
    db: mcp_db::DbPool,
    #[allow(dead_code)]
    job_queue: JobQueue,
    crypto: CryptoService,
    github: Option<GitHubApp>,
    events: EventPublisher,
    email: Option<EmailService>,
}

/// Maximum attempts a job is retried by the [`BoundedBackoffRetry`] layer before being
/// given up on. Retries fire only when a handler returns `Err` (a transient failure that
/// was NOT recorded as a terminal `Failed` status — see the Err-vs-Ok contract in
/// `handle_build_job`).
const MAX_JOB_RETRIES: usize = 3;

/// In-flight job concurrency per worker. Previously every worker was implicitly serial
/// (1), so a single slow build blocked the whole queue. Bounded so we don't fan out
/// unbounded remote builds. Overridable via `BUILDER_CONCURRENCY`.
fn worker_concurrency() -> usize {
    std::env::var("BUILDER_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(4)
}

/// Hard ceiling on a single build+deploy+verify cycle. A hung remote build or a probe
/// that never resolves must not pin a worker (or leave a deployment `Building`) forever.
const BUILD_DEPLOY_TIMEOUT_SECS: u64 = 25 * 60;

/// A deployment left in a non-terminal state longer than this is considered stuck (the
/// worker died mid-build, the process was OOM-killed, etc.) and is reaped to `Failed`.
const STUCK_DEPLOYMENT_TIMEOUT_MINUTES: i64 = 30;

/// How often the background reaper sweeps for stuck deployments.
const REAPER_INTERVAL_SECS: u64 = 5 * 60;

/// A server left in `deleting` longer than this had its destroy job fail to confirm (the
/// worker died mid-destroy, the queue dropped the job, or every retry failed). The orphan
/// sweeper re-drives the teardown so we don't leak a billable Fly app.
const DELETING_STUCK_TIMEOUT_MINUTES: i64 = 15;

/// How often the orphan-destroy sweeper runs (also once at startup to reconcile).
const ORPHAN_SWEEPER_INTERVAL_SECS: u64 = 10 * 60;

/// An apalis/tower retry policy: retries a failed job up to `max_retries` times with
/// capped exponential backoff (so a returned `Err` is actually re-attempted instead of
/// being silently terminal). Backoff is `5s · 2^attempt`, capped at 5 minutes, plus a
/// little jitter to avoid synchronized retries across workers.
#[derive(Clone)]
struct BoundedBackoffRetry {
    max_retries: usize,
}

impl<T, Res, Ctx> tower::retry::Policy<Request<T, Ctx>, Res, Error> for BoundedBackoffRetry
where
    T: Clone,
    Ctx: Clone,
{
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

    fn retry(
        &mut self,
        req: &mut Request<T, Ctx>,
        result: &mut Result<Res, Error>,
    ) -> Option<Self::Future> {
        if result.is_ok() {
            return None;
        }
        let attempt = req.parts.attempt.current();
        if attempt >= self.max_retries {
            return None;
        }
        let base = 5u64.saturating_mul(1u64 << attempt.min(6));
        let capped = base.min(300);
        // Cheap jitter (0..=2s) derived from the clock — avoids pulling in a rng crate.
        let jitter = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_millis() as u64)
            .unwrap_or(0))
            % 3;
        Some(Box::pin(tokio::time::sleep(std::time::Duration::from_secs(
            capped + jitter,
        ))))
    }

    fn clone_request(&mut self, req: &Request<T, Ctx>) -> Option<Request<T, Ctx>> {
        let req = req.clone();
        req.parts.attempt.increment();
        Some(req)
    }
}

/// Background reaper: periodically fails deployments stuck in a non-terminal state for
/// too long. Covers the case where a worker died mid-build (SIGKILL/OOM/crash) leaving a
/// deployment pinned at `Building`/`Deploying` with no one to ever finish it. Runs once
/// at startup (reconcile after a restart) and then on an interval.
async fn run_stuck_deployment_reaper(db: mcp_db::DbPool) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(REAPER_INTERVAL_SECS));
    loop {
        interval.tick().await;
        match DeploymentRepository::timeout_stuck_deployments(&db, STUCK_DEPLOYMENT_TIMEOUT_MINUTES)
            .await
        {
            Ok(n) if n > 0 => {
                tracing::warn!(
                    "Reaper: failed {} deployment(s) stuck >{}min in a non-terminal state",
                    n,
                    STUCK_DEPLOYMENT_TIMEOUT_MINUTES
                );
            }
            Ok(_) => {}
            Err(e) => tracing::error!("Reaper: failed to sweep stuck deployments: {}", e),
        }
    }
}

/// Background sweeper for orphaned teardowns: finds servers stuck in `deleting` (their
/// destroy job never confirmed) and re-drives the Fly app destroy. Destroying a missing
/// app is a no-op, so this is idempotent; on success it hard-deletes the row to complete
/// the soft-delete. Runs once at startup (reconcile after a restart) then on an interval.
async fn run_orphan_destroy_sweeper(ctx: Arc<BuilderContext>) {
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(ORPHAN_SWEEPER_INTERVAL_SECS));
    loop {
        interval.tick().await;
        let stuck = match ServerRepository::list_deleting_older_than(
            &ctx.db,
            DELETING_STUCK_TIMEOUT_MINUTES,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Orphan sweeper: failed to list servers stuck in deleting: {}", e);
                continue;
            }
        };
        if stuck.is_empty() {
            continue;
        }
        tracing::warn!(
            "Orphan sweeper: {} server(s) stuck in 'deleting' >{}min; re-driving teardown",
            stuck.len(),
            DELETING_STUCK_TIMEOUT_MINUTES
        );
        for server in stuck {
            match containerd::destroy_container(&server.container_name).await {
                Ok(()) => {
                    if let Err(e) = ServerRepository::delete(&ctx.db, server.id).await {
                        tracing::error!(
                            "Orphan sweeper: destroyed {} but failed to delete row {}: {}",
                            server.container_name, server.id, e
                        );
                    } else {
                        tracing::info!(
                            "Orphan sweeper: torn down {} and removed server {}",
                            server.container_name, server.id
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Orphan sweeper: destroy of {} (server {}) failed, will retry: {}",
                        server.container_name, server.id, e
                    );
                }
            }
        }
    }
}

/// Background sweeper for stale Firecracker artefacts: snapshot directories,
/// rootfs/marker files, and dead socket files whose container no longer exists
/// in the DB. Runs once at startup (to reconcile after a restart or crash) and
/// then every 10 minutes.
async fn run_stale_resource_sweeper(db: mcp_db::DbPool) {
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(ORPHAN_SWEEPER_INTERVAL_SECS));
    loop {
        interval.tick().await;
        match ServerRepository::list_all_container_names(&db).await {
            Ok(known) => {
                firecracker::cleanup_stale_resources(&known).await;
            }
            Err(e) => {
                tracing::error!("stale-resource sweeper: failed to list container names: {}", e);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // musl builds require explicit CryptoProvider selection
    let _ = rustls::crypto::ring::default_provider().install_default();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mcp_builder=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::from_env()?;
    tracing::info!("Starting MCP Cloud Builder Worker");

    let db_pool = mcp_db::create_pool(&config).await?;
    let job_queue = JobQueue::connect(&config.redis.url).await?;

    // Create crypto service for decrypting secrets
    let crypto = CryptoService::from_hex(
        &std::env::var("ENCRYPTION_KEY").expect("ENCRYPTION_KEY must be set"),
    )
    .expect("Invalid encryption key");

    // Create GitHub App client (optional - may not have valid credentials in dev)
    let github = GitHubApp::new(&config).ok();
    if github.is_some() {
        tracing::info!("GitHub App initialized");
    } else {
        tracing::warn!("GitHub App not configured - will use public repos only");
    }

    // Create event publisher for real-time WebSocket updates
    let events = EventPublisher::new(&config.redis.url);

    // Create email service (optional)
    let email = match EmailService::from_env() {
        Ok(service) => {
            tracing::info!("Resend email service initialized");
            Some(service)
        }
        Err(e) => {
            tracing::warn!("Email service not configured: {} - email notifications disabled", e);
            None
        }
    };

    let context = Arc::new(BuilderContext {
        config: config.clone(),
        db: db_pool,
        job_queue,
        crypto,
        github,
        events,
        email,
    });

    // Connect to Redis for job queue
    let redis_url = &config.redis.url;
    let redis_client = redis::Client::open(redis_url.as_str())?;
    let redis_conn = redis::aio::ConnectionManager::new(redis_client).await?;

    // Set poll interval to 30 seconds to reduce Redis commands
    // (default is too frequent and consumes Upstash free tier quickly)
    // Use explicit namespaces to match the API queue configuration
    let build_config = apalis_redis::Config::default()
        .set_namespace("build_jobs")
        .set_poll_interval(std::time::Duration::from_secs(30));
    let deploy_config = apalis_redis::Config::default()
        .set_namespace("deploy_jobs")
        .set_poll_interval(std::time::Duration::from_secs(30));
    let destroy_config = apalis_redis::Config::default()
        .set_namespace("destroy_jobs")
        .set_poll_interval(std::time::Duration::from_secs(30));

    // Log the key names that apalis-redis will use
    tracing::info!("Build queue config: namespace={:?}", build_config.get_namespace());
    tracing::info!("  active_jobs_list: {}", build_config.active_jobs_list());
    tracing::info!("  job_data_hash: {}", build_config.job_data_hash());

    let storage = RedisStorage::<BuildJob>::new_with_config(
        redis_conn.clone(),
        build_config,
    );
    let deploy_storage = RedisStorage::<DeployJob>::new_with_config(
        redis_conn.clone(),
        deploy_config,
    );
    let destroy_storage = RedisStorage::<DestroyJob>::new_with_config(
        redis_conn,
        destroy_config,
    );

    tracing::info!("Connected to job queue, created BuildJob, DeployJob and DestroyJob storage instances");

    // Create workers.
    // - `.concurrency(n)` lets each worker process several jobs at once so one slow
    //   build no longer stalls the entire queue.
    // - `.retry(policy)` makes a handler's `Err` actually re-attempt with bounded
    //   exponential backoff (apalis treats a bare `Err` as terminal otherwise).
    let retry_policy = BoundedBackoffRetry { max_retries: MAX_JOB_RETRIES };
    let concurrency = worker_concurrency();

    let build_worker = WorkerBuilder::new("build-worker")
        .concurrency(concurrency)
        .retry(retry_policy.clone())
        .data(context.clone())
        .backend(storage)
        .build_fn(handle_build_job);

    let deploy_worker = WorkerBuilder::new("deploy-worker")
        .concurrency(concurrency)
        .retry(retry_policy.clone())
        .data(context.clone())
        .backend(deploy_storage)
        .build_fn(handle_deploy_job);

    let destroy_worker = WorkerBuilder::new("destroy-worker")
        .concurrency(concurrency)
        .retry(retry_policy.clone())
        .data(context.clone())
        .backend(destroy_storage)
        .build_fn(handle_destroy_job);

    // HTTP server: health + internal API (exec / stats / metrics from API)
    let http_port = std::env::var("BUILDER_HTTP_PORT").unwrap_or_else(|_| "8083".to_string());
    // Bind to loopback only — the internal API must never be reachable from external IPs.
    // Network-level firewall is a second layer; this is the primary defense.
    let http_addr = format!("127.0.0.1:{}", http_port);
    let builder_token = std::env::var("BUILDER_TOKEN")
        .expect("BUILDER_TOKEN must be set");
    if builder_token.is_empty() {
        panic!("BUILDER_TOKEN must not be empty — set a random secret of at least 32 characters");
    }

    let http_router = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/", get(|| async { "NodeFlare Builder" }))
        .route("/internal/exec/:container", post({
            let tok = builder_token.clone();
            move |headers: axum::http::HeaderMap, _path: axum::extract::Path<String>, _body: axum::Json<serde_json::Value>| {
                let tok = tok.clone();
                async move {
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
                    if !bearer_token_eq(auth, &tok) {
                        return (axum::http::StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "unauthorized"})));
                    }
                    // Firecracker VMs are not containers: exec requires a guest agent
                    // (e.g. vsock-based) which is not yet implemented.
                    (axum::http::StatusCode::NOT_IMPLEMENTED, axum::Json(serde_json::json!({"error": "exec is not supported for Firecracker VMs"})))
                }
            }
        }))
        .route("/internal/stats", get({
            let tok = builder_token.clone();
            move |headers: axum::http::HeaderMap| {
                let tok = tok.clone();
                async move {
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
                    if !bearer_token_eq(auth, &tok) {
                        return (axum::http::StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "unauthorized"})));
                    }
                    // List running Firecracker VMs by active socket files + live process check
                    let vms = firecracker::list_running_vms().await;
                    (axum::http::StatusCode::OK, axum::Json(serde_json::json!({"vms": vms, "count": vms.len()})))
                }
            }
        }))
        .route("/internal/metrics/:container", get({
            let tok = builder_token.clone();
            move |headers: axum::http::HeaderMap, path: axum::extract::Path<String>| {
                let tok = tok.clone();
                async move {
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
                    if !bearer_token_eq(auth, &tok) {
                        return (axum::http::StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "unauthorized"})));
                    }
                    let container = path.0;
                    if !is_valid_container_name(&container) {
                        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"error": "invalid container name"})));
                    }
                    // Read VM RSS from the Firecracker process's /proc entry
                    match firecracker::vm_rss_bytes(&container).await {
                        Some(rss_bytes) => {
                            let rss_mb = rss_bytes / (1024 * 1024);
                            (axum::http::StatusCode::OK, axum::Json(serde_json::json!({
                                "container": container,
                                "mem_rss_bytes": rss_bytes,
                                "mem_rss_mb": rss_mb,
                            })))
                        }
                        None => {
                            (axum::http::StatusCode::NOT_FOUND, axum::Json(serde_json::json!({
                                "error": "VM not running or metrics unavailable"
                            })))
                        }
                    }
                }
            }
        }))
        .route("/internal/start/:container", post({
            let tok = builder_token.clone();
            let db = context.db.clone();
            let crypto = context.crypto.clone();
            move |headers: axum::http::HeaderMap, path: axum::extract::Path<String>| {
                let tok = tok.clone();
                let db = db.clone();
                let crypto = crypto.clone();
                async move {
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
                    if !bearer_token_eq(auth, &tok) {
                        return (axum::http::StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "unauthorized"})));
                    }
                    let container = path.0;
                    if !is_valid_container_name(&container) {
                        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"error": "invalid container name"})));
                    }
                    match firecracker::restore(&container).await {
                        Ok(_port) => {
                            tracing::info!("scale-to-zero: restored VM {} from snapshot", container);
                            if let Err(e) = ServerRepository::update_status_by_container_name(
                                &db,
                                &container,
                                mcp_common::types::ServerStatus::Running,
                            ).await {
                                tracing::warn!("scale-to-zero: failed to update status for {}: {}", container, e);
                            }
                            (axum::http::StatusCode::OK, axum::Json(serde_json::json!({"started": container})))
                        }
                        Err(restore_err) => {
                            tracing::warn!("scale-to-zero: restore of {} failed ({}), attempting fresh boot", container, restore_err);
                            // Read image name from marker file (written by extract_rootfs)
                            let marker_path = format!("/opt/firecracker/rootfs/{}.marker", container);
                            let image = match tokio::fs::read_to_string(&marker_path).await {
                                Ok(content) => content.lines().next().unwrap_or("").trim().to_string(),
                                Err(e) => {
                                    tracing::warn!("scale-to-zero: no marker for {}: {}", container, e);
                                    return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": format!("no snapshot and no marker for {}", container)})));
                                }
                            };
                            if image.is_empty() {
                                return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": "empty image name in marker"})));
                            }
                            let server = match ServerRepository::find_by_container_name(&db, &container).await {
                                Ok(Some(s)) => s,
                                Ok(None) => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": "server not found"}))),
                                Err(e) => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": e.to_string()}))),
                            };
                            let encrypted_secrets = SecretRepository::list_by_server(&db, server.id).await.unwrap_or_default();
                            let host_port = containerd::derive_host_port(&container);
                            let mut env_pairs: Vec<(String, String)> = vec![("PORT".to_string(), host_port.to_string())];
                            for secret in encrypted_secrets {
                                if let Ok(bytes) = crypto.decrypt(&secret.encrypted_value, &secret.nonce) {
                                    if let Ok(value) = String::from_utf8(bytes) {
                                        env_pairs.push((secret.key, value));
                                    }
                                }
                            }
                            let memory_mb = server.memory_mb.unwrap_or(256) as u64;
                            match firecracker::boot(&container, &image, memory_mb, &env_pairs).await {
                                Ok(_) => {
                                    tracing::info!("scale-to-zero: fresh-booted VM {} from image {}", container, image);
                                    if let Err(e) = ServerRepository::update_status_by_container_name(
                                        &db,
                                        &container,
                                        mcp_common::types::ServerStatus::Running,
                                    ).await {
                                        tracing::warn!("scale-to-zero: failed to update status for {}: {}", container, e);
                                    }
                                    (axum::http::StatusCode::OK, axum::Json(serde_json::json!({"started": container})))
                                }
                                Err(e) => {
                                    tracing::warn!("scale-to-zero: fresh boot of {} failed: {}", container, e);
                                    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": e.to_string()})))
                                }
                            }
                        }
                    }
                }
            }
        }))
        .route("/internal/fork/:container", post({
            let tok = builder_token.clone();
            move |headers: axum::http::HeaderMap, path: axum::extract::Path<String>| {
                let tok = tok.clone();
                async move {
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
                    if !bearer_token_eq(auth, &tok) {
                        return (axum::http::StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "unauthorized"})));
                    }
                    let base = path.0;
                    if !is_valid_container_name(&base) {
                        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"error": "invalid container name"})));
                    }
                    // Unique fork name: millisecond timestamp + atomic counter avoids
                    // collisions under concurrent same-millisecond requests.
                    let ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    let seq = FORK_SEQ.fetch_add(1, Ordering::Relaxed);
                    let prefix = &base[..base.len().min(20)];
                    let fork = format!("{}-f{}{:06}", prefix, ms, seq % 1_000_000);
                    match firecracker::restore_fork(&base, &fork).await {
                        Ok(port) => (axum::http::StatusCode::OK, axum::Json(serde_json::json!({
                            "fork_container": fork,
                            "base_container": base,
                            "port": port,
                        }))),
                        Err(e) => {
                            tracing::warn!("fork of {} failed: {}", base, e);
                            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": e.to_string()})))
                        }
                    }
                }
            }
        }))
        .route("/internal/kill-fork/:base/:fork", post({
            let tok = builder_token.clone();
            move |headers: axum::http::HeaderMap, path: axum::extract::Path<(String, String)>| {
                let tok = tok.clone();
                async move {
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
                    if !bearer_token_eq(auth, &tok) {
                        return (axum::http::StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "unauthorized"})));
                    }
                    let (base, fork) = path.0;
                    if !is_valid_container_name(&base) || !is_valid_container_name(&fork) {
                        return (axum::http::StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"error": "invalid container name"})));
                    }
                    firecracker::kill_fork(&fork, &base).await;
                    (axum::http::StatusCode::OK, axum::Json(serde_json::json!({"killed": fork})))
                }
            }
        }));

    let http_listener = TcpListener::bind(&http_addr).await?;
    tracing::info!("Builder HTTP server listening on {}", http_addr);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(http_listener, http_router).await {
            tracing::error!("Builder HTTP server error: {}", e);
        }
    });

    // Shared active-request counter per base container.
    // The fork socket server increments it on each lease (fork or hold);
    // the scale-to-zero scaler checks it before snapshotting.
    let active_map = fork_socket::new_active_map();

    // Scale-to-zero scaler: stop idle containers every 5 minutes.
    // Passes the active_map so it can skip VMs with in-flight requests.
    tokio::spawn(run_scale_to_zero_scaler(
        context.db.clone(),
        config.redis.url.clone(),
        active_map.clone(),
    ));

    // Reconcile-on-startup + periodic reaper for deployments stuck in a non-terminal
    // state (worker died mid-build, OOM, etc.) so they don't show "Building" forever.
    tokio::spawn(run_stuck_deployment_reaper(context.db.clone()));

    // Reconcile-on-startup + periodic sweeper for servers stuck in `deleting` whose Fly
    // app teardown never confirmed (worker crash, dropped job, exhausted retries) so we
    // don't leak orphaned, billable apps.
    tokio::spawn(run_orphan_destroy_sweeper(context.clone()));

    // Stale Firecracker resource sweeper: removes snapshot dirs, rootfs files, and
    // dead socket files that belong to containers no longer in the DB.
    tokio::spawn(run_stale_resource_sweeper(context.db.clone()));

    // Fork VM Unix socket server: manages fork VM lifecycle and base VM active leases.
    // Connections held open protect VMs from scale-to-zero; dropping triggers cleanup.
    if let Ok(socket_path) = std::env::var("FORK_SOCKET_PATH") {
        let path = socket_path.clone();
        let am = active_map.clone();
        tokio::spawn(async move {
            fork_socket::run(&path, am).await;
        });
        tracing::info!("Fork socket server will start on {}", socket_path);
    }

    tracing::info!("Starting job queue workers (poll interval: 30s)...");

    // Spawn a heartbeat task that verifies Redis connectivity
    // If Redis connection fails, exit the process so Fly.io restarts it
    let redis_heartbeat_url = config.redis.url.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        let mut consecutive_failures = 0;
        const MAX_FAILURES: u32 = 3;
        // Scalability: Configurable timeout for Redis operations
        let redis_timeout_secs: u64 = std::env::var("REDIS_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15);

        loop {
            interval.tick().await;

            // Try to ping Redis with timeout
            let ping_result = tokio::time::timeout(
                std::time::Duration::from_secs(redis_timeout_secs),
                async {
                    let client = redis::Client::open(redis_heartbeat_url.as_str())?;
                    let mut conn = client.get_multiplexed_async_connection().await?;
                    redis::cmd("PING").query_async::<String>(&mut conn).await
                }
            ).await;

            match ping_result {
                Ok(Ok(_)) => {
                    consecutive_failures = 0;
                    tracing::info!("Builder heartbeat - Redis connection OK");
                }
                Ok(Err(e)) => {
                    consecutive_failures += 1;
                    tracing::error!("Redis PING failed: {} ({}/{})", e, consecutive_failures, MAX_FAILURES);
                }
                Err(_) => {
                    consecutive_failures += 1;
                    tracing::error!("Redis PING timed out after {}s ({}/{})", redis_timeout_secs, consecutive_failures, MAX_FAILURES);
                }
            }

            if consecutive_failures >= MAX_FAILURES {
                tracing::error!("Redis connection failed {} times consecutively, exiting for restart", MAX_FAILURES);
                std::process::exit(1);
            }
        }
    });

    // Run workers - this blocks until shutdown is signalled, polling for jobs.
    let monitor = Monitor::new()
        .register(build_worker)
        .register(deploy_worker)
        .register(destroy_worker)
        // On SIGTERM/Ctrl-C, let in-flight jobs drain (up to this bound) before exiting,
        // so a deploy in progress isn't left half-applied.
        .shutdown_timeout(std::time::Duration::from_secs(30));

    tracing::info!("Monitor created with 3 workers, calling run()...");

    // Graceful shutdown: SIGTERM (Fly.io machine stop / deploy) or Ctrl-C triggers a
    // drain rather than an abrupt kill.
    let shutdown_signal = async {
        let sigterm = async {
            #[cfg(unix)]
            {
                let mut s = tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::terminate(),
                )?;
                s.recv().await;
                Ok::<(), std::io::Error>(())
            }
            #[cfg(not(unix))]
            {
                futures::future::pending::<()>().await;
                Ok::<(), std::io::Error>(())
            }
        };
        tokio::select! {
            r = tokio::signal::ctrl_c() => r,
            r = sigterm => r,
        }
    };

    // Run the monitor until a shutdown signal arrives, then drain.
    match monitor.run_with_signal(shutdown_signal).await {
        Ok(()) => {
            tracing::info!("Monitor.run() completed successfully");
        }
        Err(e) => {
            tracing::error!("Monitor.run() failed with error: {:?}", e);
        }
    }

    tracing::warn!("Monitor exited - this should not happen in normal operation!");
    Ok(())
}

async fn handle_build_job(mut job: BuildJob, ctx: Data<Arc<BuilderContext>>) -> Result<(), Error> {
    tracing::info!("Processing build job: {:?}", job.deployment_id);

    // Update deployment status to building
    DeploymentRepository::update(
        &ctx.db,
        job.deployment_id,
        UpdateDeployment {
            status: Some(mcp_common::types::DeploymentStatus::Building),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| Error::Failed(Arc::new(e.into())))?;

    // Publish building status via WebSocket
    ctx.events
        .publish_deployment_status(
            job.deployment_id,
            job.server_id,
            mcp_common::types::DeploymentStatus::Building,
            None,
            Some(10),
        )
        .await
        .ok();

    // Parse owner/repo from github_repo
    let parts: Vec<&str> = job.github_repo.split('/').collect();
    if parts.len() != 2 {
        let err_msg = format!("Invalid github_repo format: {}", job.github_repo);
        DeploymentRepository::update(
            &ctx.db,
            job.deployment_id,
            UpdateDeployment {
                status: Some(mcp_common::types::DeploymentStatus::Failed),
                error_message: Some(err_msg.clone()),
                finished_at: Some(chrono::Utc::now()),
                ..Default::default()
            },
        )
        .await
        .ok();
        // Terminal: the deployment was already recorded as `Failed` above. Return Ok so
        // the RetryLayer does NOT re-run a job that can only fail the same way (the Err
        // contract is reserved for transient failures that were NOT recorded).
        return Ok(());
    }
    let (owner, repo) = (parts[0], parts[1]);

    // Create temp directory for source code
    let temp_dir = tempfile::tempdir()
        .map_err(|e| Error::Failed(Arc::new(e.into())))?;
    let source_dir = temp_dir.path();

    // Helper to log and publish to WebSocket
    // Takes ownership of String to avoid double allocation
    let log_to_db_and_ws = |ctx: &BuilderContext, deployment_id: uuid::Uuid, msg: String| {
        let db = ctx.db.clone();
        let events = ctx.events.clone();
        async move {
            DeploymentRepository::append_log(&db, deployment_id, &msg).await.ok();
            events.publish_build_log(deployment_id, &msg, LogStream::Stdout).await.ok();
        }
    };

    // Download source code
    let download_result: Result<(), anyhow::Error> = async {
        if let (Some(github), Some(installation_id)) = (&ctx.github, job.github_installation_id) {
            log_to_db_and_ws(&ctx, job.deployment_id, "Downloading source from GitHub...".to_string()).await;

            match github.download_tarball(installation_id, owner, repo, &job.github_branch).await {
                Ok(tarball) => {
                    log_to_db_and_ws(&ctx, job.deployment_id, format!("Downloaded {} bytes, extracting...", tarball.len())).await;
                    // Extract tarball to source_dir
                    extract_tarball(&tarball, source_dir)?;
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!("GitHub App download failed, falling back to git clone: {}", e);
                    log_to_db_and_ws(&ctx, job.deployment_id, "Falling back to git clone...".to_string()).await;
                    clone_repo(&job.github_repo, &job.github_branch, source_dir).await
                }
            }
        } else {
            log_to_db_and_ws(&ctx, job.deployment_id, "Cloning public repository...".to_string()).await;
            clone_repo(&job.github_repo, &job.github_branch, source_dir).await
        }
    }.await;

    if let Err(e) = download_result {
        let err_msg = format!("Failed to download source: {}", e);
        handle_build_failure(&ctx, &job, &err_msg).await;
        // Terminal (recorded Failed): a bad repo/branch/permissions won't fix itself on
        // retry; return Ok so we don't re-run an already-failed deployment.
        return Ok(());
    }

    // Determine the actual source directory (respecting root_directory setting)
    let actual_source_dir = if job.root_directory.is_empty() || job.root_directory == "." || job.root_directory == "/" {
        source_dir.to_path_buf()
    } else {
        // SECURITY: Validate root_directory to prevent path traversal attacks
        // Block directory traversal sequences
        if job.root_directory.contains("..") {
            let err_msg = "Root directory cannot contain '..' (path traversal not allowed)";
            handle_build_failure(&ctx, &job, err_msg).await;
            // Terminal: the deployment was already recorded as `Failed` above. Return Ok so
        // the RetryLayer does NOT re-run a job that can only fail the same way (the Err
        // contract is reserved for transient failures that were NOT recorded).
        return Ok(());
        }

        // Strip leading slash if present
        let clean_root_dir = job.root_directory.trim_start_matches('/');

        // Block absolute paths and other dangerous patterns
        if clean_root_dir.starts_with('/') || clean_root_dir.contains('\0') {
            let err_msg = "Invalid root directory path";
            handle_build_failure(&ctx, &job, err_msg).await;
            // Terminal: the deployment was already recorded as `Failed` above. Return Ok so
        // the RetryLayer does NOT re-run a job that can only fail the same way (the Err
        // contract is reserved for transient failures that were NOT recorded).
        return Ok(());
        }

        let subdir = source_dir.join(clean_root_dir);

        // Verify the subdirectory exists
        if !subdir.exists() {
            let err_msg = format!("Root directory '{}' not found in repository", job.root_directory);
            handle_build_failure(&ctx, &job, &err_msg).await;
            // Terminal: the deployment was already recorded as `Failed` above. Return Ok so
        // the RetryLayer does NOT re-run a job that can only fail the same way (the Err
        // contract is reserved for transient failures that were NOT recorded).
        return Ok(());
        }

        // SECURITY: Canonicalize and verify the path is within source_dir
        let canonical_subdir = match subdir.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                let err_msg = format!("Failed to resolve root directory: {}", e);
                handle_build_failure(&ctx, &job, &err_msg).await;
                // Terminal: the deployment was already recorded as `Failed` above. Return Ok so
        // the RetryLayer does NOT re-run a job that can only fail the same way (the Err
        // contract is reserved for transient failures that were NOT recorded).
        return Ok(());
            }
        };

        let canonical_source = match source_dir.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                let err_msg = format!("Failed to resolve source directory: {}", e);
                handle_build_failure(&ctx, &job, &err_msg).await;
                // Terminal: the deployment was already recorded as `Failed` above. Return Ok so
        // the RetryLayer does NOT re-run a job that can only fail the same way (the Err
        // contract is reserved for transient failures that were NOT recorded).
        return Ok(());
            }
        };

        // Ensure the resolved path is still within the source directory
        if !canonical_subdir.starts_with(&canonical_source) {
            let err_msg = "Root directory path escapes repository boundary (path traversal blocked)";
            handle_build_failure(&ctx, &job, err_msg).await;
            // Terminal: the deployment was already recorded as `Failed` above. Return Ok so
        // the RetryLayer does NOT re-run a job that can only fail the same way (the Err
        // contract is reserved for transient failures that were NOT recorded).
        return Ok(());
        }

        log_to_db_and_ws(&ctx, job.deployment_id, format!("Using root directory: {}", job.root_directory)).await;
        canonical_subdir
    };

    // Strategy 3: a member of a workspaces monorepo cannot be built from its own
    // subdirectory — tsconfig typically extends the repo root and workspace dependencies
    // (internal packages like mcp-utils) are only resolvable from the root.
    // Always build from the repo ROOT for workspace members, then:
    //   a) auto-detect the entry command when the user hasn't set one, and
    //   b) for pnpm workspaces without a user-specified build command, auto-generate
    //      `pnpm --filter {./subdir}... run build --if-present`.
    //      The trailing `...` is pnpm's "includeDependencies" selector — it runs the
    //      member AND all workspace packages it depends on, in topological order
    //      (dependencies compiled before their dependents). Leading `...` would be
    //      "includeDependents" (the reverse direction) and is incorrect here.
    let mut build_context = actual_source_dir.clone();
    if !job.root_directory.is_empty()
        && job.root_directory != "."
        && job.root_directory != "/"
    {
        let clean_root_dir = job.root_directory.trim_start_matches('/').to_string();
        let workspaces = flyctl::detect_workspace_globs(source_dir);
        if flyctl::subdir_is_workspace_member(&clean_root_dir, &workspaces) {
            // Always use the monorepo root as the build context so workspace deps are
            // available — this holds regardless of whether the user set an entry command.
            build_context = source_dir.to_path_buf();

            // Auto-detect entry command only when the user hasn't set one.
            if job.entry_command.is_none() {
                if let Some(entry) =
                    flyctl::detect_member_entry(source_dir, &actual_source_dir, &job.runtime).await
                {
                    let prefixed = flyctl::prefix_entry_with_subdir(&entry, &clean_root_dir);
                    log_to_db_and_ws(
                        &ctx,
                        job.deployment_id,
                        format!(
                            "Workspace monorepo member detected — building from repo root with entry `{}`",
                            prefixed
                        ),
                    )
                    .await;
                    job.entry_command = Some(prefixed);
                }
            }

            // For pnpm workspaces without a user-specified build command, auto-generate
            // a command that builds the target member AND its workspace dependencies.
            if job.build_command.is_none()
                && (source_dir.join("pnpm-workspace.yaml").exists()
                    || source_dir.join("pnpm-workspace.yml").exists())
            {
                let cmd = format!(
                    "pnpm --filter {{./{clean_root_dir}}}... run --if-present build"
                );
                log_to_db_and_ws(
                    &ctx,
                    job.deployment_id,
                    format!("pnpm workspace member — auto-generated build command: {cmd}"),
                )
                .await;
                job.build_command = Some(cmd);
            }
        }
    }

    // Get secrets for this server and decrypt them
    let encrypted_secrets = SecretRepository::list_by_server(&ctx.db, job.server_id)
        .await
        .unwrap_or_default();

    let mut secrets: Vec<mcp_queue::SecretEnv> = encrypted_secrets
        .into_iter()
        .filter_map(|secret| {
            ctx.crypto
                .decrypt(&secret.encrypted_value, &secret.nonce)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .map(|value| mcp_queue::SecretEnv {
                    key: secret.key,
                    value,
                })
        })
        .collect();

    // If the server requires upstream OAuth, tell the adapter which provider to use,
    // and generate a per-server identity token so the MCP server can authenticate
    // against NodeFlare's internal API to fetch OAuth tokens.
    if job.upstream_oauth_provider.is_some() {
        let server_token = uuid::Uuid::new_v4().to_string();
        match ctx.crypto.encrypt_string(&server_token) {
            Ok((encrypted_value, nonce)) => {
                if let Err(e) = SecretRepository::upsert(&ctx.db, CreateSecret {
                    server_id: job.server_id,
                    key: "_NODEFLARE_SERVER_TOKEN".to_string(),
                    encrypted_value,
                    nonce,
                }).await {
                    tracing::warn!("Failed to persist NODEFLARE_SERVER_TOKEN for server {}: {}", job.server_id, e);
                }
            }
            Err(e) => tracing::warn!("Failed to encrypt NODEFLARE_SERVER_TOKEN: {}", e),
        }
        secrets.push(mcp_queue::SecretEnv {
            key: "NODEFLARE_SERVER_TOKEN".to_string(),
            value: server_token,
        });
        let api_internal_url = std::env::var("NODEFLARE_API_INTERNAL_URL")
            .unwrap_or_else(|_| "http://nodeflare-api.internal:8080".to_string());
        secrets.push(mcp_queue::SecretEnv {
            key: "NODEFLARE_API_INTERNAL_URL".to_string(),
            value: api_internal_url,
        });
        secrets.push(mcp_queue::SecretEnv {
            key: "SERVER_ID".to_string(),
            value: job.server_id.to_string(),
        });
    }
    if let Some(ref provider) = job.upstream_oauth_provider {
        secrets.push(mcp_queue::SecretEnv {
            key: "OAUTH_PROVIDER".to_string(),
            value: provider.clone(),
        });
    }

    // Update to deploying status
    DeploymentRepository::update(
        &ctx.db,
        job.deployment_id,
        UpdateDeployment {
            status: Some(mcp_common::types::DeploymentStatus::Deploying),
            ..Default::default()
        },
    )
    .await
    .ok();

    ctx.events
        .publish_deployment_status(
            job.deployment_id,
            job.server_id,
            mcp_common::types::DeploymentStatus::Deploying,
            None,
            Some(50),
        )
        .await
        .ok();

    // Build and deploy using flyctl (remote builder).
    //
    // Build logs are funnelled through a single mpsc channel drained by one ordered
    // consumer task that batches appends into one DB write per batch. The previous
    // fire-and-forget `tokio::spawn`-per-line had no ordering guarantee (lines could be
    // persisted out of order) and did one DB round-trip per line.
    let deployment_id = job.deployment_id;
    let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let log_consumer = {
        let db = ctx.db.clone();
        let events = ctx.events.clone();
        tokio::spawn(async move {
            // Drain in order; coalesce whatever is already queued into one append.
            while let Some(first) = log_rx.recv().await {
                let mut batch = vec![first];
                while let Ok(next) = log_rx.try_recv() {
                    batch.push(next);
                    if batch.len() >= 50 {
                        break;
                    }
                }
                // Publish to the live WS stream in order.
                for line in &batch {
                    events
                        .publish_build_log(deployment_id, line, LogStream::Stdout)
                        .await
                        .ok();
                }
                // One batched, ordered DB append for the whole chunk.
                let joined = batch.join("\n");
                DeploymentRepository::append_log(&db, deployment_id, &joined)
                    .await
                    .ok();
            }
        })
    };

    let on_log = move |msg: &str| {
        // Non-blocking, ordered hand-off to the consumer task.
        let _ = log_tx.send(msg.to_string());
    };

    let plan_memory_ceiling = plan_memory_ceiling_mb(&ctx.db, job.server_id).await;

    // Judgment D: a built image and a started machine don't prove the MCP server
    // runs — a wrong entry path leaves the adapter listening while the child process
    // crash-loops, so the deploy looks successful but every request 500s. Probe a
    // real MCP initialize and demote a proven-broken server to a failure so it goes
    // through normal failure reporting instead of being shown as "succeeded".
    //
    // The whole build+deploy+verify cycle is bounded by a hard timeout so a hung remote
    // build or a never-resolving probe can't pin the worker or leave the deployment
    // stuck in `Deploying` forever (the startup reaper is the backstop for crashes).
    let build_result = match tokio::time::timeout(
        std::time::Duration::from_secs(BUILD_DEPLOY_TIMEOUT_SECS),
        async {
            let deploy = containerd::build_and_run(
                &ctx.config,
                &job,
                &build_context,
                &secrets,
                plan_memory_ceiling,
                &on_log,
            )
            .await?;

            match flyctl::verify_mcp_initialize(&deploy.endpoint_url, &job.mcp_path, &on_log).await {
                flyctl::ProbeOutcome::Verified => Ok(deploy),
                flyctl::ProbeOutcome::Inconclusive(detail) => {
                    on_log(&format!(
                        "Warning: could not verify the MCP server responded; leaving as deployed. ({})",
                        detail
                    ));
                    Ok(deploy)
                }
                flyctl::ProbeOutcome::Broken(detail) => {
                    let server_logs = containerd::fetch_container_logs(&deploy.container_name)
                        .await
                        .map(|l| flyctl::extract_error_lines(&l, 12))
                        .filter(|s| !s.is_empty());
                    let message = match server_logs {
                        Some(logs) => format!(
                            "Container started but the MCP server did not respond. \
                            This is usually a wrong startup command or missing build output.\n\n\
                            Server error:\n{}",
                            logs
                        ),
                        None => format!(
                            "Container started but the MCP server did not respond \
                            ({}). This is usually a wrong startup command or missing build output.",
                            detail
                        ),
                    };
                    Err(anyhow::anyhow!(message))
                }
            }
        },
    )
    .await
    {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!(
            "Build and deploy exceeded the {}-minute limit and was aborted. \
            This usually means the build is hanging (e.g. an interactive prompt or an \
            extremely large image).",
            BUILD_DEPLOY_TIMEOUT_SECS / 60
        )),
    };

    // Stop the logger and flush any buffered lines before we write the final status.
    drop(on_log);
    log_consumer.await.ok();

    match build_result {
        Ok(deploy_result) => {
            // Deployment successful
            DeploymentRepository::update(
                &ctx.db,
                job.deployment_id,
                UpdateDeployment {
                    status: Some(mcp_common::types::DeploymentStatus::Succeeded),
                    finished_at: Some(chrono::Utc::now()),
                    ..Default::default()
                },
            )
            .await
            .ok();

            ctx.events
                .publish_deployment_status(
                    job.deployment_id,
                    job.server_id,
                    mcp_common::types::DeploymentStatus::Succeeded,
                    None,
                    Some(100),
                )
                .await
                .ok();

            if let Err(e) = ServerRepository::update_status(
                &ctx.db,
                job.server_id,
                mcp_common::types::ServerStatus::Running,
                Some(&deploy_result.endpoint_url),
            )
            .await
            {
                tracing::error!("Failed to update server status to Running: {}", e);
            }
            ctx.events.publish_server_status(
                job.server_id,
                mcp_common::types::ServerStatus::Running,
                Some(deploy_result.endpoint_url.clone()),
                None,
            ).await.ok();

            // Update region status; store container_name as the machine_id
            if let Err(e) = ServerRegionRepository::update(
                &ctx.db,
                job.server_id,
                &job.region,
                UpdateServerRegion {
                    status: Some(RegionStatus::Running),
                    endpoint_url: Some(deploy_result.endpoint_url),
                    machine_id: Some(deploy_result.container_name),
                },
            )
            .await
            {
                tracing::error!("Failed to update region status: {}", e);
            }

            tracing::info!("Deployment {} succeeded", job.deployment_id);

            // Send success email notification
            send_deploy_notification(&ctx, job.server_id, true, None).await;
        }
        Err(e) => {
            tracing::error!("Build failed: {:#}", e);
            let error_msg = e.to_string();

            // Get user's locale preference and analyze error with localized hints
            let locale = get_user_locale_for_server(&ctx.db, job.server_id).await;
            let hint = analyze_error_for_hints(&ctx.db, &error_msg, &locale).await;
            let full_error_msg = if let Some(ref hint) = hint {
                format!("{}{}", error_msg, hint)
            } else {
                error_msg.clone()
            };

            DeploymentRepository::update(
                &ctx.db,
                job.deployment_id,
                UpdateDeployment {
                    status: Some(mcp_common::types::DeploymentStatus::Failed),
                    error_message: Some(full_error_msg.clone()),
                    finished_at: Some(chrono::Utc::now()),
                    ..Default::default()
                },
            )
            .await
            .ok();

            // Publish failed status
            ctx.events
                .publish_deployment_status(
                    job.deployment_id,
                    job.server_id,
                    mcp_common::types::DeploymentStatus::Failed,
                    Some(full_error_msg.clone()),
                    Some(100),
                )
                .await
                .ok();
            ctx.events.publish_build_log(job.deployment_id, &error_msg, LogStream::Stderr).await.ok();

            // Log hint separately if available
            if let Some(hint) = hint {
                ctx.events
                    .publish_build_log(job.deployment_id, &hint, LogStream::Stderr)
                    .await
                    .ok();
                DeploymentRepository::append_log(&ctx.db, job.deployment_id, &hint)
                    .await
                    .ok();
            }

            // Only set server to failed if there's no newer successful deployment
            // This prevents race conditions where an old failed deployment (due to retries)
            // overwrites a newer successful deployment's status
            let current_version = DeploymentRepository::find_by_id(&ctx.db, job.deployment_id)
                .await
                .ok()
                .flatten()
                .map(|d| d.version)
                .unwrap_or(0);

            let has_newer_success = DeploymentRepository::has_succeeded_deployment_after(
                &ctx.db,
                job.server_id,
                current_version,
            )
            .await
            .unwrap_or(false);

            if has_newer_success {
                tracing::info!(
                    "Skipping server status update to Failed for deployment {} (version {}) - newer successful deployment exists",
                    job.deployment_id,
                    current_version
                );
            } else {
                if let Err(e) = ServerRepository::update_status(
                    &ctx.db,
                    job.server_id,
                    mcp_common::types::ServerStatus::Failed,
                    None,
                )
                .await
                {
                    tracing::error!("Failed to update server status to Failed: {}", e);
                }
                ctx.events.publish_server_status(
                    job.server_id,
                    mcp_common::types::ServerStatus::Failed,
                    None,
                    Some(full_error_msg.clone()),
                ).await.ok();
            }

            // Send failure email notification
            send_deploy_notification(&ctx, job.server_id, false, Some(&full_error_msg)).await;
        }
    }

    Ok(())
}

/// Helper to handle build failure
async fn handle_build_failure(ctx: &BuilderContext, job: &BuildJob, error_msg: &str) {
    // Get user's locale preference and analyze error with localized hints
    let locale = get_user_locale_for_server(&ctx.db, job.server_id).await;
    let hint = analyze_error_for_hints(&ctx.db, error_msg, &locale).await;
    let full_error_msg = if let Some(hint) = &hint {
        format!("{}{}", error_msg, hint)
    } else {
        error_msg.to_string()
    };

    DeploymentRepository::update(
        &ctx.db,
        job.deployment_id,
        UpdateDeployment {
            status: Some(mcp_common::types::DeploymentStatus::Failed),
            error_message: Some(full_error_msg.clone()),
            finished_at: Some(chrono::Utc::now()),
            ..Default::default()
        },
    )
    .await
    .ok();

    ctx.events
        .publish_deployment_status(
            job.deployment_id,
            job.server_id,
            mcp_common::types::DeploymentStatus::Failed,
            Some(full_error_msg.clone()),
            Some(100),
        )
        .await
        .ok();

    // Log error message
    ctx.events
        .publish_build_log(job.deployment_id, error_msg, LogStream::Stderr)
        .await
        .ok();

    // Log hint separately if available
    if let Some(hint) = hint {
        ctx.events
            .publish_build_log(job.deployment_id, &hint, LogStream::Stderr)
            .await
            .ok();

        // Also append hint to deployment log
        DeploymentRepository::append_log(&ctx.db, job.deployment_id, &hint)
            .await
            .ok();
    }

    // Only set server to failed if there's no newer successful deployment
    // This prevents race conditions where an old failed deployment overwrites a newer success
    let current_version = DeploymentRepository::find_by_id(&ctx.db, job.deployment_id)
        .await
        .ok()
        .flatten()
        .map(|d| d.version)
        .unwrap_or(0);

    let has_newer_success = DeploymentRepository::has_succeeded_deployment_after(
        &ctx.db,
        job.server_id,
        current_version,
    )
    .await
    .unwrap_or(false);

    if !has_newer_success {
        ServerRepository::update_status(
            &ctx.db,
            job.server_id,
            mcp_common::types::ServerStatus::Failed,
            None,
        )
        .await
        .ok();
        ctx.events.publish_server_status(
            job.server_id,
            mcp_common::types::ServerStatus::Failed,
            None,
            Some(full_error_msg.clone()),
        ).await.ok();
    } else {
        tracing::info!(
            "Skipping server status update to Failed for deployment {} (version {}) - newer successful deployment exists",
            job.deployment_id,
            current_version
        );
    }

    send_deploy_notification(ctx, job.server_id, false, Some(&full_error_msg)).await;
}

/// Extract tarball to directory
fn extract_tarball(tarball: &[u8], dest: &std::path::Path) -> anyhow::Result<()> {
    use flate2::read::GzDecoder;
    use std::io::Cursor;
    use tar::Archive;

    let gz = GzDecoder::new(Cursor::new(tarball));
    let mut archive = Archive::new(gz);

    // GitHub tarballs have a top-level directory like "owner-repo-sha/"
    // We need to strip this prefix
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();

        // Skip symlinks for security
        if entry.header().entry_type().is_symlink() {
            continue;
        }

        // SECURITY: reject any member whose path tries to escape the destination via an
        // absolute path or a `..` traversal (tar slip). A crafted archive could otherwise
        // write outside `dest` (e.g. over /etc or the builder's own files).
        use std::path::Component;
        if path.components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
            tracing::warn!("Skipping unsafe tarball entry: {}", path.display());
            continue;
        }

        // Strip the first component (the top-level directory)
        let components: Vec<_> = path.components().collect();
        if components.len() > 1 {
            let stripped: std::path::PathBuf = components[1..].iter().collect();
            let dest_path = dest.join(&stripped);

            // Defence in depth: the resolved target must stay under `dest`.
            if !dest_path.starts_with(dest) {
                tracing::warn!("Skipping tarball entry escaping dest: {}", dest_path.display());
                continue;
            }

            if entry.header().entry_type().is_dir() {
                std::fs::create_dir_all(&dest_path)?;
            } else {
                if let Some(parent) = dest_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                entry.unpack(&dest_path)?;
            }
        }
    }

    Ok(())
}

/// Validate git branch name to prevent command injection
/// Branch names must follow git naming conventions
fn validate_branch_name(branch: &str) -> anyhow::Result<()> {
    if branch.is_empty() {
        anyhow::bail!("Branch name cannot be empty");
    }

    // Max length check
    if branch.len() > 255 {
        anyhow::bail!("Branch name exceeds maximum length of 255 characters");
    }

    // Cannot start with dash (could be interpreted as option)
    if branch.starts_with('-') {
        anyhow::bail!("Branch name cannot start with a dash");
    }

    // Must not contain dangerous characters
    let forbidden_chars = [' ', '\t', '\n', '\r', '\0', '~', '^', ':', '?', '*', '[', '\\'];
    for c in branch.chars() {
        if forbidden_chars.contains(&c) {
            anyhow::bail!("Branch name contains invalid character: {:?}", c);
        }
    }

    // Must not contain ..
    if branch.contains("..") {
        anyhow::bail!("Branch name cannot contain '..'");
    }

    // Must not end with .lock
    if branch.ends_with(".lock") {
        anyhow::bail!("Branch name cannot end with '.lock'");
    }

    // Must not be empty after trimming
    if branch.trim().is_empty() {
        anyhow::bail!("Branch name cannot be empty or whitespace only");
    }

    Ok(())
}

/// Validate GitHub repository name to prevent injection
fn validate_github_repo(repo: &str) -> anyhow::Result<()> {
    if repo.is_empty() {
        anyhow::bail!("Repository name cannot be empty");
    }

    // Must be in owner/repo format
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Repository must be in 'owner/repo' format");
    }

    let owner = parts[0];
    let repo_name = parts[1];

    // Validate owner
    if owner.is_empty() || owner.len() > 39 {
        anyhow::bail!("Invalid repository owner");
    }

    // Owner must start with alphanumeric
    if !owner.chars().next().unwrap().is_ascii_alphanumeric() {
        anyhow::bail!("Repository owner must start with alphanumeric character");
    }

    // Owner must contain only alphanumeric and hyphens
    if !owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        anyhow::bail!("Repository owner contains invalid characters");
    }

    // Validate repo name
    if repo_name.is_empty() || repo_name.len() > 100 {
        anyhow::bail!("Invalid repository name length");
    }

    // Repo name must contain only alphanumeric, hyphens, underscores, and dots
    if !repo_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
        anyhow::bail!("Repository name contains invalid characters");
    }

    Ok(())
}

/// Clone repository using git
async fn clone_repo(
    github_repo: &str,
    branch: &str,
    dest: &std::path::Path,
) -> anyhow::Result<()> {
    // Validate inputs to prevent command injection
    validate_github_repo(github_repo)?;
    validate_branch_name(branch)?;

    let output = tokio::process::Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            branch,
            &format!("https://github.com/{}.git", github_repo),
            dest.to_str().ok_or_else(|| anyhow::anyhow!("Invalid path"))?,
        ])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Combine stdout and stderr for error message
        let mut error_details = String::new();
        if !stdout.is_empty() {
            error_details.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !error_details.is_empty() {
                error_details.push('\n');
            }
            error_details.push_str(&stderr);
        }

        return Err(anyhow::anyhow!("Git clone failed: {}", error_details.trim()));
    }

    Ok(())
}

async fn handle_deploy_job(job: DeployJob, ctx: Data<Arc<BuilderContext>>) -> Result<(), Error> {
    tracing::info!("Processing deploy job: {:?} to region {}", job.deployment_id, job.region);

    // Update deployment status
    DeploymentRepository::update(
        &ctx.db,
        job.deployment_id,
        UpdateDeployment {
            status: Some(mcp_common::types::DeploymentStatus::Deploying),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| Error::Failed(Arc::new(e.into())))?;

    // Update region status to deploying
    ServerRegionRepository::update_status(
        &ctx.db,
        job.server_id,
        &job.region,
        RegionStatus::Deploying,
    )
    .await
    .ok();

    // Publish deploying status
    ctx.events
        .publish_deployment_status(
            job.deployment_id,
            job.server_id,
            mcp_common::types::DeploymentStatus::Deploying,
            None,
            Some(80),
        )
        .await
        .ok();

    // Run the pre-built container
    match containerd::run_container(&ctx.config, &job).await {
        Ok(endpoint_url) => {
            DeploymentRepository::update(
                &ctx.db,
                job.deployment_id,
                UpdateDeployment {
                    status: Some(mcp_common::types::DeploymentStatus::Succeeded),
                    finished_at: Some(chrono::Utc::now()),
                    ..Default::default()
                },
            )
            .await
            .ok();

            // Publish succeeded status
            ctx.events
                .publish_deployment_status(
                    job.deployment_id,
                    job.server_id,
                    mcp_common::types::DeploymentStatus::Succeeded,
                    None,
                    Some(100),
                )
                .await
                .ok();

            if let Err(e) = ServerRepository::update_status(
                &ctx.db,
                job.server_id,
                mcp_common::types::ServerStatus::Running,
                Some(&endpoint_url),
            )
            .await
            {
                tracing::error!("Failed to update server status to Running: {}", e);
            }

            // Update region status to running with endpoint URL
            if let Err(e) = ServerRegionRepository::update(
                &ctx.db,
                job.server_id,
                &job.region,
                UpdateServerRegion {
                    status: Some(RegionStatus::Running),
                    endpoint_url: Some(endpoint_url.clone()),
                    ..Default::default()
                },
            )
            .await
            {
                tracing::error!("Failed to update region status: {}", e);
            }

            tracing::info!(
                "Deploy to region {} succeeded for server {}",
                job.region,
                job.server_id
            );

            // Send success email notification
            send_deploy_notification(&ctx, job.server_id, true, None).await;
        }
        Err(e) => {
            tracing::error!("Deploy to region {} failed: {}", job.region, e);
            let error_msg = e.to_string();

            // Get user's locale preference and analyze error with localized hints
            let locale = get_user_locale_for_server(&ctx.db, job.server_id).await;
            let hint = analyze_error_for_hints(&ctx.db, &error_msg, &locale).await;
            let full_error_msg = if let Some(ref hint) = hint {
                format!("{}{}", error_msg, hint)
            } else {
                error_msg.clone()
            };

            DeploymentRepository::update(
                &ctx.db,
                job.deployment_id,
                UpdateDeployment {
                    status: Some(mcp_common::types::DeploymentStatus::Failed),
                    error_message: Some(full_error_msg.clone()),
                    finished_at: Some(chrono::Utc::now()),
                    ..Default::default()
                },
            )
            .await
            .ok();

            // Publish failed status
            ctx.events
                .publish_deployment_status(
                    job.deployment_id,
                    job.server_id,
                    mcp_common::types::DeploymentStatus::Failed,
                    Some(full_error_msg.clone()),
                    Some(100),
                )
                .await
                .ok();

            // Only set server to failed if this is the latest deployment
            let should_set_failed = match DeploymentRepository::find_latest_by_server(&ctx.db, job.server_id).await {
                Ok(Some(latest)) => latest.id == job.deployment_id,
                _ => true,
            };

            if should_set_failed {
                if let Err(e) = ServerRepository::update_status(
                    &ctx.db,
                    job.server_id,
                    mcp_common::types::ServerStatus::Failed,
                    None,
                )
                .await
                {
                    tracing::error!("Failed to update server status to Failed: {}", e);
                }
            } else {
                tracing::info!(
                    "Skipping server status update to Failed for deployment {} - newer deployment exists",
                    job.deployment_id
                );
            }

            // Update region status to failed
            ServerRegionRepository::update_status(
                &ctx.db,
                job.server_id,
                &job.region,
                RegionStatus::Failed,
            )
            .await
            .ok();

            // Send failure email notification
            send_deploy_notification(&ctx, job.server_id, false, Some(&full_error_msg)).await;
        }
    }

    Ok(())
}

/// Tear down a deleted server's container. Retries transient failures with backoff;
/// destroying a missing container is a no-op, so this converges to "container is gone".
async fn handle_destroy_job(job: DestroyJob, ctx: Data<Arc<BuilderContext>>) -> Result<(), Error> {
    tracing::info!("Processing destroy job: container={}, server={}", job.app_name, job.server_id);

    const ATTEMPTS: usize = 5;
    let mut last_err = None;
    for attempt in 1..=ATTEMPTS {
        match containerd::destroy_container(&job.app_name).await {
            Ok(()) => {
                tracing::info!("Container {} destroyed (server {})", job.app_name, job.server_id);
                if let Err(e) = ServerRepository::delete(&ctx.db, job.server_id).await {
                    tracing::error!(
                        "Container {} destroyed but failed to hard-delete server row {}: {}",
                        job.app_name, job.server_id, e
                    );
                    return Err(Error::Failed(Arc::new(e.into())));
                }
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(
                    "destroy_container {} attempt {}/{} failed: {}",
                    job.app_name, attempt, ATTEMPTS, e
                );
                last_err = Some(e);
                if attempt < ATTEMPTS {
                    let secs = (5u64 << (attempt - 1)).min(30);
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                }
            }
        }
    }
    // Exhausted retries — return Err so the queue can re-attempt later rather than
    // silently leaving an orphan.
    Err(Error::Failed(Arc::new(
        last_err
            .unwrap_or_else(|| anyhow::anyhow!("destroy failed"))
            .into(),
    )))
}

/// Scale-to-zero scaler: every 5 minutes, stop containers that have had no
/// requests for more than 5 minutes (Redis key `last_request:{server_id}` absent or expired).
async fn run_scale_to_zero_scaler(
    db: mcp_db::DbPool,
    redis_url: String,
    active: fork_socket::ActiveMap,
) {
    let idle_threshold_secs: u64 = std::env::var("SCALE_TO_ZERO_IDLE_SECS")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(300);
    let check_interval_secs: u64 = std::env::var("SCALE_TO_ZERO_CHECK_SECS")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(300);

    let redis_client = match redis::Client::open(redis_url.as_str()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("scale-to-zero: failed to connect to Redis: {}", e);
            return;
        }
    };

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(check_interval_secs)).await;

        let servers = match ServerRepository::list_running(&db).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("scale-to-zero: failed to list running servers: {}", e);
                continue;
            }
        };

        let mut conn = match redis_client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("scale-to-zero: Redis connection failed: {}", e);
                continue;
            }
        };

        let now = chrono::Utc::now().timestamp();

        for server in servers {
            let key = format!("last_request:{}", server.id);
            let last_ts: Option<i64> = redis::cmd("GET")
                .arg(&key)
                .query_async(&mut conn)
                .await
                .ok()
                .and_then(|v: Option<String>| v?.parse().ok());

            let is_idle = match last_ts {
                None => true,
                Some(ts) => (now - ts) as u64 >= idle_threshold_secs,
            };

            if !is_idle {
                continue;
            }

            // If no Firecracker socket exists the VM is not running here — either it
            // crashed, was never deployed on this host, or is still on Fly.io.
            // Skip silently; the proxy will trigger a fresh boot when needed.
            if !firecracker::vm_socket_exists(&server.container_name) {
                tracing::debug!("scale-to-zero: no VM socket for {} — skipping", server.container_name);
                continue;
            }

            // Skip VMs with active proxy connections (VmLease or ForkLease held).
            // This prevents snapshotting a VM mid-request, and prevents overwriting
            // snapshot files while a fork is being restored from them.
            let n = fork_socket::active_count(&active, &server.container_name);
            if n > 0 {
                tracing::debug!("scale-to-zero: {} has {} active connection(s), skipping", server.container_name, n);
                continue;
            }


            tracing::info!("scale-to-zero: snapshotting idle VM {} (server={})", server.container_name, server.id);

            match firecracker::snapshot(&server.container_name).await {
                Ok(()) => {
                    if let Err(e) = ServerRepository::update_status(&db, server.id, mcp_common::types::ServerStatus::Stopped, None).await {
                        tracing::error!("scale-to-zero: failed to update status for {}: {}", server.id, e);
                    } else {
                        tracing::info!("scale-to-zero: VM {} snapshotted and stopped", server.container_name);
                    }
                }
                Err(e) => {
                    tracing::warn!("scale-to-zero: snapshot of {} failed: {}", server.container_name, e);
                }
            }
        }
    }
}

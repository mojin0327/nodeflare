use anyhow::Result;
use axum::{
    body::Body,
    extract::{Host, Path, State},
    http::{header::HeaderValue, HeaderMap, Request, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Json, Router,
};
use bytes::Bytes;
use fred::interfaces::ClientLike;
use futures::StreamExt;
use mcp_common::{AppConfig, McpMethod, ScopeChecker};
use mcp_db::{ServerRepository, WorkspaceRepository};
use auth::AuthCredential;
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;
use tokio::net::TcpListener;
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod auth;
mod cache;
mod code_mode;
mod code_runner;
mod embedding;
mod fork_socket;
mod mcp_transform;
mod meta_tools;
mod rate_limit;
mod redis_cache;
mod unix_client;

use cache::{RequestCache, CoalesceResult};
use mcp_db::{Tool, ToolRepository, UpsertTool};
use redis_cache::{RedisCache, CachedServer, CodeExecContext};

/// RAII guard that decrements the per-server active-request counter on drop.
///
/// Guarantees the decrement fires on ALL exit paths, including `?` unwinding
/// (forward_request error, auth failure, etc.).  Without this guard the counter
/// would drift upward on any error, causing every subsequent request to look
/// concurrent and always fork — breaking the fork-vs-direct decision.
///
/// Uses DashMap so concurrent requests for *different* servers never contend
/// on the same lock shard.
struct ActiveRequestGuard<'a> {
    map: &'a dashmap::DashMap<uuid::Uuid, usize>,
    server_id: uuid::Uuid,
}

impl<'a> Drop for ActiveRequestGuard<'a> {
    fn drop(&mut self) {
        use dashmap::mapref::entry::Entry;
        match self.map.entry(self.server_id) {
            Entry::Occupied(mut e) => {
                let v = e.get().saturating_sub(1);
                if v == 0 {
                    e.remove();
                } else {
                    *e.get_mut() = v;
                }
            }
            Entry::Vacant(_) => {}
        }
    }
}

pub struct ProxyState {
    pub config: AppConfig,
    pub db: mcp_db::DbPool,
    pub redis: fred::prelude::RedisClient,
    pub http_client: reqwest::Client,
    /// Long-lived client for SSE/streaming upstream requests (idle timeout, no total
    /// timeout). Built once and reused so connections are pooled across requests.
    pub sse_client: reqwest::Client,
    pub request_cache: RequestCache,
    pub redis_cache: RedisCache,
    /// Maximum request body size accepted from clients / read from upstreams.
    pub body_limit: usize,
    /// Maximum size of a (buffered, non-SSE) upstream response we will read into memory.
    /// Upstreams are tenant-controlled, so this bounds a single response's memory use and
    /// prevents a malicious/huge reply from OOM-ing the shared proxy.
    pub max_upstream_response_bytes: usize,
    /// Gemini embedding client for semantic search_tools. `None` disables semantic
    /// search (falls back to lexical) when GEMINI_API_KEY is unset.
    pub embedding: Option<embedding::EmbeddingClient>,
    /// Sandboxed code runner for code mode. `None` disables run_code execution when
    /// PROXY_CODE_RUNNER_URL is unset.
    pub code_runner: Option<code_runner::CodeRunnerClient>,
    /// Meta-tool definitions loaded from the DB at startup. The name/description are
    /// configurable per row; handler_type routes to the correct in-proxy logic.
    pub meta_tools: Vec<mcp_db::ProxyMetaTool>,
    /// Builder internal API URL for scale-to-zero wake/stop operations.
    pub builder_internal_url: Option<String>,
    /// Bearer token for authenticating to Builder internal API.
    pub builder_token: Option<String>,
    /// In-process throttle for `last_request` Redis writes (120 s window by default).
    /// Avoids a Redis SET on every single request; scale-to-zero only needs
    /// minute-level precision, so one write per server per window is enough.
    /// DashMap: requests for different servers never contend on the same lock shard.
    pub last_request_throttle: dashmap::DashMap<uuid::Uuid, std::time::Instant>,
    /// Per-server in-flight request counter for fork decisions.
    /// Incremented when a request starts forwarding, decremented on completion.
    /// DashMap: requests for different servers never contend on the same lock shard.
    pub active_requests: dashmap::DashMap<uuid::Uuid, usize>,
    /// Path to the builder's Unix socket for fork VM lifecycle management.
    /// When set, fork requests go through the socket (OS-guaranteed cleanup on crash).
    /// When None, falls back to the HTTP kill-fork API.
    pub fork_socket_path: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mcp_proxy=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::from_env()?;
    tracing::info!("Starting MCP Cloud Proxy Gateway");

    let db_pool = mcp_db::create_pool(&config).await?;

    let redis_config = fred::types::RedisConfig::from_url(&config.redis.url)?;

    // Configure Redis performance with short timeouts to prevent blocking Claude connections
    let perf_config = fred::types::PerformanceConfig {
        default_command_timeout: std::time::Duration::from_secs(2),
        ..Default::default()
    };

    // Configure connection with short timeouts - Redis failures should not block proxy
    let conn_config = fred::types::ConnectionConfig {
        connection_timeout: std::time::Duration::from_secs(2),
        internal_command_timeout: std::time::Duration::from_secs(2),
        ..Default::default()
    };

    // Reconnection policy: exponential backoff, retry forever (max_attempts=0)
    // This handles Upstash closing idle connections
    let reconnect_policy = fred::types::ReconnectPolicy::new_exponential(
        0,     // max_attempts: 0 = retry forever
        100,   // min_delay_ms: start with 100ms
        5000,  // max_delay_ms: cap at 5 seconds
        2,     // multiplier: double delay each attempt
    );

    let redis = fred::prelude::RedisClient::new(
        redis_config,
        Some(perf_config),
        Some(conn_config),
        Some(reconnect_policy),
    );
    redis.connect();
    tracing::info!("Connecting to Redis...");
    redis.wait_for_connect().await?;
    tracing::info!("Redis connected successfully with auto-reconnect enabled");

    // Upstream timeout is configurable; long-running tool calls were being cut at the
    // old hard-coded 30s. Also set a pool idle timeout so we don't pin dead connections.
    let upstream_timeout_secs: u64 = std::env::var("PROXY_UPSTREAM_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(620);
    let pool_idle_timeout_secs: u64 = std::env::var("PROXY_POOL_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(upstream_timeout_secs))
        .pool_idle_timeout(std::time::Duration::from_secs(pool_idle_timeout_secs))
        // SECURITY (SSRF): never auto-follow upstream redirects. Upstreams are
        // tenant-controlled and could 302 us at 169.254.169.254 / localhost / other
        // tenants' *.internal addresses. Return the 3xx to the caller instead.
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    // SSE/streaming client: built once and reused (was previously rebuilt per
    // request, defeating connection pooling). No total timeout — MCP SSE streams are
    // legitimately long-lived — but a read (idle) timeout drops silently hung upstreams.
    let sse_idle_timeout_secs: u64 = std::env::var("PROXY_SSE_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(620);
    let sse_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(sse_idle_timeout_secs))
        .pool_idle_timeout(std::time::Duration::from_secs(pool_idle_timeout_secs))
        // SECURITY (SSRF): see http_client above — don't follow tenant-controlled redirects.
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    // Request cache: TTL and max entries from environment
    let cache_ttl: u64 = std::env::var("PROXY_CACHE_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let cache_max_entries: usize = std::env::var("PROXY_CACHE_MAX_ENTRIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10000);
    let request_cache = RequestCache::new(cache_ttl, cache_max_entries);

    // Redis cache for API keys and server metadata
    let redis_cache = RedisCache::new(redis.clone());

    // Get request body limit from env (default: 10MB for proxy)
    let body_limit: usize = std::env::var("PROXY_BODY_LIMIT_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10 * 1024 * 1024); // 10MB default for proxy

    // Cap on how much of a (buffered) upstream response we read into memory (DoS guard).
    let max_upstream_response_bytes: usize = std::env::var("PROXY_MAX_UPSTREAM_RESPONSE_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10 * 1024 * 1024); // 10MB default

    // Optional Gemini embedding client for semantic search_tools (None = lexical only).
    let embedding = embedding::EmbeddingClient::from_env(http_client.clone());
    // Optional sandboxed code runner for code mode (None = run_code unavailable).
    let code_runner = code_runner::CodeRunnerClient::from_env(http_client.clone());

    // Load meta-tool definitions from DB (name/description configurable without redeploy).
    let meta_tools = mcp_db::ProxyMetaToolRepository::list_enabled(&db_pool)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("Failed to load proxy meta-tools from DB, using defaults: {}", e);
            meta_tools::default_definitions()
        });
    tracing::info!("Loaded {} proxy meta-tool definitions", meta_tools.len());

    let builder_internal_url = std::env::var("BUILDER_INTERNAL_URL").ok();
    let builder_token = std::env::var("BUILDER_TOKEN").ok();
    let fork_socket_path = std::env::var("FORK_SOCKET_PATH").ok();

    if fork_socket_path.is_some() {
        tracing::info!("Fork socket mode enabled: {}", fork_socket_path.as_deref().unwrap_or(""));
    } else {
        tracing::info!("Fork socket mode disabled (FORK_SOCKET_PATH unset), using HTTP kill-fork fallback");
    }

    let state = Arc::new(ProxyState {
        config: config.clone(),
        db: db_pool,
        redis,
        http_client,
        sse_client,
        request_cache,
        redis_cache,
        body_limit,
        max_upstream_response_bytes,
        embedding,
        code_runner,
        meta_tools,
        builder_internal_url,
        builder_token,
        last_request_throttle: dashmap::DashMap::new(),
        active_requests: dashmap::DashMap::new(),
        fork_socket_path,
    });

    let app = Router::new()
        .route("/health", any(health_check))
        // OAuth 2.1 metadata endpoint (RFC 8414)
        .route("/.well-known/oauth-authorization-server", get(oauth_metadata))
        // RFC 9728 allows the discovery URL to include the resource path as a suffix,
        // e.g. /.well-known/oauth-protected-resource/mcp for resource https://host/mcp.
        .route("/.well-known/oauth-authorization-server/*path", get(oauth_metadata))
        // OAuth 2.0 Protected Resource Metadata (RFC 9728)
        .route("/.well-known/oauth-protected-resource", get(protected_resource_metadata))
        .route("/.well-known/oauth-protected-resource/*path", get(protected_resource_metadata))
        // Internal: sandboxed code (code mode) calls tools back through here. This is the
        // security boundary — it re-checks scope from the exec token, not the sandbox.
        .route("/internal/code-exec/:server_id/tools-call", post(code_exec_tools_call))
        // Subdomain-based routing: {slug}.mcp.cloud/* -> MCP server
        .fallback(any(proxy_handler))
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(body_limit))
        .with_state(state);

    // Bind dual-stack (IPv6 + IPv4). Fly private networking (`.internal`) is IPv6-only,
    // and the code runner's sandbox reaches the internal tool-call endpoint over it; the
    // public fly-proxy path uses IPv4. `[::]` on Fly's Linux is dual-stack (bindv6only=0),
    // so a single listener serves both. (config.server.host is still used for URLs.)
    let addr = format!("[::]:{}", config.server.proxy_port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("Proxy gateway listening on {} (dual-stack)", addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;

    Ok(())
}

async fn health_check() -> &'static str {
    "ok"
}

/// OAuth 2.1 Authorization Server Metadata (RFC 8414)
#[derive(Debug, Serialize)]
struct OAuthServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: String,
    response_types_supported: Vec<String>,
    grant_types_supported: Vec<String>,
    code_challenge_methods_supported: Vec<String>,
    token_endpoint_auth_methods_supported: Vec<String>,
}

/// OAuth metadata endpoint - returns API server's OAuth endpoints
async fn oauth_metadata(
    State(state): State<Arc<ProxyState>>,
    Host(host): Host,
) -> Response {
    tracing::info!("OAuth metadata requested from proxy, host={}", host);

    // If the target server has NodeFlare auth disabled, do NOT advertise OAuth
    // authorization-server metadata. Some MCP clients key auth discovery off this
    // endpoint (the older 2025-03-26 flow) and would start an OAuth flow even though
    // the proxy forwards requests without requiring any credentials. Returning 404
    // keeps the "no auth" contract consistent with the protected-resource and MCP
    // endpoints (both already signal "not protected" for auth-disabled servers).
    // Note: if the subdomain can't be resolved to a running server we fall through to
    // advertising metadata to avoid changing edge-case responses.
    if let Ok(slug) = extract_subdomain(&host, &state.config.server.proxy_base_domain) {
        if let Ok(server) = resolve_server(&state, &slug).await {
            if !server.auth_enabled {
                tracing::info!(
                    "Auth disabled for server {}, not advertising OAuth authorization-server metadata",
                    slug
                );
                return StatusCode::NOT_FOUND.into_response();
            }
        }
    }

    let api_url = std::env::var("API_URL").unwrap_or_else(|_| {
        format!("http://{}:{}", state.config.server.host, state.config.server.port)
    });

    tracing::info!("OAuth metadata: api_url={}", api_url);

    Json(OAuthServerMetadata {
        issuer: api_url.clone(),
        authorization_endpoint: format!("{}/oauth/authorize", api_url),
        token_endpoint: format!("{}/oauth/token", api_url),
        registration_endpoint: format!("{}/oauth/register", api_url),
        response_types_supported: vec!["code".to_string()],
        grant_types_supported: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        code_challenge_methods_supported: vec!["S256".to_string()],
        token_endpoint_auth_methods_supported: vec![
            "client_secret_basic".to_string(),
            "client_secret_post".to_string(),
            "none".to_string(),
        ],
    })
    .into_response()
}

/// OAuth 2.0 Protected Resource Metadata (RFC 9728)
#[derive(Debug, Serialize)]
struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    scopes_supported: Vec<String>,
}

/// OAuth 2.0 Protected Resource Metadata endpoint (RFC 9728)
async fn protected_resource_metadata(
    State(state): State<Arc<ProxyState>>,
    Host(host): Host,
) -> Response {
    tracing::info!("Protected resource metadata requested from proxy, host={}", host);

    // If the target server has NodeFlare auth disabled, do NOT advertise OAuth
    // protected-resource metadata. Otherwise MCP clients (which probe this endpoint
    // during discovery per RFC 9728) would enforce auth client-side even though the
    // proxy forwards requests without requiring any credentials.
    // Note: if the subdomain can't be resolved to a running server we fall through to
    // the previous behavior (advertise metadata) to avoid changing edge-case responses.
    if let Ok(slug) = extract_subdomain(&host, &state.config.server.proxy_base_domain) {
        if let Ok(server) = resolve_server(&state, &slug).await {
            if !server.auth_enabled {
                tracing::info!(
                    "Auth disabled for server {}, not advertising OAuth protected-resource metadata",
                    slug
                );
                return StatusCode::NOT_FOUND.into_response();
            }
        }
    }

    let api_url = std::env::var("API_URL").unwrap_or_else(|_| {
        format!("http://{}:{}", state.config.server.host, state.config.server.port)
    });

    // Resource is this proxy endpoint (RFC 9728 format)
    let resource = format!("https://{}", host);
    tracing::info!("Protected resource metadata: resource={}, authorization_servers=[{}]", resource, api_url);

    Json(ProtectedResourceMetadata {
        resource,
        authorization_servers: vec![api_url],
        scopes_supported: vec![
            "*".to_string(),
            "tools:*".to_string(),
            "tools:list".to_string(),
            "tools:call".to_string(),
            "resources:*".to_string(),
            "resources:list".to_string(),
            "resources:read".to_string(),
            "prompts:*".to_string(),
            "prompts:list".to_string(),
            "prompts:get".to_string(),
        ],
    })
    .into_response()
}

async fn proxy_handler(
    State(state): State<Arc<ProxyState>>,
    Host(host): Host,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    uri: Uri,
    request: Request<Body>,
) -> Result<Response, ProxyError> {
    let start = Instant::now();

    // Detect browser access and redirect to main site.
    // OAuth paths (/authorize, /callback, /oauth/*) must pass through even from browsers.
    let path = uri.path();
    let is_oauth_path = path.starts_with("/authorize")
        || path.starts_with("/callback")
        || path.starts_with("/oauth/");

    let is_browser = !is_oauth_path
        && request.method() == axum::http::Method::GET
        && request
            .headers()
            .get(axum::http::header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .map(|accept| {
                accept.contains("text/html")
                    && !accept.contains("application/json")
                    && !accept.contains("text/event-stream")
            })
            .unwrap_or(false);

    if is_browser {
        tracing::info!("Browser access detected, redirecting to main site");
        let main_site_url = std::env::var("MAIN_SITE_URL")
            .unwrap_or_else(|_| "https://nodeflare.tech".to_string());
        return Ok(Response::builder()
            .status(StatusCode::FOUND)
            .header(axum::http::header::LOCATION, main_site_url)
            .body(Body::empty())
            .unwrap());
    }

    let client_ip = rate_limit::extract_client_ip(request.headers(), &addr);
    tracing::info!(
        "proxy_handler: host={}, uri={}, client_ip={}, base_domain={}",
        host, uri, client_ip, state.config.server.proxy_base_domain
    );

    let server_slug = extract_subdomain(&host, &state.config.server.proxy_base_domain)?;
    tracing::info!("proxy_handler: extracted slug={}", server_slug);

    let host_str = host.clone();
    let add_host_to_error = |e: ProxyError| -> ProxyError {
        match e {
            ProxyError::Unauthorized(msg, _) => ProxyError::Unauthorized(msg, Some(host_str.clone())),
            other => other,
        }
    };

    let server = resolve_server(&state, &server_slug).await?;
    tracing::info!(
        "proxy_handler: server resolved, id={}, auth_enabled={}",
        server.id, server.auth_enabled
    );

    // For stopped servers: peek at the first 512B of POST bodies to detect MCP method.
    // Lets us serve tools/list from Redis cache while waking the VM in the background —
    // auth hits Redis/DB (not the VM), so we do it before the wake.
    let (request, stopped_peek) =
        if server.status == "stopped" && request.method() == axum::http::Method::POST {
            let (parts, body) = request.into_parts();
            let prefix = axum::body::to_bytes(body, 512).await.unwrap_or_default();
            let mcp_method = serde_json::from_slice::<serde_json::Value>(&prefix)
                .ok()
                .and_then(|v| v.get("method").and_then(|m| m.as_str()).map(str::to_string));
            (
                Request::from_parts(parts, Body::from(prefix.clone())),
                Some((prefix, mcp_method)),
            )
        } else {
            (request, None)
        };

    // Auth — runs before scale-to-zero wake because credential validation uses
    // Redis/DB only and does not require the upstream VM to be running.
    let credential: Option<AuthCredential> = if server.auth_enabled {
        let api_key = auth::extract_api_key(&request).map_err(|e| add_host_to_error(e))?;
        tracing::debug!(
            "proxy_handler: API key extracted (prefix={}...)",
            &api_key[..api_key.len().min(8)]
        );
        let credential = auth::validate_credential(&state, &api_key, &client_ip)
            .await
            .map_err(|e| add_host_to_error(e))?;
        tracing::info!("proxy_handler: credential validated (took {:?})", start.elapsed());
        authorize_credential_for_server(&state, &credential, &server).await?;
        tracing::debug!("proxy_handler: checking rate limit");
        match rate_limit::check(&state, credential.id(), &server).await {
            Ok(_) => tracing::debug!("proxy_handler: rate limit check passed"),
            Err(ProxyError::RateLimitExceeded) => return Err(ProxyError::RateLimitExceeded),
            Err(e) => tracing::warn!("proxy_handler: rate limit check failed (continuing): {}", e),
        }
        Some(credential)
    } else {
        let path = uri.path();
        if path == "/register" || path.ends_with("/register") {
            tracing::info!(
                "proxy_handler: auth disabled, rejecting /register for server {}",
                server.id
            );
            return Ok(StatusCode::NOT_FOUND.into_response());
        }
        tracing::info!(
            "proxy_handler: auth disabled for server {}, skipping credential validation",
            server.id
        );
        // auth_enabled=false: no credential to throttle by, so enforce a per-IP
        // cap to prevent a single client from flooding the server.
        let ip_limit: u64 = std::env::var("UNAUTHENTICATED_RATE_LIMIT_PER_MINUTE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60);
        match rate_limit::check_by_ip(&state, &client_ip, ip_limit).await {
            Ok(_) => {}
            Err(ProxyError::RateLimitExceeded) => return Err(ProxyError::RateLimitExceeded),
            Err(e) => tracing::warn!("proxy_handler: IP rate limit check failed (continuing): {}", e),
        }
        None
    };

    // Monthly quota.
    tracing::debug!("proxy_handler: checking monthly quota");
    match rate_limit::check_and_increment_monthly_quota(&state, server.workspace_id).await {
        Ok(_) => tracing::debug!("proxy_handler: monthly quota check passed"),
        Err(e @ ProxyError::QuotaExceeded(_)) => return Err(e),
        Err(e @ ProxyError::PaymentRequired(_)) => return Err(e),
        Err(e @ ProxyError::RateLimitExceeded) => return Err(e),
        Err(e) => tracing::warn!("proxy_handler: monthly quota check failed (continuing): {}", e),
    }

    // Scale-to-zero wake.
    if server.status == "stopped" {
        if let (Some(builder_url), Some(token)) = (&state.builder_internal_url, &state.builder_token) {
            // Predictive wake: if this is tools/list and we have a Redis-cached response,
            // return it instantly and wake the VM in the background. By the time the AI
            // sends the first tools/call (5–30 s of AI think time), the VM is ready.
            if let Some((ref peek_bytes, Some(ref method))) = stopped_peek {
                if method == "tools/list" {
                    if let Some(cached) = state.redis_cache.get_tools_list(&server.id).await {
                        tracing::info!(
                            "predictive-wake[{}]: tools/list cache HIT — bg wake, instant response",
                            server.container_name
                        );
                        let bg_url = format!("{}/internal/start/{}", builder_url, server.container_name);
                        let bg_token = token.clone();
                        let bg_client = state.http_client.clone();
                        let bg_slug = server_slug.clone();
                        let bg_redis = state.redis_cache.clone();
                        tokio::spawn(async move {
                            let _ = bg_client.post(&bg_url).bearer_auth(&bg_token).send().await;
                            bg_redis.invalidate_server(&bg_slug).await;
                        });

                        let mcp_info = extract_mcp_request_info(peek_bytes);
                        let default_headers =
                            vec![("content-type".to_string(), "application/json".to_string())];
                        let fwd = ForwardContext {
                            auth_enabled: server.auth_enabled,
                            client_ip: &client_ip,
                            host: &host,
                            server_id: server.id,
                            filter_by_scope: server.tool_list_filter_by_scope,
                            slim: server.tool_schema_slim,
                            search_mode: server.tool_search_mode,
                            code_mode: server.tool_code_mode,
                        };
                        let body = inject_jsonrpc_id(&cached, mcp_info.id.as_ref());
                        let body = transform_list_body(
                            &mcp_info,
                            &default_headers,
                            body,
                            credential.as_ref(),
                            &fwd,
                            &state.meta_tools,
                        );
                        let duration_ms = start.elapsed().as_millis() as i32;
                        let server_id = server.id;
                        let api_key_id = credential.as_ref().map(|c| c.id());
                        let db = state.db.clone();
                        tokio::spawn(async move {
                            let _ = mcp_db::RequestLogRepository::create(
                                &db,
                                mcp_db::CreateRequestLog {
                                    server_id,
                                    tool_name: None,
                                    api_key_id,
                                    client_info: None,
                                    request_body: None,
                                    response_status: "success".to_string(),
                                    error_message: None,
                                    duration_ms,
                                },
                            )
                            .await;
                        });
                        return Ok(build_response(200, &default_headers, body)?);
                    }
                }
            }

            // Cache miss or non-tools/list: wake normally.
            // builder's /internal/start blocks until wait_for_port succeeds, so the
            // 200 response means the VM is already accepting connections — no TCP
            // polling needed on this side.
            tracing::info!(
                "proxy_handler: waking stopped server {} (container={})",
                server.id, server.container_name
            );
            let wake_url = format!("{}/internal/start/{}", builder_url, server.container_name);
            let wake_result = state.http_client
                .post(&wake_url)
                .bearer_auth(token)
                .send()
                .await;
            match wake_result {
                Ok(r) if r.status().is_success() => {
                    tracing::info!(
                        "proxy_handler: VM {} is up (builder confirmed ready)",
                        server.container_name
                    );
                    state.redis_cache.invalidate_server(&server_slug).await;
                }
                Ok(r) => tracing::warn!("proxy_handler: wake returned {}", r.status()),
                Err(e) => tracing::warn!("proxy_handler: wake failed: {}", e),
            }
        }
    }

    // Record last_request timestamp for scale-to-zero idle detection.
    // Throttled to at most one Redis write per PROXY_LAST_REQUEST_THROTTLE_SECS per server
    // (default 120 s). The scaler only needs minute-level precision (idle threshold is
    // 5 min by default), so one write per 2 minutes is sufficient.
    {
        static THROTTLE: std::sync::OnceLock<std::time::Duration> = std::sync::OnceLock::new();
        let throttle = *THROTTLE.get_or_init(|| {
            let secs: u64 = std::env::var("PROXY_LAST_REQUEST_THROTTLE_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120);
            std::time::Duration::from_secs(secs)
        });
        let should_write = {
            let now = std::time::Instant::now();
            // DashMap: each server_id hits at most one shard, so concurrent
            // requests for different servers never block each other here.
            match state.last_request_throttle.get(&server.id) {
                Some(t) if t.elapsed() < throttle => false,
                _ => {
                    state.last_request_throttle.insert(server.id, now);
                    true
                }
            }
        };
        if should_write {
            let redis = state.redis.clone();
            let server_id = server.id.to_string();
            tokio::spawn(async move {
                let key = format!("last_request:{}", server_id);
                let _: Result<(), _> = fred::prelude::KeysInterface::set(
                    &redis,
                    &key,
                    chrono::Utc::now().timestamp(),
                    Some(fred::types::Expiration::EX(600)),
                    None,
                    false,
                )
                .await;
            });
        }
    }

    // Status gate: reject servers that are not running (building/failed/deleting).
    if server.status != "running" && server.status != "stopped" {
        tracing::warn!(
            "proxy_handler: server {} not serveable (status={})",
            server.id, server.status
        );
        return Err(ProxyError::ServiceUnavailable(format!(
            "Server is not running (status: {})",
            server.status
        )));
    }

    let endpoint_url = server.endpoint_url.as_ref().ok_or_else(|| {
        tracing::error!(
            "proxy_handler: server {} has no endpoint_url (not deployed)",
            server.id
        );
        ProxyError::ServiceUnavailable("Server not deployed".into())
    })?;

    let mcp_path = server.mcp_path.trim_start_matches('/');
    let query = uri.query().map(|q| format!("?{}", q)).unwrap_or_default();

    // VM fork: if there is already an in-flight request for this server and a
    // snapshot is available, spin up a fresh fork VM so concurrent requests
    // run in parallel rather than queuing behind each other.
    //
    // Two cleanup modes (selected by FORK_SOCKET_PATH env var):
    //   Socket mode: ForkLease / VmLease hold connections open; dropping them
    //     signals the builder (kill fork / decrement active count).  Guaranteed
    //     to fire even on proxy crash (kernel closes all fds on process exit).
    //   HTTP fallback: explicit POST /internal/kill-fork after the request
    //     completes.  Best-effort — orphans if the proxy crashes mid-request.
    // Unix socket endpoint_url: "unix:/run/mcp/server.sock"
    // → base_url: "unix:/run/mcp/server.sock|/mcp?query"
    // TCP endpoint_url: "http://127.0.0.1:PORT"
    // → base_url: "http://127.0.0.1:PORT/mcp?query"
    let base_url = if let Some(sock) = endpoint_url.strip_prefix("unix:") {
        format!("unix:{}|/{}{}", sock.trim_end_matches('/'), mcp_path, query)
    } else {
        format!("{}/{}{}", endpoint_url.trim_end_matches('/'), mcp_path, query)
    };

    // Increment the active-request counter and install a Drop guard so it is
    // ALWAYS decremented — even if forward_request returns an error or any
    // other ? unwinds the stack.  Previously the decrement was done manually
    // after forward_request, which was silently skipped on error paths.
    let concurrent = {
        let mut entry = state.active_requests.entry(server.id).or_insert(0);
        *entry += 1;
        *entry > 1
    };
    let _active_guard = ActiveRequestGuard {
        map: &state.active_requests,
        server_id: server.id,
    };

    // VmLease: held while this request is in flight.  The builder's scale-to-zero
    // scaler skips any container whose lease count > 0, preventing two races:
    //   1. Snapshotting a VM mid-request (kills the active connection).
    //   2. Overwriting snapshot files while a concurrent fork is restoring from them.
    // Best-effort: if the socket is unavailable we log and continue without a lease.
    //
    // NOTE: snapshot_ready must be checked AFTER hold_vm to close the TOCTOU window
    // where the builder could begin snapshotting between the exists() check and the
    // lease acquisition.
    let _vm_lease: Option<fork_socket::VmLease> =
        if let Some(socket_path) = state.fork_socket_path.as_deref() {
            match fork_socket::hold_vm(socket_path, &server.container_name).await {
                Ok(lease) => Some(lease),
                Err(e) => {
                    tracing::warn!(
                        "proxy_handler: vm_lease unavailable for {}, scale-to-zero may race: {}",
                        server.container_name, e
                    );
                    None
                }
            }
        } else {
            None
        };

    let snapshot_ready = {
        let snap_base = std::path::PathBuf::from("/opt/firecracker/snapshots")
            .join(&server.container_name);
        snap_base.join("snapshot.bin").exists() && snap_base.join("memory.bin").exists()
    };

    // fork_lease: held until end of handler; dropping it sends EOF to builder → kill_fork.
    // http_fork_info: used by the HTTP fallback path for the explicit kill-fork call.
    let mut fork_lease: Option<fork_socket::ForkLease> = None;
    let mut http_fork_info: Option<(String, String)> = None;

    let target_url: String = if concurrent && snapshot_ready {
        if let Some(socket_path) = state.fork_socket_path.as_deref() {
            // Socket mode: request fork and hold the lease alive.
            match fork_socket::request_fork(socket_path, &server.container_name).await {
                Ok(lease) => {
                    let fork_url = format!("http://127.0.0.1:{}/{}{}", lease.port, mcp_path, query);
                    tracing::info!(
                        "proxy_handler: forked {} → {} on port {} (socket)",
                        server.container_name, lease.fork_name, lease.port
                    );
                    fork_lease = Some(lease);
                    fork_url
                }
                Err(e) => {
                    tracing::warn!(
                        "proxy_handler: fork socket failed for {}, falling back to base VM: {}",
                        server.container_name, e
                    );
                    base_url.clone()
                }
            }
        } else if let (Some(builder_url), Some(token)) = (
            state.builder_internal_url.as_deref(),
            state.builder_token.as_deref(),
        ) {
            // HTTP fallback mode: call builder fork endpoint.
            let fork_url = format!("{}/internal/fork/{}", builder_url, server.container_name);
            match state.http_client.post(&fork_url).bearer_auth(token).send().await {
                Ok(r) if r.status().is_success() => {
                    if let Ok(body) = r.json::<serde_json::Value>().await {
                        let port = body["port"].as_u64();
                        let fork_name = body["fork_container"].as_str().map(|s| s.to_string());
                        let base_name = body["base_container"].as_str().map(|s| s.to_string());
                        if let (Some(p), Some(name), Some(base)) = (port, fork_name, base_name) {
                            tracing::info!(
                                "proxy_handler: forked {} → {} on port {} (http)",
                                server.container_name, name, p
                            );
                            http_fork_info = Some((base, name));
                            format!("http://127.0.0.1:{}/{}{}", p, mcp_path, query)
                        } else {
                            base_url.clone()
                        }
                    } else {
                        base_url.clone()
                    }
                }
                _ => base_url.clone(),
            }
        } else {
            base_url.clone()
        }
    } else {
        base_url.clone()
    };

    tracing::info!("proxy_handler: forwarding request to {}", target_url);

    let fwd = ForwardContext {
        auth_enabled: server.auth_enabled,
        client_ip: &client_ip,
        host: &host,
        server_id: server.id,
        filter_by_scope: server.tool_list_filter_by_scope,
        slim: server.tool_schema_slim,
        search_mode: server.tool_search_mode,
        code_mode: server.tool_code_mode,
    };
    let (response, mcp_info) =
        forward_request(&state, &target_url, request, credential.as_ref(), &fwd).await?;
    tracing::info!("proxy_handler: request forwarded, status={}", response.status());

    // _active_guard drops here (or on any error path above), decrementing the counter.
    // _vm_lease drops here, decrementing the base VM's active count in the builder.
    // Both are guaranteed to run on all exit paths via RAII Drop impls.
    drop(_active_guard);
    drop(_vm_lease);

    // Socket mode: fork_lease drops here, sending SHUT_WR → builder kills the fork VM.
    // This is OS-guaranteed: even if we never reach this point (e.g. `?` unwound above),
    // the lease will have already been dropped by the stack unwind, so cleanup fires.
    drop(fork_lease);

    // HTTP fallback mode: explicit kill-fork POST.
    if let Some((base_name, fork_name)) = http_fork_info {
        if let (Some(builder_url), Some(token)) = (
            state.builder_internal_url.as_deref(),
            state.builder_token.as_deref(),
        ) {
            let kill_url = format!("{}/internal/kill-fork/{}/{}", builder_url, base_name, fork_name);
            let client = state.http_client.clone();
            let token = token.to_string();
            tokio::spawn(async move {
                let _ = client.post(&kill_url).bearer_auth(token).send().await;
            });
        }
    }

    let duration_ms = start.elapsed().as_millis() as i32;
    let server_id = server.id;
    let api_key_id = credential.as_ref().map(|c| c.id());
    let status = if response.status().is_success() { "success" } else { "error" };
    let tool_name = mcp_info.target.clone();

    let db = state.db.clone();
    tokio::spawn(async move {
        let _ = mcp_db::RequestLogRepository::create(
            &db,
            mcp_db::CreateRequestLog {
                server_id,
                tool_name,
                api_key_id,
                client_info: None,
                request_body: None,
                response_status: status.to_string(),
                error_message: None,
                duration_ms,
            },
        )
        .await;
    });

    Ok(response)
}

/// Enforce that a validated credential is authorized to reach `server`'s workspace.
///
/// The target server is resolved from the caller-controlled host label, so credential
/// validity alone is not enough — we must confirm the credential's owner belongs to the
/// server's tenant, or one tenant's token could be replayed against another's server.
///
/// Rules (fail-closed):
/// 1. Server-bound credential (`server_id` set): must match this exact server.
/// 2. Workspace-bound credential (`workspace_id` set, e.g. API keys and workspace OAuth
///    clients): must match this server's workspace.
/// 3. Account-wide credential (DCR OAuth token, no workspace/server binding): the
///    authorizing user must be a member of this server's workspace.
async fn authorize_credential_for_server(
    state: &ProxyState,
    credential: &AuthCredential,
    server: &CachedServer,
) -> Result<(), ProxyError> {
    // 1. Server-bound credential.
    if let Some(cred_server_id) = credential.server_id() {
        if cred_server_id != server.id {
            tracing::warn!("authorize: credential server_id={} != server.id={}", cred_server_id, server.id);
            return Err(ProxyError::Forbidden("Credential not valid for this server".into()));
        }
        return Ok(());
    }

    // 2. Workspace-bound credential.
    if let Some(cred_ws) = credential.workspace_id() {
        if cred_ws != server.workspace_id {
            tracing::warn!("authorize: credential workspace_id={} != server.workspace_id={}", cred_ws, server.workspace_id);
            return Err(ProxyError::Forbidden("Credential not valid for this workspace".into()));
        }
        return Ok(());
    }

    // 3. Account-wide credential: only OAuth tokens reach here (API keys always carry a
    //    workspace_id). The authorizing user must be a member of the target workspace.
    match credential {
        AuthCredential::OAuthToken(token) => {
            let is_member = WorkspaceRepository::get_member(&state.db, server.workspace_id, token.user_id)
                .await
                .map_err(|e| ProxyError::Internal(e.to_string()))?
                .is_some();
            if !is_member {
                tracing::warn!(
                    "authorize: user_id={} is not a member of workspace_id={} (token client_id={})",
                    token.user_id, server.workspace_id, token.oauth_client_id
                );
                return Err(ProxyError::Forbidden("Token not authorized for this workspace".into()));
            }
            Ok(())
        }
        AuthCredential::ApiKey(_) => {
            // Unreachable: ApiKey::workspace_id is non-optional, so rule 2 already returned.
            Err(ProxyError::Forbidden("Credential not authorized".into()))
        }
    }
}

/// Resolve the server addressed by a request's host label.
///
/// `label` is the single subdomain label extracted from the host, i.e. the
/// `<workspace_slug>--<server_slug>` in `<workspace_slug>--<server_slug>.<base_domain>`.
/// Server slugs are only unique *within a workspace*, so the workspace slug (globally
/// unique) is required to route deterministically — the old single-label `<slug>` form is
/// intentionally no longer accepted. The full label is the cache key (also unique).
async fn resolve_server(state: &ProxyState, label: &str) -> Result<CachedServer, ProxyError> {
    tracing::debug!("resolve_server: looking up label={}", label);

    // Try Redis cache first (keyed by the full <workspace>--<server> label).
    let cache_start = Instant::now();
    if let Some(cached) = state.redis_cache.get_server(label).await {
        tracing::debug!("resolve_server: cache HIT for label={} (took {:?})", label, cache_start.elapsed());
        return Ok(cached);
    }
    tracing::debug!("resolve_server: cache MISS for label={} (took {:?})", label, cache_start.elapsed());

    // Split on the first `--`. New slugs forbid consecutive hyphens, so the first `--`
    // is unambiguously the workspace/server separator.
    let (workspace_slug, server_slug) = label.split_once("--").ok_or_else(|| {
        tracing::warn!("resolve_server: host label '{}' is not <workspace>--<server>", label);
        ProxyError::NotFound("Server not found".into())
    })?;

    // Cache miss - query database
    let db_start = Instant::now();
    tracing::debug!("resolve_server: querying database for workspace={} server={}", workspace_slug, server_slug);
    let server = ServerRepository::find_by_workspace_and_server_slug(&state.db, workspace_slug, server_slug)
        .await
        .map_err(|e| {
            tracing::error!("resolve_server: database error for label={} (took {:?}): {}", label, db_start.elapsed(), e);
            ProxyError::Internal(e.to_string())
        })?
        .ok_or_else(|| {
            tracing::warn!("resolve_server: server NOT found for label={} (took {:?})", label, db_start.elapsed());
            ProxyError::NotFound("Server not found".into())
        })?;

    tracing::info!("resolve_server: found server id={} for label={} (took {:?})", server.id, label, db_start.elapsed());

    // Cache the result (async, don't block), keyed by the full label.
    let redis_cache = state.redis_cache.clone();
    let server_clone = server.clone();
    let label_owned = label.to_string();
    tokio::spawn(async move {
        redis_cache.set_server(&label_owned, &server_clone).await;
    });

    Ok(CachedServer::from(&server))
}

/// Extracted MCP request info for scope checking and logging
#[derive(Debug, Clone)]
struct McpRequestInfo {
    method: McpMethod,
    method_str: Option<String>,
    target: Option<String>,
    /// The JSON-RPC request `id`, echoed back in error responses for correlation.
    id: Option<serde_json::Value>,
}

/// Extract MCP method and target from JSON-RPC request body
fn extract_mcp_request_info(body: &[u8]) -> McpRequestInfo {
    let mut info = McpRequestInfo {
        method: McpMethod::Unknown,
        method_str: None,
        target: None,
        id: None,
    };

    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) {
        info.id = json.get("id").cloned();
        if let Some(method_str) = json.get("method").and_then(|m| m.as_str()) {
            info.method_str = Some(method_str.to_string());
            info.method = McpMethod::parse(method_str);

            // Extract target based on method type
            match info.method {
                McpMethod::ToolsCall => {
                    // Extract tool name from params.name
                    info.target = json
                        .get("params")
                        .and_then(|p| p.get("name"))
                        .and_then(|n| n.as_str())
                        .map(String::from);
                }
                McpMethod::ResourcesRead => {
                    // Extract resource URI from params.uri
                    info.target = json
                        .get("params")
                        .and_then(|p| p.get("uri"))
                        .and_then(|u| u.as_str())
                        .map(String::from);
                }
                McpMethod::PromptsGet => {
                    // Extract prompt name from params.name
                    info.target = json
                        .get("params")
                        .and_then(|p| p.get("name"))
                        .and_then(|n| n.as_str())
                        .map(String::from);
                }
                _ => {}
            }
        }
    }

    info
}

/// Check if credential has permission for the MCP request
fn check_scope_permission(credential: &AuthCredential, mcp_info: &McpRequestInfo) -> Result<(), ProxyError> {
    // Unknown methods are allowed (forward compatibility)
    if matches!(mcp_info.method, McpMethod::Unknown) {
        return Ok(());
    }

    let allowed = credential.is_method_allowed(mcp_info.method, mcp_info.target.as_deref());

    if allowed {
        Ok(())
    } else {
        let scope_needed = match mcp_info.method {
            McpMethod::ToolsList => "tools:list or tools:*",
            McpMethod::ToolsCall => {
                if let Some(ref tool) = mcp_info.target {
                    return Err(ProxyError::Forbidden(format!(
                        "Access token lacks permission for tools:call:{} (need tools:call, tools:call:{}, or tools:*)",
                        tool, tool
                    )));
                }
                "tools:call or tools:*"
            }
            McpMethod::ResourcesList => "resources:list or resources:*",
            McpMethod::ResourcesRead => {
                if let Some(ref uri) = mcp_info.target {
                    return Err(ProxyError::Forbidden(format!(
                        "Access token lacks permission for resources:read:{} (need resources:read, resources:read:{}, or resources:*)",
                        uri, uri
                    )));
                }
                "resources:read or resources:*"
            }
            McpMethod::PromptsList => "prompts:list or prompts:*",
            McpMethod::PromptsGet => {
                if let Some(ref name) = mcp_info.target {
                    return Err(ProxyError::Forbidden(format!(
                        "Access token lacks permission for prompts:get:{} (need prompts:get, prompts:get:{}, or prompts:*)",
                        name, name
                    )));
                }
                "prompts:get or prompts:*"
            }
            McpMethod::Unknown => return Ok(()),
        };

        Err(ProxyError::Forbidden(format!(
            "Access token lacks required scope: {}",
            scope_needed
        )))
    }
}

/// Check if request is for SSE (Server-Sent Events)
fn is_sse_request(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/event-stream"))
        .unwrap_or(false)
}

/// Per-request context needed to build sanitized upstream headers and apply
/// tools/list optimizations.
struct ForwardContext<'a> {
    /// Whether NodeFlare validated the credential (so the client `Authorization`
    /// should be stripped) vs. the upstream doing its own auth (forward it).
    auth_enabled: bool,
    client_ip: &'a str,
    host: &'a str,
    /// Server whose tools/list we may filter/slim and whose tool catalog we update.
    server_id: Uuid,
    /// Filter tools/list down to tools the credential may call.
    filter_by_scope: bool,
    /// Trim verbose tool schemas in tools/list.
    slim: bool,
    /// Collapse tools/list into search_tools + call_tool meta-tools.
    search_mode: bool,
    /// Expose run_code + search_tools and execute code in a sandbox (takes precedence
    /// over search_mode when both are set).
    code_mode: bool,
}

/// Returns true if `name` is a hop-by-hop header (RFC 7230 §6.1).
///
/// `http::HeaderName::as_str()` always returns the lowercase canonical form, so a
/// plain `match` is sufficient — no case conversion needed.
#[inline]
fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// Build the sanitized header set to send upstream: drop hop-by-hop headers (and any
/// header named in the inbound `Connection` token list), strip the client
/// `Authorization` only when NodeFlare did the auth, and add `X-Forwarded-*`.
fn build_upstream_headers(inbound: &axum::http::HeaderMap, fwd: &ForwardContext) -> axum::http::HeaderMap {
    // `http::HeaderName::as_str()` is always lowercase, so we only lowercase the
    // *value* tokens from the Connection header, not the header names themselves.
    let mut conn_tokens = std::collections::HashSet::new();
    if let Some(conn) = inbound.get("connection").and_then(|v| v.to_str().ok()) {
        for tok in conn.split(',') {
            let t = tok.trim().to_ascii_lowercase();
            if !t.is_empty() {
                conn_tokens.insert(t);
            }
        }
    }

    let mut out = axum::http::HeaderMap::new();
    for (name, value) in inbound.iter() {
        // HeaderName::as_str() is always lowercase — no allocation needed.
        let n = name.as_str();
        // `host` is set by the HTTP client from the target URL; `x-forwarded-for` is
        // rebuilt below to append our hop.
        if n == "host" || n == "x-forwarded-for" {
            continue;
        }
        if is_hop_by_hop(n) {
            continue;
        }
        // O(1) HashSet lookup instead of O(conn_tokens.len()) linear scan.
        if conn_tokens.contains(n) {
            continue;
        }
        // Strip the client's Authorization only when we authenticated the request.
        // When the upstream does its own auth (auth_enabled=false), forward it.
        if n == "authorization" && fwd.auth_enabled {
            continue;
        }
        out.append(name.clone(), value.clone());
    }

    // X-Forwarded-For: append our observed client IP to any existing chain.
    let xff = match inbound.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        Some(existing) if !existing.trim().is_empty() => format!("{}, {}", existing, fwd.client_ip),
        _ => fwd.client_ip.to_string(),
    };
    if let Ok(v) = HeaderValue::from_str(&xff) {
        out.insert("x-forwarded-for", v);
    }
    if let Ok(v) = HeaderValue::from_str(fwd.host) {
        out.insert("x-forwarded-host", v);
    }
    out.insert("x-forwarded-proto", HeaderValue::from_static("https"));

    out
}

/// Caller-identity component of the response cache key.
///
/// Cached list responses (tools/list, resources/list, prompts/list) can vary by who
/// is asking: an upstream MCP server may filter tools by the caller's OAuth scope,
/// and our own future per-credential filtering will too. The cache key must therefore
/// include the caller's authorization context — otherwise one caller's cached (or
/// in-flight coalesced) list could be served to another, an information leak.
///
/// - Authenticated by NodeFlare: we strip the client `Authorization` before
///   forwarding, so the upstream is credential-agnostic today. We still key by our
///   credential id so distinct credentials never share an entry — correct now, and
///   still correct once we filter the list per credential.
/// - Pass-through (upstream does its own auth): the client's credentials are
///   forwarded, so the list can differ per caller. Key on the forwarded
///   credential-bearing headers. (Localization headers and exotic custom auth headers
///   are intentionally not included — a known long-tail limitation, bounded by TTL.)
fn cache_identity(credential: Option<&AuthCredential>, inbound_headers: &axum::http::HeaderMap) -> Vec<u8> {
    if let Some(cred) = credential {
        return cred.id().as_bytes().to_vec();
    }
    let mut identity = Vec::new();
    for name in ["authorization", "cookie"] {
        if let Some(value) = inbound_headers.get(name) {
            identity.extend_from_slice(name.as_bytes());
            identity.push(b'=');
            identity.extend_from_slice(value.as_bytes());
            identity.push(b'\n');
        }
    }
    identity
}

/// Canonical cache-key body: strip the volatile JSON-RPC `id` and `jsonrpc` fields so
/// two otherwise-identical requests (differing only by id) share one cache entry.
/// (serde_json's Value map orders keys, so the encoding is stable.)
fn jsonrpc_cache_key_body(body: &[u8]) -> Vec<u8> {
    if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(obj) = v.as_object_mut() {
            obj.remove("id");
            obj.remove("jsonrpc");
            if let Ok(s) = serde_json::to_vec(&v) {
                return s;
            }
        }
    }
    body.to_vec()
}

/// Strip the `id` from a JSON-RPC response so the cached copy is id-agnostic.
fn strip_jsonrpc_id(body: &[u8]) -> Vec<u8> {
    if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(obj) = v.as_object_mut() {
            if obj.remove("id").is_some() {
                if let Ok(s) = serde_json::to_vec(&v) {
                    return s;
                }
            }
        }
    }
    body.to_vec()
}

/// Re-inject the caller's JSON-RPC `id` into a cached (id-stripped) response body.
fn inject_jsonrpc_id(body: &[u8], id: Option<&serde_json::Value>) -> Vec<u8> {
    if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(obj) = v.as_object_mut() {
            obj.insert(
                "id".to_string(),
                id.cloned().unwrap_or(serde_json::Value::Null),
            );
            if let Ok(s) = serde_json::to_vec(&v) {
                return s;
            }
        }
    }
    body.to_vec()
}

/// Forward a request to the upstream MCP server.
///
/// `credential` is `Some` when NodeFlare auth is enabled (scope checks apply) and
/// `None` when the upstream handles its own auth. This single function replaces the
/// previously-duplicated `forward_request` / `forward_request_no_auth`.
async fn forward_request(
    state: &ProxyState,
    target_url: &str,
    request: Request<Body>,
    credential: Option<&AuthCredential>,
    fwd: &ForwardContext<'_>,
) -> Result<(Response, McpRequestInfo), ProxyError> {
    let method = request.method().clone();
    let inbound_headers = request.headers().clone();
    // GET requests to the MCP endpoint always return an SSE stream from the adapter,
    // regardless of whether the client sends Accept: text/event-stream.
    let is_sse = method == axum::http::Method::GET || is_sse_request(&inbound_headers);

    // Sanitize headers once (hop-by-hop strip, Authorization policy, X-Forwarded-*).
    let headers = build_upstream_headers(&inbound_headers, fwd);

    // Read body (limit comes from configuration, not a hard-coded constant).
    let mut body_bytes = axum::body::to_bytes(request.into_body(), state.body_limit)
        .await
        .map_err(|e| ProxyError::BadRequest(format!("Failed to read body: {}", e)))?;

    tracing::info!("forward_request: method={}, is_sse={}, auth={}", method, is_sse, fwd.auth_enabled);
    // Request body may contain sensitive data (tool args, secrets, PII) — log only at DEBUG
    if tracing::enabled!(tracing::Level::DEBUG) {
        let req_body_preview = if body_bytes.len() > 500 {
            format!("{}... (truncated)", String::from_utf8_lossy(&body_bytes[..500]))
        } else {
            String::from_utf8_lossy(&body_bytes).to_string()
        };
        tracing::debug!("forward_request: request_body={}", req_body_preview);
    }

    // Extract MCP request info (method + target + id)
    let mut mcp_info = extract_mcp_request_info(&body_bytes);
    tracing::info!(
        "mcp request: method={} target={} search_mode={} code_mode={} server={}",
        mcp_info.method_str.as_deref().unwrap_or("-"),
        mcp_info.target.as_deref().unwrap_or("-"),
        fwd.search_mode,
        fwd.code_mode,
        fwd.server_id
    );

    // Search/code mode: handle the synthetic meta-tools before scope checks / forwarding.
    // Route by handler_type so the exposed name can be changed in the DB without code changes.
    let target = mcp_info.target.clone();
    if (fwd.search_mode || fwd.code_mode) && matches!(mcp_info.method, McpMethod::ToolsCall) {
        let handler = target.as_deref().and_then(|name| {
            state.meta_tools.iter().find(|m| m.name == name).map(|m| m.handler_type.as_str())
        });
        match handler {
            // `search_tools` is served locally from the tool catalog — never forwarded.
            // Available in both search mode and code mode (for discovery).
            Some("search_tools") => {
                let query = meta_tools::extract_search_query(&body_bytes);
                let response =
                    search_tools_response(state, credential, fwd, &query, mcp_info.id.as_ref()).await;
                return Ok((response, mcp_info));
            }
            // `run_code` (code mode): execute in the sandbox; never forwarded.
            Some("run_code") if fwd.code_mode => {
                let response =
                    run_code_response(state, fwd, credential, target_url, &body_bytes, mcp_info.id.as_ref())
                        .await;
                return Ok((response, mcp_info));
            }
            // `call_tool` (search mode): unwrap into a real tools/call, then flow through
            // the normal path below (scope-checked against the real tool, then forwarded).
            Some("call_tool") if fwd.search_mode => {
                if let Some(rewritten) = meta_tools::rewrite_call_tool_body(&body_bytes) {
                    body_bytes = Bytes::from(rewritten);
                    mcp_info = extract_mcp_request_info(&body_bytes);
                }
            }
            _ => {}
        }
    }

    // Check scope permission before forwarding (only when we did the auth).
    if let Some(cred) = credential {
        if let Err(e) = check_scope_permission(cred, &mcp_info) {
            // Echo the inbound request id so the client can correlate the error.
            return Ok((error_response_with_id(e, mcp_info.id.as_ref()), mcp_info));
        }
    }

    // tools/list gets token-reduction transforms (scope filter + optional slim) and
    // populates the server's tool catalog, so it must be buffered rather than streamed.
    let is_tools_list = matches!(mcp_info.method, McpMethod::ToolsList);

    // SSE requests: stream directly (no buffering, minimal latency) — except tools/list,
    // which we buffer below to transform it. tools/list is request/response, not a
    // long-lived stream, so buffering it is safe.
    if is_sse && !is_tools_list {
        tracing::info!("SSE request detected, using streaming forward to {}", target_url);
        let response = execute_streaming_request(state, target_url, method, &headers, body_bytes).await?;
        return Ok((response, mcp_info));
    }

    // tools/list over SSE: buffer (don't cache — the JSON-RPC id lives inside the SSE
    // `data:` frame, which our id-agnostic cache can't rewrite), then catalog + transform.
    if is_sse && is_tools_list {
        let (response_body, status, response_headers) =
            execute_upstream_request(state, target_url, method, &headers, body_bytes).await?;
        if status >= 200 && status < 300 {
            spawn_catalog_update(state, fwd.server_id, &response_headers, &response_body);
            // Persist to Redis for predictive wake (id-stripped so any caller can use it).
            let stripped = strip_jsonrpc_id(&response_body);
            let rc = state.redis_cache.clone();
            let sid = fwd.server_id;
            tokio::spawn(async move { rc.set_tools_list(&sid, &stripped).await; });
        }
        let body = transform_list_body(&mcp_info, &response_headers, response_body, credential, fwd, &state.meta_tools);
        let response = build_response(status, &response_headers, body)?;
        return Ok((response, mcp_info));
    }

    // Only cache read-only MCP methods (list operations)
    let is_cacheable = matches!(
        mcp_info.method,
        McpMethod::ToolsList | McpMethod::ResourcesList | McpMethod::PromptsList
    );

    // Try request coalescing + caching for cacheable requests.
    // The cache is keyed on an id-stripped body and stored without an id, so cache
    // hits never leak another caller's JSON-RPC id — we re-inject the caller's id.
    if is_cacheable {
        let key_body = jsonrpc_cache_key_body(&body_bytes);
        let identity = cache_identity(credential, &inbound_headers);
        match state.request_cache.try_coalesce(target_url, &identity, &key_body).await {
            CoalesceResult::Cached(cached) => {
                tracing::debug!("Cache hit for {}", target_url);
                let body = inject_jsonrpc_id(&cached.body, mcp_info.id.as_ref());
                let body = transform_list_body(&mcp_info, &cached.headers, body, credential, fwd, &state.meta_tools);
                let response = build_response(cached.status, &cached.headers, body)?;
                return Ok((response, mcp_info));
            }
            CoalesceResult::Coalesced(cached) => {
                tracing::debug!("Request coalesced for {}", target_url);
                let body = inject_jsonrpc_id(&cached.body, mcp_info.id.as_ref());
                let body = transform_list_body(&mcp_info, &cached.headers, body, credential, fwd, &state.meta_tools);
                let response = build_response(cached.status, &cached.headers, body)?;
                return Ok((response, mcp_info));
            }
            CoalesceResult::Execute(handle) => {
                // Execute the request and cache the result
                match execute_upstream_request(state, target_url, method, &headers, body_bytes).await {
                    Ok((response_body, status, response_headers)) => {
                        // Cache successful responses only, storing an id-stripped copy.
                        // The cached body is the RAW upstream response; per-caller
                        // transforms are applied on egress below, so the cache stays
                        // reusable and never stores a filtered/slimmed copy.
                        if status >= 200 && status < 300 {
                            spawn_catalog_update(state, fwd.server_id, &response_headers, &response_body);
                            let cacheable = strip_jsonrpc_id(&response_body);
                            // Persist to Redis for predictive wake (survives proxy restarts).
                            let rc = state.redis_cache.clone();
                            let sid = fwd.server_id;
                            let redis_body = cacheable.clone();
                            tokio::spawn(async move { rc.set_tools_list(&sid, &redis_body).await; });
                            state.request_cache.complete(handle, cacheable, status, response_headers.clone()).await;
                        } else {
                            state.request_cache.cancel(handle).await;
                        }

                        let body = transform_list_body(&mcp_info, &response_headers, response_body, credential, fwd, &state.meta_tools);
                        let response = build_response(status, &response_headers, body)?;
                        return Ok((response, mcp_info));
                    }
                    Err(e) => {
                        state.request_cache.cancel(handle).await;
                        return Err(e);
                    }
                }
            }
        }
    }

    // Non-cacheable requests: execute directly
    let (response_body, status, response_headers) =
        execute_upstream_request(state, target_url, method, &headers, body_bytes).await?;

    let response = build_response(status, &response_headers, response_body)?;
    Ok((response, mcp_info))
}

/// Content-type from a response header list, if present.
fn response_content_type(headers: &[(String, String)]) -> Option<&str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.as_str())
}

/// Apply tools/list token-reduction transforms to an outgoing response body.
/// No-op for any method other than tools/list, and when neither transform is active.
fn transform_list_body(
    mcp_info: &McpRequestInfo,
    headers: &[(String, String)],
    body: Vec<u8>,
    credential: Option<&AuthCredential>,
    fwd: &ForwardContext<'_>,
    db_meta_tools: &[mcp_db::ProxyMetaTool],
) -> Vec<u8> {
    if !matches!(mcp_info.method, McpMethod::ToolsList) {
        return body;
    }
    let content_type = response_content_type(headers);
    // Code mode (takes precedence) exposes run_code + search_tools.
    if fwd.code_mode {
        return mcp_transform::replace_tools(&body, content_type, &code_mode::definitions_from_db(db_meta_tools));
    }
    // Search mode replaces the whole tool surface with the two meta-tools.
    if fwd.search_mode {
        return mcp_transform::replace_tools(&body, content_type, &meta_tools::definitions_from_db(db_meta_tools));
    }
    // Filtering only happens when we authenticated the caller (credential present);
    // skip the work entirely when nothing would change.
    let will_filter = fwd.filter_by_scope && credential.is_some();
    if !will_filter && !fwd.slim {
        return body;
    }
    mcp_transform::transform_tools_list(&body, content_type, credential, fwd.filter_by_scope, fwd.slim)
}

/// Serve a `search_tools` call locally from the server's tool catalog. Uses semantic
/// (embedding) search when available, falling back to lexical search; honors scope
/// filtering so a credential only discovers tools it is allowed to call.
async fn search_tools_response(
    state: &ProxyState,
    credential: Option<&AuthCredential>,
    fwd: &ForwardContext<'_>,
    query: &str,
    id: Option<&serde_json::Value>,
) -> Response {
    let matched = search_catalog(state, credential, fwd, query).await;
    // In code mode the model needs a code API to write against, so return TS signatures;
    // otherwise return the tool list as JSON for direct call_tool use.
    let body = if fwd.code_mode {
        code_mode::result_json(&code_mode::generate_ts_api(&matched), id)
    } else {
        let refs: Vec<&Tool> = matched.iter().collect();
        meta_tools::search_result_json(&refs, id)
    };
    let headers = vec![("content-type".to_string(), "application/json".to_string())];
    build_response(200, &headers, body)
        .unwrap_or_else(|_| Response::builder().status(500).body(Body::empty()).unwrap())
}

/// Execute a `run_code` call in the sandboxed runner and wrap the result. Issues a
/// short-lived exec token so the sandbox can call tools back through the scope-enforced
/// endpoint. Degrades to an error result when no runner is configured or unauthenticated.
async fn run_code_response(
    state: &ProxyState,
    fwd: &ForwardContext<'_>,
    credential: Option<&AuthCredential>,
    target_url: &str,
    body: &[u8],
    id: Option<&serde_json::Value>,
) -> Response {
    let respond = |bytes: Vec<u8>| -> Response {
        let headers = vec![("content-type".to_string(), "application/json".to_string())];
        build_response(200, &headers, bytes)
            .unwrap_or_else(|_| Response::builder().status(500).body(Body::empty()).unwrap())
    };

    tracing::info!("run_code: invoked for server={}", fwd.server_id);
    let Some(runner) = &state.code_runner else {
        tracing::warn!("run_code: rejected — no code runner configured (PROXY_CODE_RUNNER_URL)");
        return respond(code_mode::error_json(
            "Code execution is enabled for this server but no code runner is configured.",
            id,
        ));
    };
    // Code mode relies on scopes to limit what the sandbox can call back through the
    // internal tool-call endpoint. For auth-disabled servers every tool is public, so
    // grant wildcard scope instead of blocking execution entirely.
    let scopes = match credential {
        Some(cred) => cred.scopes(),
        None if !fwd.auth_enabled => vec!["*".to_string()],
        None => {
            tracing::warn!("run_code: rejected — unauthenticated (code mode requires NodeFlare auth)");
            return respond(code_mode::error_json(
                "Code execution requires NodeFlare authentication.",
                id,
            ));
        }
    };
    let Some(code) = code_mode::extract_code(body) else {
        tracing::warn!("run_code: rejected — missing 'code' argument");
        return respond(code_mode::error_json("run_code requires a 'code' argument.", id));
    };

    // Issue a short-lived token carrying the authorization for the sandbox's tool calls.
    let token = Uuid::new_v4().to_string();
    let ctx = CodeExecContext {
        server_id: fwd.server_id,
        target_url: target_url.to_string(),
        scopes,
    };
    // TTL covers the execution wall-clock plus a small buffer.
    let ttl = runner.timeout_secs() as i64 + 10;
    state.redis_cache.set_code_exec(&token, &ctx, ttl).await;
    let endpoint = runner.tools_endpoint(fwd.server_id);
    tracing::info!(
        "run_code: dispatching to runner (code={} bytes, scopes={}, tools_endpoint={}, timeout={}s, max_tool_calls={})",
        code.len(),
        ctx.scopes.len(),
        endpoint,
        runner.timeout_secs(),
        runner.max_tool_calls()
    );

    let req = code_runner::RunRequest {
        code,
        token,
        tools_endpoint: endpoint,
        timeout_secs: runner.timeout_secs(),
        max_tool_calls: runner.max_tool_calls(),
    };
    let started = Instant::now();
    let id_owned = id.cloned();
    let runner = runner.clone();

    // Stream SSE keepalives while the runner executes, so the client does not
    // time out waiting for a long-running tool (e.g. nuclei full scan).
    let stream = async_stream::stream! {
        let run_fut = runner.run(req);
        tokio::pin!(run_fut);
        let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(15));
        keepalive.tick().await; // consume the immediate first tick
        loop {
            tokio::select! {
                result = &mut run_fut => {
                    let payload = match result {
                        Ok(output) => {
                            tracing::info!("run_code: success ({} bytes) in {:?}", output.len(), started.elapsed());
                            code_mode::result_json(&output, id_owned.as_ref())
                        }
                        Err(e) => {
                            tracing::warn!("run_code: FAILED in {:?}: {}", started.elapsed(), e);
                            code_mode::error_json(&format!("code execution failed: {e}"), id_owned.as_ref())
                        }
                    };
                    let line = format!("event: message\ndata: {}\n\n",
                        String::from_utf8_lossy(&payload));
                    yield Ok::<_, std::convert::Infallible>(Bytes::from(line));
                    break;
                }
                _ = keepalive.tick() => {
                    yield Ok::<_, std::convert::Infallible>(Bytes::from_static(b": keepalive\n\n"));
                }
            }
        }
    };
    Response::builder()
        .status(200)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| Response::builder().status(500).body(Body::empty()).unwrap())
}

/// Internal endpoint the sandbox calls to invoke a tool (the security boundary).
/// Validates the exec token, re-checks scope, and forwards a tools/call upstream.
async fn code_exec_tools_call(
    State(state): State<Arc<ProxyState>>,
    Path(server_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // 1. Bearer token → context.
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let Some(token) = token else {
        tracing::warn!("code-exec[{}]: missing bearer token", server_id);
        return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
    };
    // get_code_exec reads without deleting: the same token must be valid for all
    // tool calls within a single code execution (up to max_tool_calls, same session).
    // Replay protection comes from UUID v4 entropy + short TTL (timeout_secs+10s).
    let Some(ctx) = state.redis_cache.get_code_exec(token).await else {
        tracing::warn!("code-exec[{}]: invalid or expired token", server_id);
        return (StatusCode::UNAUTHORIZED, "invalid or expired token").into_response();
    };
    // The token is bound to one server; reject a mismatched path.
    if ctx.server_id != server_id {
        tracing::warn!(
            "code-exec[{}]: token/server mismatch (token bound to {})",
            server_id,
            ctx.server_id
        );
        return (StatusCode::FORBIDDEN, "token/server mismatch").into_response();
    }

    // 2. Parse { tool, arguments }.
    let parsed: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("code-exec[{}]: invalid JSON body", server_id);
            return (StatusCode::BAD_REQUEST, "invalid JSON body").into_response();
        }
    };
    let Some(tool) = parsed.get("tool").and_then(|t| t.as_str()) else {
        tracing::warn!("code-exec[{}]: missing 'tool' in body", server_id);
        return (StatusCode::BAD_REQUEST, "missing 'tool'").into_response();
    };
    let arguments = parsed
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
    // Log the exact arguments forwarded to the upstream tool (truncated), so we can see
    // whether code mode passes the same args a direct client would.
    let args_preview = {
        let s = arguments.to_string();
        if s.len() > 600 {
            // Truncate on a char boundary (args may contain multi-byte UTF-8).
            let mut end = 600;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}… ({}B)", &s[..end], s.len())
        } else {
            s
        }
    };
    tracing::info!(
        "code-exec[{}]: tool-call '{}' args={}",
        server_id,
        tool,
        args_preview
    );

    // 3. Re-check scope from the token (NOT the sandbox wrapper — this is the boundary).
    let checker = ScopeChecker::new(&ctx.scopes);
    if !checker.is_allowed(McpMethod::ToolsCall, Some(tool)) {
        tracing::warn!("code-exec[{}]: scope DENIED for tool '{}'", server_id, tool);
        return (
            StatusCode::FORBIDDEN,
            format!("scope does not permit tools:call:{}", tool),
        )
            .into_response();
    }

    // 4. Forward a normal tools/call upstream (request JSON, not SSE, so we can parse it).
    let rpc = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments }
    });
    let rpc_bytes = match serde_json::to_vec(&rpc) {
        Ok(b) => Bytes::from(b),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "serialize").into_response(),
    };
    let mut up_headers = HeaderMap::new();
    up_headers.insert("content-type", HeaderValue::from_static("application/json"));
    up_headers.insert("accept", HeaderValue::from_static("application/json"));

    match execute_upstream_request(
        &state,
        &ctx.target_url,
        axum::http::Method::POST,
        &up_headers,
        rpc_bytes,
    )
    .await
    {
        Ok((resp_body, status, _)) => {
            match serde_json::from_slice::<serde_json::Value>(&resp_body) {
                Ok(v) => {
                    // A JSON-RPC `error` is a TOOL-level failure, not an infrastructure
                    // one. Return it to the sandbox as a catchable `__toolError` (HTTP
                    // 200) so the model sees the real error and can try/catch it — NOT a
                    // 502, which the sandbox throws opaquely and which looks like the
                    // whole server is down.
                    if let Some(err) = v.get("error") {
                        tracing::info!(
                            "code-exec[{}]: tool '{}' returned JSON-RPC error: {}",
                            server_id,
                            tool,
                            err
                        );
                        return (
                            StatusCode::OK,
                            axum::Json(serde_json::json!({ "__toolError": err })),
                        )
                            .into_response();
                    }
                    tracing::info!(
                        "code-exec[{}]: tool '{}' OK (upstream {}, {} bytes)",
                        server_id,
                        tool,
                        status,
                        resp_body.len()
                    );
                    let result = v.get("result").cloned().unwrap_or(serde_json::Value::Null);
                    (StatusCode::OK, axum::Json(result)).into_response()
                }
                // Non-JSON (e.g. SSE) upstream: pass through raw with its status.
                Err(_) => {
                    tracing::info!(
                        "code-exec[{}]: tool '{}' non-JSON upstream (status {}, {} bytes), passing through",
                        server_id,
                        tool,
                        status,
                        resp_body.len()
                    );
                    build_response(
                        status,
                        &[("content-type".to_string(), "application/json".to_string())],
                        resp_body,
                    )
                    .unwrap_or_else(|_| (StatusCode::BAD_GATEWAY, "bad upstream").into_response())
                }
            }
        }
        // Transport failure to the upstream: also surface as a catchable error so the
        // model gets a message instead of an opaque 502.
        Err(e) => {
            tracing::warn!("code-exec[{}]: tool '{}' upstream transport error: {}", server_id, tool, e);
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({ "__toolError": format!("upstream error: {e}") })),
            )
                .into_response()
        }
    }
}

/// Keep only tools the credential is allowed to call (when scope filtering is on).
fn scope_filter_tools(
    tools: Vec<Tool>,
    credential: Option<&AuthCredential>,
    fwd: &ForwardContext<'_>,
) -> Vec<Tool> {
    match (fwd.filter_by_scope, credential) {
        (true, Some(cred)) => tools
            .into_iter()
            .filter(|t| cred.is_method_allowed(McpMethod::ToolsCall, Some(&t.name)))
            .collect(),
        _ => tools,
    }
}

/// Find catalog tools matching `query`: semantic search first (if embeddings are
/// configured and the query is non-empty), otherwise lexical. Always scope-filtered.
async fn search_catalog(
    state: &ProxyState,
    credential: Option<&AuthCredential>,
    fwd: &ForwardContext<'_>,
    query: &str,
) -> Vec<Tool> {
    let limit = meta_tools::search_limit();

    // Semantic path.
    if let Some(client) = &state.embedding {
        if !query.trim().is_empty() {
            if let Some(qvec) = client.embed(query).await {
                // Over-fetch so scope filtering still leaves enough results.
                let fetch = (limit * 4) as i64;
                match ToolRepository::search_by_embedding(&state.db, fwd.server_id, qvec, fetch).await {
                    Ok(tools) if !tools.is_empty() => {
                        let filtered = scope_filter_tools(tools, credential, fwd);
                        if !filtered.is_empty() {
                            let out: Vec<Tool> = filtered.into_iter().take(limit).collect();
                            tracing::info!(
                                "search_tools[{}]: SEMANTIC hit — query='{}' -> {} tools",
                                fwd.server_id,
                                query,
                                out.len()
                            );
                            return out;
                        }
                        tracing::info!("search_tools[{}]: semantic matched but scope-filtered to 0, falling back", fwd.server_id);
                    }
                    Ok(_) => {
                        tracing::info!("search_tools[{}]: no embeddings yet, falling back to lexical", fwd.server_id);
                    }
                    Err(e) => tracing::warn!("search_tools[{}]: semantic search failed: {}", fwd.server_id, e),
                }
            } else {
                tracing::warn!("search_tools[{}]: query embedding failed, falling back to lexical", fwd.server_id);
            }
        }
    }

    // Lexical fallback.
    let tools = ToolRepository::list_by_server(&state.db, fwd.server_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("search_tools: catalog read failed for {}: {}", fwd.server_id, e);
            Vec::new()
        });
    let visible = scope_filter_tools(tools, credential, fwd);
    let out: Vec<Tool> = meta_tools::rank_tools(&visible, query, limit)
        .into_iter()
        .cloned()
        .collect();
    tracing::info!(
        "search_tools[{}]: LEXICAL — query='{}' -> {} tools",
        fwd.server_id,
        query,
        out.len()
    );
    out
}

/// Best-effort, non-blocking refresh of the server's tool catalog from an observed
/// tools/list response. For servers whose list varies by caller (e.g. OAuth scope in
/// pass-through mode) this records the most-recently observed set. An empty list is
/// ignored so a low-scope caller can't wipe a populated catalog.
fn spawn_catalog_update(
    state: &ProxyState,
    server_id: Uuid,
    headers: &[(String, String)],
    body: &[u8],
) {
    let content_type = response_content_type(headers);
    let Some(tools) = mcp_transform::extract_tools(body, content_type) else {
        return;
    };
    if tools.is_empty() {
        return;
    }
    let db = state.db.clone();
    let embedding = state.embedding.clone();
    tokio::spawn(async move {
        let upserts: Vec<UpsertTool> = tools
            .into_iter()
            .map(|t| UpsertTool {
                server_id,
                name: t.name,
                description: t.description,
                input_schema: t.input_schema,
            })
            .collect();
        let n = upserts.len();
        if let Err(e) = ToolRepository::sync_tools(&db, server_id, upserts).await {
            tracing::warn!("tool catalog sync failed for server {}: {}", server_id, e);
            return;
        }
        tracing::info!("catalog[{}]: synced {} tools from tools/list", server_id, n);
        // Backfill embeddings for new/changed tools (sync nulls embeddings on change).
        if let Some(client) = embedding {
            backfill_embeddings(&db, &client, server_id).await;
        }
    });
}

/// Text used to embed a tool: name plus description (when present).
fn tool_embed_text(tool: &Tool) -> String {
    match &tool.description {
        Some(desc) if !desc.is_empty() => format!("{}: {}", tool.name, desc),
        _ => tool.name.clone(),
    }
}

/// Embed and store vectors for a server's tools that don't have one yet. Best-effort.
async fn backfill_embeddings(
    db: &mcp_db::DbPool,
    client: &embedding::EmbeddingClient,
    server_id: Uuid,
) {
    let missing = match ToolRepository::list_missing_embeddings(db, server_id).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("list_missing_embeddings failed for {}: {}", server_id, e);
            return;
        }
    };
    if missing.is_empty() {
        return;
    }
    tracing::info!("embeddings[{}]: backfilling {} tools", server_id, missing.len());
    let texts: Vec<String> = missing.iter().map(tool_embed_text).collect();
    let Some(vectors) = client.embed_batch(&texts).await else {
        tracing::warn!("embeddings[{}]: batch embed failed (Gemini)", server_id);
        return;
    };
    let mut stored = 0usize;
    for (tool, vector) in missing.iter().zip(vectors.into_iter()) {
        if let Some(vector) = vector {
            match ToolRepository::set_embedding(db, tool.id, vector).await {
                Ok(_) => stored += 1,
                Err(e) => tracing::warn!("set_embedding failed for tool {}: {}", tool.id, e),
            }
        }
    }
    tracing::info!("embeddings[{}]: stored {}/{}", server_id, stored, missing.len());
}

/// Build response from raw parts
fn build_response(status: u16, headers: &[(String, String)], body: Vec<u8>) -> Result<Response, ProxyError> {
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
        .body(Body::from(body))
        .map_err(|e| ProxyError::Internal(e.to_string()))
}

/// Execute request to upstream MCP server
async fn execute_upstream_request(
    state: &ProxyState,
    target_url: &str,
    method: axum::http::Method,
    headers: &axum::http::HeaderMap,
    body_bytes: Bytes,
) -> Result<(Vec<u8>, u16, Vec<(String, String)>), ProxyError> {
    // Route unix: URLs to the Unix Domain Socket client.
    if let Some((socket_path, http_path)) = unix_client::parse_target(target_url) {
        return unix_client::send_buffered(
            socket_path, http_path, method, headers, body_bytes,
            state.max_upstream_response_bytes,
        ).await;
    }

    // Build outgoing request. `headers` is already sanitized by build_upstream_headers
    // (hop-by-hop stripped, Authorization policy applied, X-Forwarded-* added).
    let mut req_builder = state.http_client.request(method, target_url);
    for (name, value) in headers.iter() {
        req_builder = req_builder.header(name, value);
    }

    req_builder = req_builder.body(body_bytes);

    // Send request
    let mut response = req_builder
        .send()
        .await
        .map_err(|e| ProxyError::ServiceUnavailable(format!("Upstream error: {}", e)))?;

    // Convert response
    let status = response.status().as_u16();

    // Only preserve essential headers to reduce allocations
    // Most headers (connection, transfer-encoding, etc.) are handled by the framework
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            // Only keep headers that are meaningful for the response
            let name = k.as_str();
            match name {
                "content-type" | "content-encoding" | "cache-control" | "etag" | "vary" | "x-request-id" | "mcp-session-id" => {
                    v.to_str().ok().map(|val| (name.to_string(), val.to_string()))
                }
                _ => None,
            }
        })
        .collect();

    // SECURITY (DoS): the upstream is tenant-controlled, so bound how much we buffer.
    // Reject early on a too-large Content-Length, then enforce again while streaming
    // (chunked replies may omit or understate the length).
    let max_bytes = state.max_upstream_response_bytes;
    if let Some(len) = response.content_length() {
        if len as usize > max_bytes {
            return Err(ProxyError::ServiceUnavailable(format!(
                "Upstream response too large: {} bytes (limit {})",
                len, max_bytes
            )));
        }
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ProxyError::Internal(format!("Failed to read response: {}", e)))?
    {
        if buf.len() + chunk.len() > max_bytes {
            return Err(ProxyError::ServiceUnavailable(format!(
                "Upstream response exceeded {} byte limit",
                max_bytes
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    let body = Bytes::from(buf);

    tracing::info!("upstream response: status={}", status);
    // Response body may contain sensitive data — log only at DEBUG
    if tracing::enabled!(tracing::Level::DEBUG) {
        let body_preview = if body.len() > 500 {
            format!("{}... (truncated, total {} bytes)", String::from_utf8_lossy(&body[..500]), body.len())
        } else {
            String::from_utf8_lossy(&body).to_string()
        };
        tracing::debug!("upstream response: body={}", body_preview);
    }

    Ok((body.to_vec(), status, headers))
}

/// Execute streaming request for SSE (Server-Sent Events)
/// Streams response directly without buffering for minimal latency
async fn execute_streaming_request(
    state: &ProxyState,
    target_url: &str,
    method: axum::http::Method,
    headers: &axum::http::HeaderMap,
    body_bytes: Bytes,
) -> Result<Response, ProxyError> {
    // Route unix: URLs to the Unix Domain Socket client.
    if let Some((socket_path, http_path)) = unix_client::parse_target(target_url) {
        return unix_client::send_streaming(socket_path, http_path, method, headers, body_bytes).await;
    }

    // Reuse the shared, long-lived SSE client (built once at startup with a
    // connect_timeout + read/idle timeout and no total timeout) so connections are
    // pooled across requests instead of rebuilt per request.
    let mut req_builder = state.sse_client.request(method, target_url);

    // `headers` is already sanitized (hop-by-hop stripped, Authorization policy
    // applied, X-Forwarded-* added), and still carries Accept for SSE.
    for (name, value) in headers.iter() {
        req_builder = req_builder.header(name, value);
    }

    req_builder = req_builder.body(body_bytes);

    // Send request
    let response = req_builder
        .send()
        .await
        .map_err(|e| ProxyError::ServiceUnavailable(format!("Upstream error: {}", e)))?;

    let status = response.status();

    // Build response with streaming body
    let mut builder = Response::builder().status(status);

    // Copy essential headers from upstream - don't override content-type!
    let mut has_content_type = false;
    for (name, value) in response.headers().iter() {
        let header_name = name.as_str();
        match header_name {
            "content-type" => {
                if let Ok(val) = value.to_str() {
                    builder = builder.header(header_name, val);
                    has_content_type = true;
                    tracing::debug!("SSE streaming: preserving upstream content-type: {}", val);
                }
            }
            "content-encoding" | "cache-control" | "x-request-id" | "mcp-session-id" | "x-accel-buffering" => {
                if let Ok(val) = value.to_str() {
                    builder = builder.header(header_name, val);
                }
            }
            _ => {}
        }
    }

    // Only set default headers if upstream didn't provide them
    if !has_content_type {
        builder = builder.header("content-type", "text/event-stream");
    }
    builder = builder.header("cache-control", "no-cache");
    builder = builder.header("x-accel-buffering", "no"); // Disable proxy buffering

    // Stream the response body directly without buffering
    let stream = response.bytes_stream().map(|result| {
        result.map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
        })
    });

    let body = Body::from_stream(stream);

    builder
        .body(body)
        .map_err(|e| ProxyError::Internal(e.to_string()))
}

/// Extract server slug from subdomain
/// e.g., "my-server.mcp.cloud" with base "mcp.cloud" -> "my-server"
/// e.g., "my-server.localhost:8081" with base "localhost:8081" -> "my-server"
fn extract_subdomain(host: &str, base_domain: &str) -> Result<String, ProxyError> {
    // Remove port from host if present for comparison
    let host_without_port = host.split(':').next().unwrap_or(host);
    let base_without_port = base_domain.split(':').next().unwrap_or(base_domain);

    // Check if this is a subdomain of the base domain
    if let Some(subdomain) = host_without_port.strip_suffix(&format!(".{}", base_without_port)) {
        if subdomain.is_empty() || subdomain.contains('.') {
            return Err(ProxyError::BadRequest("Invalid subdomain format".into()));
        }
        Ok(subdomain.to_string())
    } else if host_without_port == base_without_port {
        // Direct access to base domain (e.g., mcp.cloud without subdomain)
        Err(ProxyError::BadRequest(
            "No server specified. Use {server-slug}.{base-domain}".into(),
        ))
    } else {
        Err(ProxyError::BadRequest(format!(
            "Invalid host: expected *.{}",
            base_domain
        )))
    }
}

#[derive(Debug)]
pub enum ProxyError {
    Unauthorized(String, Option<String>), // message, optional host for WWW-Authenticate
    Forbidden(String),
    NotFound(String),
    BadRequest(String),
    RateLimitExceeded,
    QuotaExceeded(String),
    PaymentRequired(String),
    ServiceUnavailable(String),
    Internal(String),
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyError::Unauthorized(m, _) => write!(f, "Unauthorized: {}", m),
            ProxyError::Forbidden(m) => write!(f, "Forbidden: {}", m),
            ProxyError::NotFound(m) => write!(f, "Not found: {}", m),
            ProxyError::BadRequest(m) => write!(f, "Bad request: {}", m),
            ProxyError::RateLimitExceeded => write!(f, "Rate limit exceeded"),
            ProxyError::QuotaExceeded(m) => write!(f, "Quota exceeded: {}", m),
            ProxyError::PaymentRequired(m) => write!(f, "Payment required: {}", m),
            ProxyError::ServiceUnavailable(m) => write!(f, "Service unavailable: {}", m),
            ProxyError::Internal(m) => write!(f, "Internal error: {}", m),
        }
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        // No request context here, so the id can't be correlated; callers that know
        // the inbound JSON-RPC id use `error_response_with_id` instead.
        error_response_with_id(self, None)
    }
}

/// Render a `ProxyError` as a JSON-RPC 2.0 error, echoing the caller's request `id`
/// when known (so clients can correlate the error with their request).
fn error_response_with_id(err: ProxyError, id: Option<&serde_json::Value>) -> Response {
    {
        let (status, message, error_code, www_authenticate) = match &err {
            ProxyError::Unauthorized(m, host) => {
                // Include WWW-Authenticate header for OAuth 2.1 compliance (RFC 9728)
                let resource_metadata_url = match host {
                    Some(h) => format!("https://{}/.well-known/oauth-protected-resource", h),
                    None => "/.well-known/oauth-protected-resource".to_string(),
                };
                let www_auth = format!(
                    "Bearer resource_metadata=\"{}\", scope=\"*\"",
                    resource_metadata_url
                );
                (StatusCode::UNAUTHORIZED, m.clone(), "UNAUTHORIZED", Some(www_auth))
            }
            ProxyError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone(), "FORBIDDEN", None),
            ProxyError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone(), "NOT_FOUND", None),
            ProxyError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone(), "BAD_REQUEST", None),
            ProxyError::RateLimitExceeded => {
                (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded. Please slow down.".to_string(), "RATE_LIMIT_EXCEEDED", None)
            }
            ProxyError::QuotaExceeded(m) => {
                (StatusCode::TOO_MANY_REQUESTS, m.clone(), "MONTHLY_QUOTA_EXCEEDED", None)
            }
            ProxyError::PaymentRequired(m) => {
                (StatusCode::PAYMENT_REQUIRED, m.clone(), "PAYMENT_REQUIRED", None)
            }
            ProxyError::ServiceUnavailable(m) => {
                // Log internal details, return safe message
                tracing::error!("Service unavailable: {}", m);
                (StatusCode::SERVICE_UNAVAILABLE, "Service temporarily unavailable".to_string(), "SERVICE_UNAVAILABLE", None)
            }
            ProxyError::Internal(m) => {
                // Log internal details, return safe message
                tracing::error!("Internal proxy error: {}", m);
                (StatusCode::INTERNAL_SERVER_ERROR, "An internal error occurred".to_string(), "INTERNAL_ERROR", None)
            }
        };

        // Map HTTP status to JSON-RPC 2.0 error codes
        // -32700: Parse error, -32600: Invalid Request, -32601: Method not found
        // -32602: Invalid params, -32603: Internal error
        // -32000 to -32099: Server error (implementation-defined)
        let jsonrpc_code = match status {
            StatusCode::BAD_REQUEST => -32600,      // Invalid Request
            StatusCode::UNAUTHORIZED => -32001,     // Server error: unauthorized
            StatusCode::FORBIDDEN => -32002,        // Server error: forbidden
            StatusCode::NOT_FOUND => -32601,        // Method not found (closest match)
            StatusCode::TOO_MANY_REQUESTS => -32003, // Server error: rate limit
            StatusCode::PAYMENT_REQUIRED => -32004, // Server error: payment required
            StatusCode::SERVICE_UNAVAILABLE => -32005, // Server error: service unavailable
            _ => -32603,                            // Internal error
        };

        // JSON-RPC 2.0 compliant error response (echo the inbound id when known)
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id.cloned().unwrap_or(serde_json::Value::Null),
            "error": {
                "code": jsonrpc_code,
                "message": message,
                "data": {
                    "type": error_code,
                    "status": status.as_u16()
                }
            }
        });

        let mut response = (status, axum::Json(body)).into_response();

        // Add WWW-Authenticate header for 401 responses (OAuth 2.1 compliance)
        if let Some(www_auth) = www_authenticate {
            response.headers_mut().insert(
                axum::http::header::WWW_AUTHENTICATE,
                www_auth.parse().unwrap_or_else(|_| HeaderValue::from_static("Bearer")),
            );
        }

        response
    }
}

use axum::http::HeaderMap;
use chrono::Datelike;
use fred::interfaces::{KeysInterface, LuaInterface};
use mcp_billing::Plan;
use mcp_db::WorkspaceRepository;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::redis_cache::{CachedServer, CachedWorkspace};
use crate::{ProxyError, ProxyState};

/// Process-local fixed-window fallback limiter.
///
/// When Redis is unreachable the distributed limiter cannot enforce anything; the
/// previous behaviour was to silently fail *open*, which means a Redis outage drops
/// all protection across the entire fleet simultaneously. This in-memory limiter
/// keeps a coarse per-process cap so a single proxy instance can't be trivially
/// abused during an outage. It is intentionally conservative and best-effort
/// (per-process, not cluster-wide).
static FALLBACK_LIMITER: LazyLock<Mutex<HashMap<String, (u64, u64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns true if the request is allowed by the process-local fallback limiter.
/// `window_bucket` partitions time into `window_secs` slots; counts reset per slot.
fn fallback_allow(key: &str, limit: u64, window_secs: u64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let window = if window_secs == 0 { now } else { now / window_secs };

    let mut map = match FALLBACK_LIMITER.lock() {
        Ok(m) => m,
        Err(p) => p.into_inner(),
    };

    // Opportunistic cleanup to bound memory: drop entries from older windows.
    // Lower threshold (1,000) makes it harder to exhaust memory AND to exploit
    // the window-reset behaviour by flooding with unique keys before cleanup.
    if map.len() > 1_000 {
        map.retain(|_, (w, _)| *w == window);
    }

    let entry = map.entry(key.to_string()).or_insert((window, 0));
    if entry.0 != window {
        *entry = (window, 0);
    }
    entry.1 += 1;
    entry.1 <= limit
}

/// Record that the distributed (Redis) limiter failed open and we fell back to the
/// process-local limiter, for alerting.
fn note_redis_fallback(scope: &'static str) {
    metrics::counter!("proxy_redis_fallback_total", "scope" => scope).increment(1);
    tracing::warn!(
        "rate_limit: Redis unavailable, using process-local fallback limiter (scope={})",
        scope
    );
}

/// Extract real client IP from request, handling reverse proxy headers.
///
/// Security: only trusts proxy headers when `TRUST_PROXY_HEADERS=true`.
///
/// The originating client controls the *leftmost* entries of `X-Forwarded-For`
/// (and can inject `cf-connecting-ip` headers when not behind that provider).
/// Trusting those lets an attacker spoof an arbitrary IP — evading their own
/// rate-limit counters or poisoning a victim's. We therefore only trust the
/// *rightmost* hop(s) that our own infrastructure appended.
///
/// On bare-metal, Caddy terminates TLS and sets `X-Real-IP`.
///
/// `TRUSTED_PROXY_HOPS` (default 1) is the number of trusted reverse proxies in
/// front of us; we take the IP `hops` entries from the right of `X-Forwarded-For`.
pub fn extract_client_ip(headers: &HeaderMap, addr: &SocketAddr) -> String {
    let trust_proxy = std::env::var("TRUST_PROXY_HEADERS")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    if !trust_proxy {
        return addr.ip().to_string();
    }

    // Cloudflare / nginx set these by *replacing* the value (not appending), so a
    // client behind the proxy cannot forge them — safe to trust under TRUST_PROXY_HEADERS.
    if let Some(cf_ip) = headers.get("cf-connecting-ip").and_then(|v| v.to_str().ok()) {
        if is_valid_ip(cf_ip) {
            return cf_ip.to_string();
        }
    }
    if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        if is_valid_ip(real_ip) {
            return real_ip.to_string();
        }
    }

    // Number of trusted reverse-proxy hops in front of us.
    let trusted_hops: usize = std::env::var("TRUSTED_PROXY_HOPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(1);

    // X-Forwarded-For: trust only the hop our own proxy appended. The list is
    // ordered client, proxy1, proxy2, ...; the rightmost entries are the ones we
    // control. Skip `trusted_hops - 1` from the right and take that IP.
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let ips: Vec<&str> = xff.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if !ips.is_empty() {
            let idx = ips.len().saturating_sub(trusted_hops);
            if let Some(ip) = ips.get(idx) {
                if is_valid_ip(ip) {
                    return ip.to_string();
                }
            }
        }
    }

    // Fall back to direct connection IP (the actual socket peer).
    addr.ip().to_string()
}

/// Validate that a string looks like a valid IP address
fn is_valid_ip(ip: &str) -> bool {
    !ip.is_empty() && (ip.parse::<std::net::Ipv4Addr>().is_ok() || ip.parse::<std::net::Ipv6Addr>().is_ok())
}

const DEFAULT_RATE_LIMIT: i32 = 100; // requests per minute
const WINDOW_SIZE_SECONDS: i64 = 60;

// Brute force protection for API key validation
const API_KEY_BRUTE_FORCE_PREFIX: &str = "bf:apikey:";

/// Brute force protection configuration
struct BruteForceConfig {
    max_attempts: i64,
    lockout_secs: u64,
    attempt_window_secs: u64,
}

impl Default for BruteForceConfig {
    fn default() -> Self {
        Self {
            max_attempts: std::env::var("API_KEY_BRUTE_FORCE_MAX_ATTEMPTS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(10),
            lockout_secs: std::env::var("API_KEY_BRUTE_FORCE_LOCKOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(600), // 10 minutes
            attempt_window_secs: std::env::var("API_KEY_BRUTE_FORCE_WINDOW_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300), // 5 minutes
        }
    }
}

/// Check if an IP is currently locked out due to API key brute force
pub async fn is_api_key_locked_out(state: &ProxyState, ip: &str) -> bool {
    let lockout_key = format!("{}lockout:{}", API_KEY_BRUTE_FORCE_PREFIX, ip);
    let exists: Result<bool, _> = state.redis.exists(&lockout_key).await;
    match &exists {
        Ok(_) => {}
        Err(e) => tracing::warn!("Redis error checking lockout for {}: {}", ip, e),
    }
    exists.unwrap_or(false)
}

/// Get remaining lockout time in seconds for API key brute force
pub async fn get_api_key_lockout_remaining(state: &ProxyState, ip: &str) -> Option<i64> {
    let lockout_key = format!("{}lockout:{}", API_KEY_BRUTE_FORCE_PREFIX, ip);
    let ttl: Result<i64, _> = state.redis.ttl(&lockout_key).await;
    ttl.ok().filter(|&t| t > 0)
}

/// Record a failed API key attempt and potentially lock out the IP
pub async fn record_api_key_failed_attempt(state: &ProxyState, ip: &str) {
    let config = BruteForceConfig::default();
    let attempts_key = format!("{}attempts:{}", API_KEY_BRUTE_FORCE_PREFIX, ip);
    let lockout_key = format!("{}lockout:{}", API_KEY_BRUTE_FORCE_PREFIX, ip);

    let lua_script = r#"
        local attempts = redis.call('INCR', KEYS[1])
        if attempts == 1 then
            redis.call('EXPIRE', KEYS[1], ARGV[1])
        end
        if attempts >= tonumber(ARGV[2]) then
            redis.call('SET', KEYS[2], '1', 'EX', ARGV[3])
            redis.call('DEL', KEYS[1])
        end
        return attempts
    "#;

    let result: Result<i64, _> = state
        .redis
        .eval(
            lua_script,
            &[attempts_key, lockout_key],
            &[
                config.attempt_window_secs.to_string(),
                config.max_attempts.to_string(),
                config.lockout_secs.to_string(),
            ],
        )
        .await;

    if let Ok(attempts) = result {
        if attempts >= config.max_attempts {
            tracing::warn!(
                "IP {} locked out after {} failed API key attempts (lockout: {}s)",
                ip,
                attempts,
                config.lockout_secs
            );
        }
    }
}

/// Clear failed API key attempts after successful validation
pub async fn clear_api_key_failed_attempts(state: &ProxyState, ip: &str) {
    let attempts_key = format!("{}attempts:{}", API_KEY_BRUTE_FORCE_PREFIX, ip);
    let _: Result<(), _> = state.redis.del(&attempts_key).await;
}

/// Lua script for atomic fixed window rate limiting
/// Uses a simple counter with expiration - more memory efficient than sliding window
/// Trade-off: slightly less accurate at window boundaries, but much better memory usage
/// Memory usage: O(1) per key instead of O(n) where n = requests per window
const RATE_LIMIT_SCRIPT: &str = r#"
local key = KEYS[1]
local limit = tonumber(ARGV[1])
local ttl = tonumber(ARGV[2])

-- Increment counter
local current = redis.call('INCR', key)

-- Set TTL only on first request (when counter is 1)
if current == 1 then
    redis.call('EXPIRE', key, ttl)
end

-- Check if limit exceeded
if current > limit then
    return -1
end

return current
"#;

pub async fn check(
    state: &ProxyState,
    credential_id: Uuid,
    server: &CachedServer,
) -> Result<(), ProxyError> {
    // Use minute-based key for fixed window rate limiting
    let now = chrono::Utc::now();
    let minute_bucket = now.timestamp() / WINDOW_SIZE_SECONDS;
    let key = format!("rate_limit:{}:{}:{}", credential_id, server.id, minute_bucket);
    let limit = server.rate_limit_per_minute.unwrap_or(DEFAULT_RATE_LIMIT) as i64;
    // TTL should be slightly longer than window to handle edge cases
    let ttl = WINDOW_SIZE_SECONDS + 5;

    // Execute atomic rate limiting with Lua script
    let result: Result<i64, _> = state
        .redis
        .eval(
            RATE_LIMIT_SCRIPT,
            &[key],
            &[limit.to_string(), ttl.to_string()],
        )
        .await;

    let result = match result {
        Ok(v) => v,
        Err(_) => {
            // Redis down: enforce a process-local cap instead of failing fully open.
            note_redis_fallback("rate_limit");
            let fb_key = format!("rl:{}:{}", credential_id, server.id);
            if fallback_allow(&fb_key, limit.max(1) as u64, WINDOW_SIZE_SECONDS as u64) {
                return Ok(());
            }
            return Err(ProxyError::RateLimitExceeded);
        }
    };

    if result < 0 {
        return Err(ProxyError::RateLimitExceeded);
    }

    Ok(())
}

/// Per-IP rate limit for unauthenticated (auth_enabled=false) servers.
///
/// auth_enabled=false means there is no credential to key a per-user counter on, so
/// this is our only throttle against individual bad actors. We keep limits tighter
/// than the per-credential default and key the Redis counter on the client IP.
pub async fn check_by_ip(
    state: &ProxyState,
    ip: &str,
    limit: u64,
) -> Result<(), ProxyError> {
    let now = chrono::Utc::now();
    let minute_bucket = now.timestamp() / WINDOW_SIZE_SECONDS;
    let key = format!("rate_limit:ip:{}:{}", ip, minute_bucket);
    let ttl = WINDOW_SIZE_SECONDS + 5;

    let result: Result<i64, _> = state
        .redis
        .eval(
            RATE_LIMIT_SCRIPT,
            &[key],
            &[limit.to_string(), ttl.to_string()],
        )
        .await;

    let result = match result {
        Ok(v) => v,
        Err(_) => {
            note_redis_fallback("rate_limit_ip");
            let fb_key = format!("rl_ip:{}", ip);
            return if fallback_allow(&fb_key, limit, WINDOW_SIZE_SECONDS as u64) {
                Ok(())
            } else {
                Err(ProxyError::RateLimitExceeded)
            };
        }
    };

    if result < 0 {
        return Err(ProxyError::RateLimitExceeded);
    }

    Ok(())
}

/// Atomic monthly quota: GET current, reject if at/over limit, else INCR + set TTL
/// when the counter is first created. Single round-trip, so there is no
/// check-then-increment TOCTOU and no window where INCR succeeds but EXPIRE is lost.
const MONTHLY_QUOTA_SCRIPT: &str = r#"
local key = KEYS[1]
local limit = tonumber(ARGV[1])
local ttl = tonumber(ARGV[2])
local current = tonumber(redis.call('GET', key) or '0')
if current >= limit then
    return -1
end
local newval = redis.call('INCR', key)
if newval == 1 then
    redis.call('EXPIRE', key, ttl)
end
return newval
"#;

/// Load the workspace plan/subscription for the quota hot path, preferring the
/// short-TTL Redis cache and falling back to Postgres (then populating the cache).
async fn load_workspace(
    state: &ProxyState,
    workspace_id: Uuid,
) -> Result<CachedWorkspace, ProxyError> {
    if let Some(cached) = state.redis_cache.get_workspace(&workspace_id).await {
        return Ok(cached);
    }

    let workspace = WorkspaceRepository::find_by_id(&state.db, workspace_id)
        .await
        .map_err(|e| ProxyError::Internal(format!("Database error: {}", e)))?
        .ok_or_else(|| ProxyError::Internal("Workspace not found".into()))?;

    // Populate cache (async, don't block).
    let redis_cache = state.redis_cache.clone();
    let ws_clone = workspace.clone();
    tokio::spawn(async move {
        redis_cache.set_workspace(&ws_clone).await;
    });

    Ok(CachedWorkspace::from(&workspace))
}

/// Check the monthly request quota for a workspace **and** increment the counter
/// atomically before the request is forwarded.
///
/// Incrementing inline (rather than fire-and-forget after a successful response)
/// removes the TOCTOU where many concurrent requests all read an under-limit count
/// and pass. The counter is incremented for every accepted request.
pub async fn check_and_increment_monthly_quota(
    state: &ProxyState,
    workspace_id: Uuid,
) -> Result<(), ProxyError> {
    let workspace = load_workspace(state, workspace_id).await?;

    // Check subscription status - block if past_due or cancelled
    if let Some(ref status) = workspace.subscription_status {
        if status == "past_due" || status == "unpaid" {
            return Err(ProxyError::PaymentRequired(
                "Your subscription payment is past due. Please update your payment method.".into()
            ));
        }
        if status == "cancelled" && workspace.plan != "free" {
            // If cancelled but not yet downgraded to free, check period end
            if let Some(period_end) = workspace.current_period_end {
                if chrono::Utc::now() > period_end {
                    return Err(ProxyError::PaymentRequired(
                        "Your subscription has expired. Please renew to continue.".into()
                    ));
                }
            }
        }
    }

    // Get plan limits
    let billing_plan = match workspace.plan.as_str() {
        "pro" => Plan::Pro,
        "team" => Plan::Team,
        "enterprise" => Plan::Enterprise,
        _ => Plan::Free,
    };
    let limits = billing_plan.limits();
    let limit = limits.max_requests_per_month;

    // Get current month key
    let now = chrono::Utc::now();
    let month_key = format!(
        "monthly_requests:{}:{:04}-{:02}",
        workspace_id,
        now.year(),
        now.month()
    );

    // TTL to expire after this month (+5 days buffer).
    let days_in_month = match now.month() {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if now.year() % 4 == 0 && (now.year() % 100 != 0 || now.year() % 400 == 0) { 29 } else { 28 },
        _ => 31,
    };
    let ttl_seconds = ((days_in_month - now.day() + 5) as i64) * 24 * 60 * 60;

    let result: Result<i64, _> = state
        .redis
        .eval(
            MONTHLY_QUOTA_SCRIPT,
            &[month_key],
            &[limit.to_string(), ttl_seconds.to_string()],
        )
        .await;

    match result {
        Ok(v) if v < 0 => Err(ProxyError::QuotaExceeded(format!(
            "Monthly request quota exceeded (limit {}). Please upgrade your plan.",
            limit
        ))),
        Ok(_) => Ok(()),
        Err(_) => {
            // Redis down: the monthly counter can't be tracked process-locally, but a
            // coarse per-process per-minute cap still bounds abuse during the outage.
            note_redis_fallback("monthly_quota");
            let fb_key = format!("mq:{}", workspace_id);
            if fallback_allow(&fb_key, DEFAULT_RATE_LIMIT as u64, WINDOW_SIZE_SECONDS as u64) {
                Ok(())
            } else {
                Err(ProxyError::RateLimitExceeded)
            }
        }
    }
}

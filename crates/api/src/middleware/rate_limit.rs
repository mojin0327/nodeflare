use axum::{
    extract::{ConnectInfo, State},
    http::{header::HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    body::Body,
};
use fred::interfaces::{KeysInterface, LuaInterface};
use std::{net::SocketAddr, sync::Arc};

use crate::state::AppState;

/// Extract real client IP from request, handling reverse proxy headers
///
/// Security: Only trusts proxy headers when TRUST_PROXY_HEADERS=true
/// Priority: fly-client-ip > cf-connecting-ip > x-real-ip > x-forwarded-for > direct connection
pub fn extract_client_ip(headers: &HeaderMap, addr: &SocketAddr) -> String {
    let trust_proxy = std::env::var("TRUST_PROXY_HEADERS")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    if !trust_proxy {
        return addr.ip().to_string();
    }

    // Fly.io specific header (most trusted when using Fly.io)
    if let Some(fly_ip) = headers.get("fly-client-ip").and_then(|v| v.to_str().ok()) {
        if is_valid_ip(fly_ip) {
            return fly_ip.to_string();
        }
    }

    // Cloudflare header
    if let Some(cf_ip) = headers.get("cf-connecting-ip").and_then(|v| v.to_str().ok()) {
        if is_valid_ip(cf_ip) {
            return cf_ip.to_string();
        }
    }

    // Nginx/generic reverse proxy header
    if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        if is_valid_ip(real_ip) {
            return real_ip.to_string();
        }
    }

    // X-Forwarded-For: take the first (leftmost) IP which is the original client
    // Format: "client, proxy1, proxy2"
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first_ip) = xff.split(',').next().map(|s| s.trim()) {
            if is_valid_ip(first_ip) {
                return first_ip.to_string();
            }
        }
    }

    // Fall back to direct connection IP
    addr.ip().to_string()
}

/// Validate that a string looks like a valid IP address (basic check)
fn is_valid_ip(ip: &str) -> bool {
    // Must not be empty and must parse as IP
    !ip.is_empty() && (ip.parse::<std::net::Ipv4Addr>().is_ok() || ip.parse::<std::net::Ipv6Addr>().is_ok())
}

/// Brute force protection configuration
#[derive(Clone)]
pub struct BruteForceConfig {
    /// Maximum failed attempts before lockout
    pub max_attempts: i64,
    /// Lockout duration in seconds
    pub lockout_secs: u64,
    /// Window for counting attempts in seconds
    pub attempt_window_secs: u64,
}

impl Default for BruteForceConfig {
    fn default() -> Self {
        Self {
            max_attempts: std::env::var("BRUTE_FORCE_MAX_ATTEMPTS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5),
            lockout_secs: std::env::var("BRUTE_FORCE_LOCKOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(900), // 15 minutes
            attempt_window_secs: std::env::var("BRUTE_FORCE_WINDOW_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300), // 5 minutes
        }
    }
}

/// Check if an IP is currently locked out due to brute force protection
pub async fn is_ip_locked_out(
    redis: &fred::prelude::RedisClient,
    ip: &str,
    key_prefix: &str,
) -> bool {
    let lockout_key = format!("{}lockout:{}", key_prefix, ip);
    let exists: Result<bool, _> = redis.exists(&lockout_key).await;
    exists.unwrap_or(false)
}

/// Record a failed authentication attempt and potentially lock out the IP
pub async fn record_failed_attempt(
    redis: &fred::prelude::RedisClient,
    ip: &str,
    key_prefix: &str,
) {
    let config = BruteForceConfig::default();
    let attempts_key = format!("{}attempts:{}", key_prefix, ip);
    let lockout_key = format!("{}lockout:{}", key_prefix, ip);

    // Increment failed attempts using Lua script for atomicity
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

    let result: Result<i64, _> = LuaInterface::eval(
        redis,
        lua_script,
        vec![attempts_key, lockout_key],
        vec![
            config.attempt_window_secs.to_string(),
            config.max_attempts.to_string(),
            config.lockout_secs.to_string(),
        ],
    )
    .await;

    if let Ok(attempts) = result {
        if attempts >= config.max_attempts {
            tracing::warn!(
                "IP {} locked out after {} failed attempts (lockout: {}s)",
                ip,
                attempts,
                config.lockout_secs
            );
        }
    }
}

/// Clear failed attempts after successful authentication
pub async fn clear_failed_attempts(
    redis: &fred::prelude::RedisClient,
    ip: &str,
    key_prefix: &str,
) {
    let attempts_key = format!("{}attempts:{}", key_prefix, ip);
    let _ = KeysInterface::del::<(), _>(redis, &attempts_key).await;
}

/// Get remaining lockout time in seconds
pub async fn get_lockout_remaining(
    redis: &fred::prelude::RedisClient,
    ip: &str,
    key_prefix: &str,
) -> Option<i64> {
    let lockout_key = format!("{}lockout:{}", key_prefix, ip);
    let ttl: Result<i64, _> = redis.ttl(&lockout_key).await;
    ttl.ok().filter(|&t| t > 0)
}

/// Rate limit configuration
#[derive(Clone)]
pub struct RateLimitConfig {
    /// Maximum requests per window
    pub max_requests: i64,
    /// Window duration in seconds
    pub window_secs: u64,
    /// Key prefix for Redis
    pub key_prefix: String,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            // Scalability: Increased from 100 to 1000 for production workloads
            // This is a global rate limit per IP; plan-specific limits are applied separately
            max_requests: std::env::var("RATE_LIMIT_MAX_REQUESTS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1000),
            window_secs: std::env::var("RATE_LIMIT_WINDOW_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60),
            key_prefix: "rate_limit:".to_string(),
        }
    }
}

/// Rate limit middleware using Redis with atomic INCR + EXPIRE via Lua script
pub async fn rate_limit_middleware(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let config = RateLimitConfig::default();
    // Extract real client IP (handles reverse proxy headers when TRUST_PROXY_HEADERS=true)
    let ip = extract_client_ip(request.headers(), &addr);
    let key = format!("{}{}", config.key_prefix, ip);

    // Use Lua script for atomic rate limiting (fixes race condition)
    let lua_script = r#"
        local current = redis.call('INCR', KEYS[1])
        if current == 1 then
            redis.call('EXPIRE', KEYS[1], ARGV[1])
        end
        return current
    "#;

    let result: Result<i64, _> = fred::interfaces::LuaInterface::eval(
        &state.redis,
        lua_script,
        vec![key.clone()],
        vec![config.window_secs.to_string()],
    )
    .await;

    match result {
        Ok(count) if count > config.max_requests => {
            tracing::warn!("Rate limit exceeded for IP: {} (count: {})", ip, count);
            (
                StatusCode::TOO_MANY_REQUESTS,
                [
                    ("X-RateLimit-Limit", config.max_requests.to_string()),
                    ("X-RateLimit-Remaining", "0".to_string()),
                    ("Retry-After", config.window_secs.to_string()),
                ],
                "Too many requests. Please try again later.",
            )
                .into_response()
        }
        Ok(count) => {
            let mut response = next.run(request).await;
            let remaining = (config.max_requests - count).max(0);

            // Add rate limit headers to response (ignore if header value fails, which shouldn't happen for numbers)
            let headers = response.headers_mut();
            if let Ok(limit_val) = config.max_requests.to_string().parse() {
                headers.insert("X-RateLimit-Limit", limit_val);
            }
            if let Ok(remaining_val) = remaining.to_string().parse() {
                headers.insert("X-RateLimit-Remaining", remaining_val);
            }

            response
        }
        Err(e) => {
            // On Redis error, allow the request but log the error
            tracing::error!("Rate limit Redis error: {}", e);
            next.run(request).await
        }
    }
}

/// WebSocket connection rate limiting
/// Returns true if connection is allowed, false if rate limited
pub async fn check_ws_connection_rate_limit(
    redis: &fred::prelude::RedisClient,
    ip: &str,
) -> Result<bool, String> {
    // Scalability: Increased from 30 to 100 connections per minute per IP
    // This allows legitimate users with multiple browser tabs/windows
    let max_connections: i64 = std::env::var("WS_RATE_LIMIT_MAX_CONNECTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let window_secs: u64 = std::env::var("WS_RATE_LIMIT_WINDOW_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);

    let key = format!("ws_rate_limit:{}", ip);

    let lua_script = r#"
        local current = redis.call('INCR', KEYS[1])
        if current == 1 then
            redis.call('EXPIRE', KEYS[1], ARGV[1])
        end
        return current
    "#;

    let result: Result<i64, _> = LuaInterface::eval(
        redis,
        lua_script,
        vec![key],
        vec![window_secs.to_string()],
    )
    .await;

    match result {
        Ok(count) => Ok(count <= max_connections),
        Err(e) => {
            tracing::error!("WebSocket rate limit Redis error: {}", e);
            Ok(true) // Allow on Redis error (fail-open)
        }
    }
}

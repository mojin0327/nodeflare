//! Session affinity for stateful MCP servers running on more than one Fly Machine.
//!
//! Streamable HTTP MCP servers keep their session state (keyed by `Mcp-Session-Id`)
//! in the memory of whichever Machine served `initialize`. Fly's edge load-balances
//! across Machines, so a follow-up request can land on a *different* Machine that has
//! never seen the session and replies `400 Bad Request: Invalid session ID`.
//!
//! We pin each session to its owning Machine using Fly's `fly-force-instance-id`
//! request header:
//!   - `initialize` (no incoming session): pick a (preferably started) Machine, force
//!     it, then record `session-id -> machine-id` once the Machine returns the new
//!     session id in its response.
//!   - follow-up (carries `Mcp-Session-Id`): look the session up and force the same
//!     Machine.
//!   - anything else (stateless calls, legacy SSE, metadata, browsers): no header is
//!     added, so routing is unchanged — no behavioural regression.
//!
//! Everything here is best-effort: any failure (no Fly token, API error, unknown
//! session, non-`.fly.dev` endpoint, single-Machine app) yields *no* forced header,
//! i.e. exactly today's round-robin behaviour. There is deliberately no fallback
//! branch for "the pinned Machine is stopped" — Fly's proxy wakes a stopped Machine
//! when a request is forced to it (verified empirically), so forcing always resolves.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use axum::http::HeaderMap;
use fred::interfaces::LuaInterface;
use fred::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ProxyState;

/// How long a `session-id -> machine-id` binding lives in Redis (refreshed on use).
const SESSION_TTL_SECS: i64 = 3600;
/// How long the per-app Machine list is cached in Redis.
/// Short TTL so the proxy quickly detects when a cold machine becomes ready and
/// can prefer it over forcing another stopped machine on subsequent initializes.
const MACHINE_LIST_TTL_SECS: i64 = 5;

/// Per-app round-robin cursors for picking a Machine on `initialize`. A single
/// global cursor doesn't round-robin per app (interleaved apps skew each other),
/// so we key the cursor by app name.
static RR_CURSORS: LazyLock<Mutex<HashMap<String, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn next_rr(app: &str, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let mut cursors = RR_CURSORS.lock().unwrap_or_else(|p| p.into_inner());
    let cur = cursors.entry(app.to_string()).or_insert(0);
    let idx = *cur % len;
    *cur = cur.wrapping_add(1);
    idx
}

/// Outcome of an affinity decision for a single request.
#[derive(Default)]
pub struct Affinity {
    /// Machine id to pin this request to via `fly-force-instance-id`, if any.
    pub forced_machine: Option<String>,
    /// When set, this request is an `initialize`: bind the session id returned by the
    /// upstream to this Machine id once the response is available.
    pub bind_session_to: Option<String>,
    /// Fly app name this decision is scoped to (used to namespace the session key
    /// when binding the new session on an `initialize` response).
    pub app: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Machine {
    id: String,
    state: String,
}


/// Decide whether (and where) to pin this request.
pub async fn decide(
    state: &ProxyState,
    target_url: &str,
    headers: &HeaderMap,
    is_initialize: bool,
) -> Affinity {
    let app = app_name_from_target(target_url);

    // Follow-up request: route to the Machine that owns this session.
    if let Some(session_id) = headers.get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        // Non-fly endpoints have no Machine concept; never force.
        let app = match app {
            Some(a) => a,
            None => return Affinity::default(),
        };
        if let Some(machine) = lookup_session(state, &app, session_id).await {
            // Cross-check the bound Machine still exists. Machines are recreated with
            // fresh ids on every deploy, so a stale binding would force traffic onto a
            // dead id and break every session after a deploy. If it's gone, drop the
            // binding and let Fly route normally (the session is lost either way).
            let machines = machine_list(state, &app).await;
            let alive = machines.iter().any(|m| m.id == machine);
            if alive {
                return Affinity {
                    forced_machine: Some(machine),
                    bind_session_to: None,
                    app: Some(app),
                };
            }
            tracing::info!(
                "affinity: bound machine {} for session {} no longer exists, dropping binding",
                machine,
                session_id
            );
            delete_session(state, &app, session_id).await;
            return Affinity::default();
        }
        // Unknown/expired session -> don't force; let Fly route normally.
        return Affinity::default();
    }

    // initialize: pick a Machine, force it, and bind the new session to it.
    if is_initialize {
        if let Some(app) = app {
            let machines = machine_list(state, &app).await;
            if let Some(machine) = pick_machine(&app, &machines) {
                return Affinity {
                    forced_machine: Some(machine.clone()),
                    bind_session_to: Some(machine),
                    app: Some(app),
                };
            }
        }
    }

    Affinity::default()
}

/// After an `initialize` response, persist `session-id -> machine-id`.
///
/// `session_id` is the `Mcp-Session-Id` the upstream returned (if any).
pub async fn capture_session(state: &ProxyState, affinity: &Affinity, session_id: Option<&str>) {
    if let (Some(app), Some(machine), Some(session_id)) = (
        affinity.app.as_deref(),
        affinity.bind_session_to.as_deref(),
        session_id,
    ) {
        store_session(state, app, session_id, machine).await;
        tracing::info!("affinity: bound session {} -> machine {}", session_id, machine);
    }
}

/// `https://mcp-xxxx.fly.dev/mcp?...` -> `Some("mcp-xxxx")`. `None` for non-fly.dev hosts.
fn app_name_from_target(target_url: &str) -> Option<String> {
    let after_scheme = target_url.split("://").nth(1)?;
    let host = after_scheme.split('/').next()?;
    let host = host.split(':').next().unwrap_or(host);
    host.strip_suffix(".fly.dev").map(|s| s.to_string())
}

fn pick_machine(app: &str, machines: &[Machine]) -> Option<String> {
    if machines.is_empty() {
        return None;
    }
    // Prefer already-running Machines so `initialize` doesn't pay a cold start; fall
    // back to any Machine (forcing wakes a stopped one).
    let started: Vec<&Machine> = machines.iter().filter(|m| m.state == "started").collect();
    let pool: Vec<&Machine> = if started.is_empty() {
        machines.iter().collect()
    } else {
        started
    };
    let idx = next_rr(app, pool.len());
    Some(pool[idx].id.clone())
}

/// `GETEX`-style lookup: fetch the binding and refresh its TTL in a single round
/// trip (fred 8 has no `getex` helper, so we use a tiny Lua script).
const GETEX_SCRIPT: &str = r#"
local v = redis.call('GET', KEYS[1])
if v then
    redis.call('EXPIRE', KEYS[1], ARGV[1])
end
return v
"#;

async fn lookup_session(state: &ProxyState, app: &str, session_id: &str) -> Option<String> {
    let key = session_key(app, session_id);
    let machine: Option<String> = state
        .redis
        .eval(GETEX_SCRIPT, &[key], &[SESSION_TTL_SECS.to_string()])
        .await
        .ok()
        .flatten();
    machine
}

async fn store_session(state: &ProxyState, app: &str, session_id: &str, machine: &str) {
    let key = session_key(app, session_id);
    let _: Result<(), _> = state
        .redis
        .set(
            &key,
            machine,
            Some(Expiration::EX(SESSION_TTL_SECS)),
            None,
            false,
        )
        .await;
}

async fn delete_session(state: &ProxyState, app: &str, session_id: &str) {
    let key = session_key(app, session_id);
    let _: Result<(), _> = state.redis.del(&key).await;
}

/// Namespaced by app so two apps can't ever collide on a session id.
fn session_key(app: &str, session_id: &str) -> String {
    format!("proxy:mcpaff:sess:{}:{}", app, session_id)
}

/// Fetch the app's Machine list, cached in Redis for `MACHINE_LIST_TTL_SECS`.
async fn machine_list(state: &ProxyState, app: &str) -> Vec<Machine> {
    let cache_key = format!("proxy:mcpaff:machines:{}", app);

    let cached: Option<String> = state.redis.get(&cache_key).await.ok().flatten();
    if let Some(json) = cached {
        if let Ok(list) = serde_json::from_str::<Vec<Machine>>(&json) {
            return list;
        }
    }

    let machines = fetch_machines(state, app).await;
    if !machines.is_empty() {
        if let Ok(json) = serde_json::to_string(&machines) {
            let _: Result<(), _> = state
                .redis
                .set(
                    &cache_key,
                    json,
                    Some(Expiration::EX(MACHINE_LIST_TTL_SECS)),
                    None,
                    false,
                )
                .await;
        }
    }
    machines
}

async fn fetch_machines(_state: &ProxyState, _app: &str) -> Vec<Machine> {
    // In bare-metal mode, each MCP server has exactly one container on the same host.
    // There are no remote machines to enumerate, so affinity routing always yields None
    // (no forced_machine). Session bindings are still stored/looked up in Redis so that
    // future multi-instance support can be added without a schema change.
    Vec::new()
}

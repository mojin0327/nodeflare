use axum::http::StatusCode;
use mcp_db::McpServer;
use uuid::Uuid;

use crate::error::db_error;
use crate::state::AppState;

// ── Common error message constants ───────────────────────────────────────────
pub const ERR_NOT_A_MEMBER: &str = "Not a member";
pub const ERR_SERVER_NOT_FOUND: &str = "Server not found";
pub const ERR_WORKSPACE_NOT_FOUND: &str = "Workspace not found";
pub const ERR_INSUFFICIENT_PERMISSIONS: &str = "Insufficient permissions";
pub const ERR_DEPLOYMENT_NOT_FOUND: &str = "Deployment not found";

// ── Security constants ────────────────────────────────────────────────────────
pub const CSRF_TOKEN_TTL_SECS: i64 = 600; // 10 minutes

// ── Shared route helpers ──────────────────────────────────────────────────────

/// Verify that `server_id` belongs to `workspace_id`. Returns `NOT_FOUND` for
/// both missing and mismatched servers to avoid leaking server existence.
pub async fn verify_server_ownership(
    state: &AppState,
    workspace_id: Uuid,
    server_id: Uuid,
) -> Result<(), (StatusCode, String)> {
    fetch_server_for_workspace(state, workspace_id, server_id).await.map(|_| ())
}

/// Fetch a server by ID and verify it belongs to `workspace_id`. Use this when
/// the handler needs the `Server` struct for further processing.
pub async fn fetch_server_for_workspace(
    state: &AppState,
    workspace_id: Uuid,
    server_id: Uuid,
) -> Result<McpServer, (StatusCode, String)> {
    use mcp_db::ServerRepository;

    let server = ServerRepository::find_by_id(&state.db, server_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::NOT_FOUND, ERR_SERVER_NOT_FOUND.to_string()))?;

    if server.workspace_id != workspace_id {
        return Err((StatusCode::NOT_FOUND, ERR_SERVER_NOT_FOUND.to_string()));
    }
    Ok(server)
}

/// Build the `Set-Cookie` header strings for access and refresh tokens.
pub fn build_auth_cookies(
    is_production: bool,
    access_token: &str,
    access_max_age: i64,
    refresh_token: &str,
    refresh_max_age: i64,
) -> (String, String) {
    let (samesite, secure) = if is_production { ("None", "; Secure") } else { ("Lax", "") };
    let access = format!(
        "access_token={}; HttpOnly{}; SameSite={}; Path=/; Max-Age={}",
        access_token, secure, samesite, access_max_age,
    );
    let refresh = format!(
        "refresh_token={}; HttpOnly{}; SameSite={}; Path=/; Max-Age={}",
        refresh_token, secure, samesite, refresh_max_age,
    );
    (access, refresh)
}

/// Build the `Set-Cookie` header strings that expire (clear) access and refresh tokens.
pub fn clear_auth_cookies(is_production: bool) -> (String, String) {
    let (samesite, secure) = if is_production { ("None", "; Secure") } else { ("Lax", "") };
    let access = format!("access_token=; HttpOnly{}; SameSite={}; Path=/; Max-Age=0", secure, samesite);
    let refresh = format!("refresh_token=; HttpOnly{}; SameSite={}; Path=/; Max-Age=0", secure, samesite);
    (access, refresh)
}

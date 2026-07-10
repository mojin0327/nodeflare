use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use fred::interfaces::KeysInterface;
use fred::types::Expiration;
use mcp_db::SecretRepository;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::state::AppState;

const REDIS_KEY_PREFIX: &str = "mcp:oauth:";
const TOKEN_REFRESH_BUFFER_SECS: i64 = 300;

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthTokenPayload {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

fn redis_key(server_id: Uuid) -> String {
    format!("{}{}", REDIS_KEY_PREFIX, server_id)
}

async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    server_id: Uuid,
) -> Result<(), (StatusCode, &'static str)> {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or((StatusCode::UNAUTHORIZED, "Missing bearer token"))?;

    let secret = SecretRepository::find_by_key(&state.db, server_id, "_NODEFLARE_SERVER_TOKEN")
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "DB error"))?
        .ok_or((StatusCode::UNAUTHORIZED, "Unknown server"))?;

    let stored = state
        .crypto
        .decrypt_string(&secret.encrypted_value, &secret.nonce)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Decryption error"))?;

    if stored != bearer {
        return Err((StatusCode::UNAUTHORIZED, "Invalid token"));
    }
    Ok(())
}

pub async fn get_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(server_id): Path<Uuid>,
) -> Result<Json<OAuthTokenPayload>, (StatusCode, &'static str)> {
    authenticate(&state, &headers, server_id).await?;

    let key = redis_key(server_id);
    let raw: Option<String> = state
        .redis
        .get(&key)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Redis error"))?;

    let payload = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or((StatusCode::NOT_FOUND, "No token stored"))?;

    Ok(Json(payload))
}

pub async fn put_token(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(server_id): Path<Uuid>,
    Json(payload): Json<OAuthTokenPayload>,
) -> Result<StatusCode, (StatusCode, &'static str)> {
    authenticate(&state, &headers, server_id).await?;

    let key = redis_key(server_id);
    let json = serde_json::to_string(&payload)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Serialization error"))?;

    let now = chrono::Utc::now().timestamp();
    let ttl = (payload.expires_at - now + TOKEN_REFRESH_BUFFER_SECS).max(60);

    state
        .redis
        .set::<(), _, _>(&key, json, Some(Expiration::EX(ttl)), None, false)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Redis error"))?;

    Ok(StatusCode::NO_CONTENT)
}

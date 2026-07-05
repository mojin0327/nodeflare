use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Path, Query, State,
    },
    response::IntoResponse,
    http::{StatusCode, header::COOKIE},
};
use futures::{SinkExt, StreamExt};
use mcp_common::types::WsMessage;
use mcp_db::{DeploymentRepository, ServerRepository};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::error::{db_error, service_error};
use crate::extractors::auth::extract_token_from_cookie;
use crate::middleware::rate_limit::{check_ws_connection_rate_limit, extract_client_ip};
use crate::state::AppState;
use crate::ws_manager::WsManager;

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    pub token: Option<String>,
}

/// Verify a WebSocket JWT and enforce revocation (individual token + user-wide, e.g.
/// after logout). The WS handlers authenticate off the raw JWT and would otherwise keep
/// working after logout until natural expiry — this mirrors the checks in the AuthUser
/// extractor so revocation applies to sockets too.
async fn verify_ws_token(state: &AppState, token: &str) -> Result<Uuid, (StatusCode, String)> {
    let claims = state
        .jwt
        .verify_token(token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token".to_string()))?;

    let user_id = claims
        .user_id()
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token".to_string()))?;

    if mcp_auth::is_token_revoked(&state.redis, &claims.jti).await {
        return Err((StatusCode::UNAUTHORIZED, "Token has been revoked".to_string()));
    }
    if let Some(revocation_time) =
        mcp_auth::get_user_revocation_timestamp(&state.redis, &user_id.to_string()).await
    {
        if claims.iat <= revocation_time {
            return Err((StatusCode::UNAUTHORIZED, "Token has been revoked".to_string()));
        }
    }

    Ok(user_id)
}

/// WebSocket handler for deployment status updates
pub async fn deployment_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(deployment_id): Path<Uuid>,
    Query(query): Query<WsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Rate limit WebSocket connection attempts
    let client_ip = extract_client_ip(&headers, &addr);
    if !check_ws_connection_rate_limit(&state.redis, &client_ip)
        .await
        .unwrap_or(true)
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many connection attempts".to_string(),
        ));
    }

    // Extract token from query parameter first (for cross-origin), then fallback to Cookie
    let token = query.token
        .or_else(|| {
            headers
                .get(COOKIE)
                .and_then(|h| h.to_str().ok())
                .and_then(extract_token_from_cookie)
                .map(|s| s.to_string())
        })
        .ok_or((StatusCode::UNAUTHORIZED, "Missing access token".to_string()))?;

    // Verify JWT token (including revocation, e.g. after logout)
    let user_id = verify_ws_token(&state, &token).await?;

    // Verify deployment exists and user has access with optimized query
    let access = DeploymentRepository::check_user_access(&state.db, deployment_id, user_id)
        .await
        .map_err(db_error)?;

    if !access {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    // Subscribe to deployment updates
    let channel = format!("deployment:{}", deployment_id);
    let rx = state.ws_manager.subscribe(&channel).await
        .map_err(|e| service_error("WebSocket", e))?;

    let ws_manager = state.ws_manager.clone();
    Ok(ws.on_upgrade(move |socket| handle_deployment_socket(socket, rx, deployment_id, ws_manager)))
}

/// WebSocket handler for server status updates
pub async fn server_status_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<WsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Rate limit WebSocket connection attempts
    let client_ip = extract_client_ip(&headers, &addr);
    if !check_ws_connection_rate_limit(&state.redis, &client_ip)
        .await
        .unwrap_or(true)
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many connection attempts".to_string(),
        ));
    }

    // Extract token from query parameter first (for cross-origin), then fallback to Cookie
    let token = query.token
        .or_else(|| {
            headers
                .get(COOKIE)
                .and_then(|h| h.to_str().ok())
                .and_then(extract_token_from_cookie)
                .map(|s| s.to_string())
        })
        .ok_or((StatusCode::UNAUTHORIZED, "Missing access token".to_string()))?;

    // Verify JWT token (including revocation, e.g. after logout)
    let user_id = verify_ws_token(&state, &token).await?;

    // Verify server exists, belongs to workspace, and user has access with optimized query
    let access = ServerRepository::check_user_access(&state.db, server_id, workspace_id, user_id)
        .await
        .map_err(db_error)?;

    if !access {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    // Subscribe to server status updates
    let channel = format!("server:{}:status", server_id);
    let rx = state.ws_manager.subscribe(&channel).await
        .map_err(|e| service_error("WebSocket", e))?;

    let ws_manager = state.ws_manager.clone();
    Ok(ws.on_upgrade(move |socket| handle_server_status_socket(socket, rx, server_id, ws_manager)))
}

/// WebSocket handler for server logs streaming
pub async fn server_logs_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<WsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Rate limit WebSocket connection attempts
    let client_ip = extract_client_ip(&headers, &addr);
    if !check_ws_connection_rate_limit(&state.redis, &client_ip)
        .await
        .unwrap_or(true)
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many connection attempts".to_string(),
        ));
    }

    // Extract token from query parameter first (for cross-origin), then fallback to Cookie
    let token = query.token
        .or_else(|| {
            headers
                .get(COOKIE)
                .and_then(|h| h.to_str().ok())
                .and_then(extract_token_from_cookie)
                .map(|s| s.to_string())
        })
        .ok_or((StatusCode::UNAUTHORIZED, "Missing access token".to_string()))?;

    // Verify JWT token (including revocation, e.g. after logout)
    let user_id = verify_ws_token(&state, &token).await?;

    // Verify server exists, belongs to workspace, and user has access with optimized query
    let access = ServerRepository::check_user_access(&state.db, server_id, workspace_id, user_id)
        .await
        .map_err(db_error)?;

    if !access {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    // Subscribe to server logs
    // Channel format must match EventPublisher: "ws:server:logs:{id}" -> "server:logs:{id}"
    let channel = format!("server:logs:{}", server_id);
    let rx = state.ws_manager.subscribe(&channel).await
        .map_err(|e| service_error("WebSocket", e))?;

    let ws_manager = state.ws_manager.clone();
    Ok(ws.on_upgrade(move |socket| handle_logs_socket(socket, rx, server_id, ws_manager)))
}

/// WebSocket handler for build logs streaming
pub async fn build_logs_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(deployment_id): Path<Uuid>,
    Query(query): Query<WsQuery>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Rate limit WebSocket connection attempts
    let client_ip = extract_client_ip(&headers, &addr);
    if !check_ws_connection_rate_limit(&state.redis, &client_ip)
        .await
        .unwrap_or(true)
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many connection attempts".to_string(),
        ));
    }

    // Extract token from query parameter first (for cross-origin), then fallback to Cookie
    let token = query.token
        .or_else(|| {
            headers
                .get(COOKIE)
                .and_then(|h| h.to_str().ok())
                .and_then(extract_token_from_cookie)
                .map(|s| s.to_string())
        })
        .ok_or((StatusCode::UNAUTHORIZED, "Missing access token".to_string()))?;

    // Verify JWT token (including revocation, e.g. after logout)
    let user_id = verify_ws_token(&state, &token).await?;

    // Verify deployment exists and user has access with optimized query
    let access = DeploymentRepository::check_user_access(&state.db, deployment_id, user_id)
        .await
        .map_err(db_error)?;

    if !access {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    // Subscribe to build logs
    // Channel format must match EventPublisher: "ws:deployment:logs:{id}" -> "deployment:logs:{id}"
    let channel = format!("deployment:logs:{}", deployment_id);
    let rx = state.ws_manager.subscribe(&channel).await
        .map_err(|e| service_error("WebSocket", e))?;

    let ws_manager = state.ws_manager.clone();
    Ok(ws.on_upgrade(move |socket| handle_build_logs_socket(socket, rx, deployment_id, ws_manager)))
}

/// Handle deployment status WebSocket connection
async fn handle_deployment_socket(
    socket: WebSocket,
    mut rx: broadcast::Receiver<WsMessage>,
    deployment_id: Uuid,
    ws_manager: WsManager,
) {
    let (mut sender, mut receiver) = socket.split();

    loop {
        tokio::select! {
            // Handle broadcast messages from Redis
            msg = rx.recv() => {
                match msg {
                    Ok(ws_msg) => {
                        let json = match serde_json::to_string(&ws_msg) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::error!("Failed to serialize WebSocket message: {}", e);
                                continue;
                            }
                        };
                        if sender.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
            // Handle client messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle app-level ping
                        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                            if matches!(ws_msg, WsMessage::Ping) {
                                tracing::debug!("Received app-level ping for deployment {}", deployment_id);
                                let pong = serde_json::to_string(&WsMessage::Pong).unwrap_or_default();
                                if sender.send(Message::Text(pong)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    // Decrement connection count
    ws_manager.on_disconnect();
    tracing::info!("WebSocket connection closed for deployment {}", deployment_id);
}

/// Handle server status WebSocket connection
async fn handle_server_status_socket(
    socket: WebSocket,
    mut rx: broadcast::Receiver<WsMessage>,
    server_id: Uuid,
    ws_manager: WsManager,
) {
    let (mut sender, mut receiver) = socket.split();

    loop {
        tokio::select! {
            // Handle broadcast messages from Redis
            msg = rx.recv() => {
                match msg {
                    Ok(ws_msg) => {
                        let json = match serde_json::to_string(&ws_msg) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::error!("Failed to serialize WebSocket message: {}", e);
                                continue;
                            }
                        };
                        if sender.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
            // Handle client messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle app-level ping
                        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                            if matches!(ws_msg, WsMessage::Ping) {
                                let pong = serde_json::to_string(&WsMessage::Pong).unwrap_or_default();
                                if sender.send(Message::Text(pong)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    // Decrement connection count
    ws_manager.on_disconnect();
    tracing::info!("WebSocket connection closed for server status {}", server_id);
}

/// Handle server logs WebSocket connection
async fn handle_logs_socket(
    socket: WebSocket,
    mut rx: broadcast::Receiver<WsMessage>,
    server_id: Uuid,
    ws_manager: WsManager,
) {
    let (mut sender, mut receiver) = socket.split();

    loop {
        tokio::select! {
            // Handle broadcast messages from Redis
            msg = rx.recv() => {
                match msg {
                    Ok(ws_msg) => {
                        let json = match serde_json::to_string(&ws_msg) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::error!("Failed to serialize WebSocket message: {}", e);
                                continue;
                            }
                        };
                        if sender.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
            // Handle client messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle app-level ping
                        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                            if matches!(ws_msg, WsMessage::Ping) {
                                let pong = serde_json::to_string(&WsMessage::Pong).unwrap_or_default();
                                if sender.send(Message::Text(pong)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    // Decrement connection count
    ws_manager.on_disconnect();
    tracing::info!("WebSocket connection closed for server logs {}", server_id);
}

/// Handle build logs WebSocket connection
async fn handle_build_logs_socket(
    socket: WebSocket,
    mut rx: broadcast::Receiver<WsMessage>,
    deployment_id: Uuid,
    ws_manager: WsManager,
) {
    let (mut sender, mut receiver) = socket.split();

    loop {
        tokio::select! {
            // Handle broadcast messages from Redis
            msg = rx.recv() => {
                match msg {
                    Ok(ws_msg) => {
                        let json = match serde_json::to_string(&ws_msg) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::error!("Failed to serialize WebSocket message: {}", e);
                                continue;
                            }
                        };
                        if sender.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
            // Handle client messages
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle app-level ping
                        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                            if matches!(ws_msg, WsMessage::Ping) {
                                let pong = serde_json::to_string(&WsMessage::Pong).unwrap_or_default();
                                if sender.send(Message::Text(pong)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    // Decrement connection count
    ws_manager.on_disconnect();
    tracing::info!("WebSocket connection closed for build logs {}", deployment_id);
}

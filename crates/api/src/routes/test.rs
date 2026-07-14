//! MCP Server Test Endpoints
//!
//! Allows users to test their deployed MCP servers directly from the dashboard
//! without needing to connect Claude.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use mcp_db::WorkspaceRepository;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::db_error;
use crate::extractors::AuthUser;
use crate::state::AppState;
use crate::routes::helpers::{fetch_server_for_workspace, ERR_INSUFFICIENT_PERMISSIONS, ERR_NOT_A_MEMBER, ERR_SERVER_NOT_FOUND};

/// JSON-RPC request structure
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u32,
    method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

/// JSON-RPC response structure
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: serde_json::Value,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

/// Parse SSE response format and extract JSON data
/// SSE format: "event: message\ndata: {json}\n\n"
fn parse_sse_response(body: &str) -> Option<String> {
    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            return Some(data.to_string());
        }
    }
    // If no SSE format found, try to parse as plain JSON
    if body.trim().starts_with('{') {
        return Some(body.trim().to_string());
    }
    None
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthCheckResponse {
    pub status: String,
    pub endpoint_url: Option<String>,
    pub connection: ConnectionStatus,
    pub tools: Option<Vec<ToolInfo>>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConnectionStatus {
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub mcp_version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
}

/// Tool execution request
#[derive(Debug, Deserialize)]
pub struct ExecuteToolRequest {
    pub tool_name: String,
    pub arguments: Option<serde_json::Value>,
}

/// Tool execution response
#[derive(Debug, Serialize)]
pub struct ExecuteToolResponse {
    pub success: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub latency_ms: u64,
}

/// Test MCP server connection and list tools
pub async fn health_check(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<HealthCheckResponse>, (StatusCode, String)> {
    // Check membership
    WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    // Get server (also verifies it belongs to this workspace)
    let server = fetch_server_for_workspace(&state, workspace_id, server_id).await?;

    // Check if server has endpoint URL
    let base_endpoint_url = match &server.endpoint_url {
        Some(url) => url.clone(),
        None => {
            return Ok(Json(HealthCheckResponse {
                status: "not_deployed".to_string(),
                endpoint_url: None,
                connection: ConnectionStatus {
                    reachable: false,
                    latency_ms: None,
                    mcp_version: None,
                },
                tools: None,
                error: Some("Server is not deployed yet".to_string()),
            }));
        }
    };

    // Construct full endpoint URL with mcp_path
    let endpoint_url = {
        let base = base_endpoint_url.trim_end_matches('/');
        let path = server.mcp_path.trim_start_matches('/');
        if path.is_empty() {
            base.to_string()
        } else {
            format!("{}/{}", base, path)
        }
    };

    // Create HTTP client with timeout (60s to allow for cold starts)
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let start = std::time::Instant::now();
    tracing::debug!("[TEST] Starting health check for endpoint: {}", endpoint_url);

    // First, try to initialize the connection
    let init_request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "nodeflare-test",
                "version": "1.0.0"
            }
        })),
    };

    // Retry logic for cold start handling (Fly.io may return 500 while machine is waking up)
    const MAX_RETRIES: u32 = 3;
    const RETRY_DELAY_MS: u64 = 1000;

    let mut init_response: Option<Result<reqwest::Response, reqwest::Error>> = None;
    let mut last_error_msg: Option<String> = None;

    for attempt in 1..=MAX_RETRIES {
        tracing::debug!("[TEST] Sending initialize request (attempt {}/{})...", attempt, MAX_RETRIES);
        let resp = client
            .post(&endpoint_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&init_request)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                tracing::debug!("[TEST] Initialize succeeded on attempt {}", attempt);
                init_response = Some(Ok(r));
                break;
            }
            Ok(r) if r.status().is_server_error() && attempt < MAX_RETRIES => {
                // 5xx error - likely cold start, retry after delay
                tracing::debug!("[TEST] Initialize got {} on attempt {}, retrying...", r.status(), attempt);
                last_error_msg = Some(format!("HTTP error: {}", r.status()));
                tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
            }
            Ok(r) => {
                // Non-5xx error or last attempt with 5xx
                init_response = Some(Ok(r));
                break;
            }
            Err(e) if attempt < MAX_RETRIES && (e.is_connect() || e.is_timeout()) => {
                // Connection/timeout error - retry
                tracing::debug!("[TEST] Initialize error on attempt {}: {:?}, retrying...", attempt, e);
                last_error_msg = Some(e.to_string());
                tokio::time::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS)).await;
            }
            Err(e) => {
                init_response = Some(Err(e));
                break;
            }
        }
    }

    tracing::debug!("[TEST] Initialize response received in {}ms", start.elapsed().as_millis());

    // If all retries exhausted with 5xx/connection errors, return error response
    if init_response.is_none() {
        let latency_ms = start.elapsed().as_millis() as u64;
        return Ok(Json(HealthCheckResponse {
            status: "error".to_string(),
            endpoint_url: Some(endpoint_url),
            connection: ConnectionStatus {
                reachable: false,
                latency_ms: Some(latency_ms),
                mcp_version: None,
            },
            tools: None,
            error: Some(last_error_msg.unwrap_or_else(|| "Connection failed after retries".to_string())),
        }));
    }

    let init_response = init_response.unwrap();

    // Extract session ID and parse SSE response
    let (mcp_version, session_id) = match init_response {
        Ok(resp) if resp.status().is_success() => {
            tracing::debug!("[TEST] Initialize HTTP status: {}", resp.status());
            // Extract session ID from headers
            let session_id = resp
                .headers()
                .get("mcp-session-id")
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            tracing::debug!("[TEST] Session ID: {:?}", session_id);

            // Parse SSE response body
            let body = resp.text().await.unwrap_or_default();
            tracing::debug!("[TEST] Initialize response body: {}", &body[..body.len().min(500)]);
            let json_data = parse_sse_response(&body);
            tracing::debug!("[TEST] Parsed JSON data: {:?}", json_data);

            let version = json_data
                .and_then(|json| serde_json::from_str::<JsonRpcResponse>(&json).ok())
                .and_then(|r| r.result)
                .and_then(|r| r.get("protocolVersion").and_then(|v| v.as_str()).map(String::from));
            tracing::debug!("[TEST] MCP version: {:?}", version);

            (version, session_id)
        }
        Ok(resp) => {
            tracing::debug!("[TEST] Initialize failed with HTTP status: {}", resp.status());
            let latency_ms = start.elapsed().as_millis() as u64;
            return Ok(Json(HealthCheckResponse {
                status: "error".to_string(),
                endpoint_url: Some(endpoint_url),
                connection: ConnectionStatus {
                    reachable: true,
                    latency_ms: Some(latency_ms),
                    mcp_version: None,
                },
                tools: None,
                error: Some(format!("HTTP error: {}", resp.status())),
            }));
        }
        Err(e) => {
            tracing::debug!("[TEST] Initialize request error: {:?}", e);
            let error_msg = if e.is_timeout() {
                "Connection timeout".to_string()
            } else if e.is_connect() {
                "Connection refused".to_string()
            } else {
                e.to_string()
            };

            return Ok(Json(HealthCheckResponse {
                status: "unreachable".to_string(),
                endpoint_url: Some(endpoint_url),
                connection: ConnectionStatus {
                    reachable: false,
                    latency_ms: None,
                    mcp_version: None,
                },
                tools: None,
                error: Some(error_msg),
            }));
        }
    };

    // List tools (with session ID if available)
    tracing::debug!("[TEST] Sending tools/list request...");
    let tools_start = std::time::Instant::now();
    let tools_request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/list",
        params: None,
    };

    let mut request_builder = client
        .post(&endpoint_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");

    // Add session ID header if we got one from initialize
    if let Some(ref sid) = session_id {
        request_builder = request_builder.header("Mcp-Session-Id", sid);
    }

    let response = request_builder.json(&tools_request).send().await;
    tracing::debug!("[TEST] tools/list response received in {}ms", tools_start.elapsed().as_millis());

    let latency_ms = start.elapsed().as_millis() as u64;
    tracing::debug!("[TEST] Total health check latency: {}ms", latency_ms);

    match response {
        Ok(resp) => {
            if resp.status().is_success() {
                // Parse SSE response body
                let body = resp.text().await.unwrap_or_default();
                let json_data = parse_sse_response(&body);

                match json_data.and_then(|d| serde_json::from_str::<JsonRpcResponse>(&d).ok()) {
                    Some(json_rpc) => {
                        if let Some(error) = json_rpc.error {
                            Ok(Json(HealthCheckResponse {
                                status: "error".to_string(),
                                endpoint_url: Some(endpoint_url),
                                connection: ConnectionStatus {
                                    reachable: true,
                                    latency_ms: Some(latency_ms),
                                    mcp_version,
                                },
                                tools: None,
                                error: Some(format!("MCP error {}: {}", error.code, error.message)),
                            }))
                        } else {
                            let tools = json_rpc.result
                                .and_then(|r| r.get("tools").cloned())
                                .and_then(|t| serde_json::from_value::<Vec<ToolInfo>>(t).ok());

                            Ok(Json(HealthCheckResponse {
                                status: "healthy".to_string(),
                                endpoint_url: Some(endpoint_url),
                                connection: ConnectionStatus {
                                    reachable: true,
                                    latency_ms: Some(latency_ms),
                                    mcp_version,
                                },
                                tools,
                                error: None,
                            }))
                        }
                    }
                    None => Ok(Json(HealthCheckResponse {
                        status: "error".to_string(),
                        endpoint_url: Some(endpoint_url),
                        connection: ConnectionStatus {
                            reachable: true,
                            latency_ms: Some(latency_ms),
                            mcp_version,
                        },
                        tools: None,
                        error: Some(format!("Invalid response format: {}", body)),
                    })),
                }
            } else {
                // Try to get error details from response body
                let body = resp.text().await.unwrap_or_default();
                Ok(Json(HealthCheckResponse {
                    status: "error".to_string(),
                    endpoint_url: Some(endpoint_url),
                    connection: ConnectionStatus {
                        reachable: true,
                        latency_ms: Some(latency_ms),
                        mcp_version,
                    },
                    tools: None,
                    error: Some(format!("HTTP error: {}", body)),
                }))
            }
        }
        Err(e) => {
            let error_msg = if e.is_timeout() {
                "Connection timeout".to_string()
            } else if e.is_connect() {
                "Connection refused".to_string()
            } else {
                e.to_string()
            };

            Ok(Json(HealthCheckResponse {
                status: "unreachable".to_string(),
                endpoint_url: Some(endpoint_url),
                connection: ConnectionStatus {
                    reachable: false,
                    latency_ms: None,
                    mcp_version: None,
                },
                tools: None,
                error: Some(error_msg),
            }))
        }
    }
}

/// Execute a specific tool on the deployed MCP server
pub async fn execute_tool(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Path((workspace_id, server_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<ExecuteToolRequest>,
) -> Result<Json<ExecuteToolResponse>, (StatusCode, String)> {
    // Check membership and write permission
    let member = WorkspaceRepository::get_member(&state.db, workspace_id, auth_user.user_id)
        .await
        .map_err(db_error)?
        .ok_or((StatusCode::FORBIDDEN, ERR_NOT_A_MEMBER.to_string()))?;

    if matches!(member.role(), mcp_common::types::WorkspaceRole::Viewer) {
        return Err((StatusCode::FORBIDDEN, ERR_INSUFFICIENT_PERMISSIONS.to_string()));
    }

    // Get server (also verifies it belongs to this workspace)
    let server = fetch_server_for_workspace(&state, workspace_id, server_id).await?;

    let base_endpoint_url = server
        .endpoint_url
        .ok_or((StatusCode::BAD_REQUEST, "Server is not deployed".to_string()))?;

    // Construct full endpoint URL with mcp_path
    let endpoint_url = {
        let base = base_endpoint_url.trim_end_matches('/');
        let path = server.mcp_path.trim_start_matches('/');
        if path.is_empty() {
            base.to_string()
        } else {
            format!("{}/{}", base, path)
        }
    };

    // Create HTTP client with timeout (60s to allow for cold starts)
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let start = std::time::Instant::now();

    // First, initialize to get session ID
    let init_request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "nodeflare-test",
                "version": "1.0.0"
            }
        })),
    };

    let init_response = client
        .post(&endpoint_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&init_request)
        .send()
        .await;

    let session_id = match init_response {
        Ok(resp) if resp.status().is_success() => {
            resp.headers()
                .get("mcp-session-id")
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        }
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            return Ok(Json(ExecuteToolResponse {
                success: false,
                result: None,
                error: Some(format!("Initialize failed: HTTP {}", resp.status())),
                latency_ms,
            }));
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            return Ok(Json(ExecuteToolResponse {
                success: false,
                result: None,
                error: Some(format!("Initialize failed: {}", e)),
                latency_ms,
            }));
        }
    };

    // Call tool
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: Some(serde_json::json!({
            "name": body.tool_name,
            "arguments": body.arguments.unwrap_or(serde_json::json!({}))
        })),
    };

    let mut request_builder = client
        .post(&endpoint_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");

    if let Some(ref sid) = session_id {
        request_builder = request_builder.header("Mcp-Session-Id", sid);
    }

    let response = request_builder.json(&request).send().await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match response {
        Ok(resp) => {
            if resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                let json_data = parse_sse_response(&body);

                match json_data.and_then(|d| serde_json::from_str::<JsonRpcResponse>(&d).ok()) {
                    Some(json_rpc) => {
                        if let Some(error) = json_rpc.error {
                            Ok(Json(ExecuteToolResponse {
                                success: false,
                                result: None,
                                error: Some(format!("{}: {}", error.code, error.message)),
                                latency_ms,
                            }))
                        } else {
                            Ok(Json(ExecuteToolResponse {
                                success: true,
                                result: json_rpc.result,
                                error: None,
                                latency_ms,
                            }))
                        }
                    }
                    None => Ok(Json(ExecuteToolResponse {
                        success: false,
                        result: None,
                        error: Some(format!("Invalid response format: {}", body)),
                        latency_ms,
                    })),
                }
            } else {
                let body = resp.text().await.unwrap_or_default();
                Ok(Json(ExecuteToolResponse {
                    success: false,
                    result: None,
                    error: Some(format!("HTTP error: {}", body)),
                    latency_ms,
                }))
            }
        }
        Err(e) => {
            let error_msg = if e.is_timeout() {
                "Request timeout".to_string()
            } else if e.is_connect() {
                "Connection failed".to_string()
            } else {
                e.to_string()
            };

            Ok(Json(ExecuteToolResponse {
                success: false,
                result: None,
                error: Some(error_msg),
                latency_ms,
            }))
        }
    }
}

//! Unix Domain Socket HTTP client for proxy→container communication.
//!
//! Target URL encoding: `unix:SOCKET_PATH|HTTP_PATH`
//! Example: `unix:/run/mcp/my-server.sock|/mcp?session=abc`
//!
//! Activated when `endpoint_url` in the DB starts with `unix:`.
//! Falls back to the reqwest TCP path for all other URL schemes.

use bytes::Bytes;
use futures::TryStreamExt;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper::Request;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;

use crate::ProxyError;

/// Parse a `unix:SOCKET_PATH|HTTP_PATH` target URL.
/// Returns `(socket_path, http_path)` on success.
pub fn parse_target(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("unix:")?;
    let pipe = rest.find('|')?;
    Some((&rest[..pipe], &rest[pipe + 1..]))
}

fn to_hyper_method(method: &axum::http::Method) -> hyper::Method {
    hyper::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(hyper::Method::POST)
}

async fn connect(socket_path: &str) -> Result<http1::SendRequest<Full<Bytes>>, ProxyError> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| ProxyError::ServiceUnavailable(format!("Unix socket connect ({}): {}", socket_path, e)))?;
    let io = TokioIo::new(stream);

    let (sender, conn) = http1::handshake(io)
        .await
        .map_err(|e| ProxyError::Internal(format!("HTTP/1 handshake on {}: {}", socket_path, e)))?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::debug!("unix socket connection closed: {}", e);
        }
    });

    Ok(sender)
}

fn build_request(
    method: &axum::http::Method,
    http_path: &str,
    headers: &axum::http::HeaderMap,
    body: Bytes,
) -> Result<Request<Full<Bytes>>, ProxyError> {
    let mut req_builder = Request::builder()
        .method(to_hyper_method(method))
        .uri(http_path)
        .header("host", "localhost");

    for (name, value) in headers.iter() {
        req_builder = req_builder.header(name.as_str(), value.as_bytes());
    }

    req_builder
        .body(Full::new(body))
        .map_err(|e| ProxyError::Internal(format!("Failed to build unix request: {}", e)))
}

/// Buffered request via Unix socket.
/// Signature mirrors `execute_upstream_request`.
pub async fn send_buffered(
    socket_path: &str,
    http_path: &str,
    method: axum::http::Method,
    headers: &axum::http::HeaderMap,
    body: Bytes,
    max_response_bytes: usize,
) -> Result<(Vec<u8>, u16, Vec<(String, String)>), ProxyError> {
    let mut sender = connect(socket_path).await?;
    let request = build_request(&method, http_path, headers, body)?;

    let response = sender
        .send_request(request)
        .await
        .map_err(|e| ProxyError::ServiceUnavailable(format!("Unix socket request error: {}", e)))?;

    let status = response.status().as_u16();

    let resp_headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            let name = k.as_str();
            match name {
                "content-type" | "content-encoding" | "cache-control" | "etag" | "vary"
                | "x-request-id" | "mcp-session-id" => {
                    v.to_str().ok().map(|val| (name.to_string(), val.to_string()))
                }
                _ => None,
            }
        })
        .collect();

    // Collect body with DoS size limit
    let mut buf: Vec<u8> = Vec::new();
    let mut body_stream = response.into_body();
    loop {
        match body_stream.frame().await {
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    if buf.len() + data.len() > max_response_bytes {
                        return Err(ProxyError::ServiceUnavailable(format!(
                            "Upstream response exceeded {} byte limit",
                            max_response_bytes
                        )));
                    }
                    buf.extend_from_slice(&data);
                }
            }
            Some(Err(e)) => {
                return Err(ProxyError::Internal(format!(
                    "Failed to read unix socket response: {}",
                    e
                )));
            }
            None => break,
        }
    }

    Ok((buf, status, resp_headers))
}

/// Streaming request via Unix socket.
/// Signature mirrors `execute_streaming_request`.
pub async fn send_streaming(
    socket_path: &str,
    http_path: &str,
    method: axum::http::Method,
    headers: &axum::http::HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, ProxyError> {
    let mut sender = connect(socket_path).await?;
    let request = build_request(&method, http_path, headers, body)?;

    let response = sender
        .send_request(request)
        .await
        .map_err(|e| ProxyError::ServiceUnavailable(format!("Unix socket streaming error: {}", e)))?;

    let status = response.status().as_u16();
    let mut builder = axum::response::Response::builder().status(status);

    let mut has_content_type = false;
    for (name, value) in response.headers().iter() {
        let header_name = name.as_str();
        match header_name {
            "content-type" => {
                if let Ok(val) = value.to_str() {
                    builder = builder.header(header_name, val);
                    has_content_type = true;
                }
            }
            "content-encoding" | "cache-control" | "x-request-id" | "mcp-session-id"
            | "x-accel-buffering" => {
                if let Ok(val) = value.to_str() {
                    builder = builder.header(header_name, val);
                }
            }
            _ => {}
        }
    }

    if !has_content_type {
        builder = builder.header("content-type", "text/event-stream");
    }
    builder = builder.header("cache-control", "no-cache");
    builder = builder.header("x-accel-buffering", "no");

    let data_stream = response
        .into_body()
        .into_data_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
    let axum_body = axum::body::Body::from_stream(data_stream);

    builder
        .body(axum_body)
        .map_err(|e| ProxyError::Internal(format!("Failed to build streaming response: {}", e)))
}

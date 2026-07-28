//! Token-reduction transforms for MCP list responses.
//!
//! The proxy can shrink the tool-schema payload an AI client loads from a
//! `tools/list` response by (a) filtering out tools the caller's credential may not
//! call, and (b) optionally trimming verbose schemas. The response may arrive as
//! `application/json` (a single JSON-RPC message) or `text/event-stream` (one or more
//! SSE `data:` frames, as in MCP Streamable HTTP). We handle both and re-emit in the
//! same framing.
//!
//! Every step is **fail-open**: on any parse uncertainty we return the original bytes
//! unchanged, so a transform bug can never corrupt a response or drop tools the client
//! needs. Filtering only ever *removes* tools the caller already cannot call (the
//! call-time scope check is the real gate); slimming only shortens descriptions.

use crate::auth::AuthCredential;
use mcp_common::McpMethod;
use serde_json::Value;

/// Max description length kept when `slim` is enabled.
const MAX_DESCRIPTION_LEN: usize = 500;

/// A tool observed in a list response, used to populate the server's tool catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<Value>,
}

/// Apply scope filtering and/or schema slimming to a `tools/list` response body.
///
/// - `content_type`: the upstream response content-type, used to pick JSON vs SSE.
/// - `credential`: the calling credential (NodeFlare-auth mode); `None` in
///   pass-through mode, where we never filter (the upstream did its own auth).
/// - `filter_by_scope`: drop tools the credential may not call.
/// - `slim`: trim long tool descriptions.
pub fn transform_tools_list(
    body: &[u8],
    content_type: Option<&str>,
    credential: Option<&AuthCredential>,
    filter_by_scope: bool,
    slim: bool,
) -> Vec<u8> {
    // Build the checker once outside the closure — is_method_allowed calls scope_checker()
    // which deserializes scopes from JSON on every call. With N tools in a tools/list,
    // this would otherwise do N JSON deserializations per request.
    let allow: Option<Box<dyn Fn(&str) -> bool>> = if filter_by_scope {
        credential.map(|cred| {
            let checker = cred.scope_checker();
            Box::new(move |name: &str| checker.is_allowed(McpMethod::ToolsCall, Some(name)))
                as Box<dyn Fn(&str) -> bool>
        })
    } else {
        None
    };
    transform_with(body, content_type, allow.as_deref(), slim)
}

/// Core transform parameterized by an `allow` predicate so the filtering logic is
/// unit-testable without constructing an `AuthCredential`.
fn transform_with(
    body: &[u8],
    content_type: Option<&str>,
    allow: Option<&dyn Fn(&str) -> bool>,
    slim: bool,
) -> Vec<u8> {
    if allow.is_none() && !slim {
        return body.to_vec();
    }
    rewrite_jsonrpc(body, content_type, &|msg| transform_message(msg, allow, slim))
}

/// Replace the `result.tools` array of a `tools/list` response with `replacement`,
/// preserving JSON/SSE framing. Used by search mode to collapse the tool surface into
/// a fixed set of meta-tools. Fail-open: returns the input unchanged on parse error or
/// if the message has no `result.tools`.
pub fn replace_tools(body: &[u8], content_type: Option<&str>, replacement: &[Value]) -> Vec<u8> {
    rewrite_jsonrpc(body, content_type, &|msg| {
        match msg
            .get_mut("result")
            .and_then(|r| r.get_mut("tools"))
            .and_then(|t| t.as_array_mut())
        {
            Some(tools) => {
                *tools = replacement.to_vec();
                true
            }
            None => false,
        }
    })
}

/// Extract the tools from a `tools/list` response for catalog population.
/// Returns `None` if the body isn't a parseable tools/list result.
pub fn extract_tools(body: &[u8], content_type: Option<&str>) -> Option<Vec<ObservedTool>> {
    let value = parse_first_jsonrpc(body, content_type)?;
    let tools = value.get("result")?.get("tools")?.as_array()?;
    let mut out = Vec::with_capacity(tools.len());
    for t in tools {
        let name = t.get("name")?.as_str()?.to_string();
        let description = t
            .get("description")
            .and_then(|d| d.as_str())
            .map(String::from);
        let input_schema = t.get("inputSchema").cloned();
        out.push(ObservedTool {
            name,
            description,
            input_schema,
        });
    }
    Some(out)
}

/// Transform a single JSON-RPC message in place. Returns true if it changed.
/// Leaves anything that isn't a `tools/list` result untouched.
fn transform_message(msg: &mut Value, allow: Option<&dyn Fn(&str) -> bool>, slim: bool) -> bool {
    let tools = match msg
        .get_mut("result")
        .and_then(|r| r.get_mut("tools"))
        .and_then(|t| t.as_array_mut())
    {
        Some(tools) => tools,
        None => return false,
    };

    let before = tools.len();
    if let Some(allow) = allow {
        tools.retain(|t| match t.get("name").and_then(|n| n.as_str()) {
            Some(name) => allow(name),
            None => true, // unidentifiable tool — keep (fail-open)
        });
    }
    let mut changed = tools.len() != before;

    if slim {
        for t in tools.iter_mut() {
            if let Some(desc) = t.get_mut("description") {
                if let Some(s) = desc.as_str() {
                    if s.len() > MAX_DESCRIPTION_LEN {
                        let cut = floor_char_boundary(s, MAX_DESCRIPTION_LEN);
                        let truncated = format!("{}…", &s[..cut]);
                        *desc = Value::String(truncated);
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

/// Whether a content-type denotes an SSE stream.
fn is_event_stream(content_type: Option<&str>, body: &[u8]) -> bool {
    match content_type {
        Some(ct) => ct.contains("text/event-stream"),
        // No content-type: guess from the body — JSON starts with '{'/'['.
        None => !matches!(first_non_ws(body), Some(b'{') | Some(b'[')),
    }
}

fn first_non_ws(body: &[u8]) -> Option<u8> {
    body.iter().copied().find(|b| !b.is_ascii_whitespace())
}

/// Rewrite every JSON-RPC message in `body` via `f`, preserving JSON/SSE framing.
fn rewrite_jsonrpc(body: &[u8], content_type: Option<&str>, f: &dyn Fn(&mut Value) -> bool) -> Vec<u8> {
    if is_event_stream(content_type, body) {
        rewrite_sse(body, f)
    } else {
        rewrite_json(body, f)
    }
}

fn rewrite_json(body: &[u8], f: &dyn Fn(&mut Value) -> bool) -> Vec<u8> {
    match serde_json::from_slice::<Value>(body) {
        Ok(mut v) => {
            if f(&mut v) {
                serde_json::to_vec(&v).unwrap_or_else(|_| body.to_vec())
            } else {
                body.to_vec()
            }
        }
        Err(_) => body.to_vec(),
    }
}

/// Rewrite the JSON payload of SSE `data:` lines. Operates line-by-line and only
/// touches lines whose payload parses as standalone JSON (the MCP single-line case);
/// anything else is passed through verbatim. Original line endings are preserved.
fn rewrite_sse(body: &[u8], f: &dyn Fn(&mut Value) -> bool) -> Vec<u8> {
    let text = match std::str::from_utf8(body) {
        Ok(t) => t,
        Err(_) => return body.to_vec(),
    };

    let mut out = String::with_capacity(text.len());
    let mut changed = false;

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if let Some(rest) = trimmed.strip_prefix("data:") {
            let payload = rest.strip_prefix(' ').unwrap_or(rest);
            if let Ok(mut v) = serde_json::from_str::<Value>(payload) {
                if f(&mut v) {
                    let ending = &line[trimmed.len()..];
                    out.push_str("data: ");
                    out.push_str(&v.to_string());
                    out.push_str(ending);
                    changed = true;
                    continue;
                }
            }
        }
        out.push_str(line);
    }

    if changed {
        out.into_bytes()
    } else {
        body.to_vec()
    }
}

/// Parse the first JSON-RPC message out of a JSON or SSE body (for catalog reads).
fn parse_first_jsonrpc(body: &[u8], content_type: Option<&str>) -> Option<Value> {
    if is_event_stream(content_type, body) {
        let text = std::str::from_utf8(body).ok()?;
        for line in text.lines() {
            let trimmed = line.trim_end_matches(['\n', '\r']);
            if let Some(rest) = trimmed.strip_prefix("data:") {
                let payload = rest.strip_prefix(' ').unwrap_or(rest);
                if let Ok(v) = serde_json::from_str::<Value>(payload) {
                    if v.get("result").is_some() {
                        return Some(v);
                    }
                }
            }
        }
        None
    } else {
        serde_json::from_slice::<Value>(body).ok()
    }
}

/// Largest index `<= max` that is a UTF-8 char boundary of `s`.
fn floor_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_json(tools: &[(&str, &str)]) -> Vec<u8> {
        let arr: Vec<Value> = tools
            .iter()
            .map(|(n, d)| {
                serde_json::json!({"name": n, "description": d, "inputSchema": {"type": "object"}})
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "result": { "tools": arr }
        }))
        .unwrap()
    }

    #[test]
    fn filters_disallowed_tools_json() {
        let body = list_json(&[("read", "r"), ("write", "w"), ("delete", "d")]);
        let allow = |name: &str| name != "delete";
        let out = transform_with(&body, Some("application/json"), Some(&allow), false);
        let v: Value = serde_json::from_slice(&out).unwrap();
        let names: Vec<&str> = v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["read", "write"]);
    }

    #[test]
    fn filters_disallowed_tools_sse() {
        let inner = String::from_utf8(list_json(&[("read", "r"), ("delete", "d")])).unwrap();
        let body = format!("event: message\ndata: {}\n\n", inner).into_bytes();
        let allow = |name: &str| name != "delete";
        let out = transform_with(&body, Some("text/event-stream"), Some(&allow), false);
        let text = String::from_utf8(out).unwrap();
        // Framing preserved.
        assert!(text.starts_with("event: message\n"));
        assert!(text.ends_with("\n\n"));
        // Only the allowed tool remains.
        assert!(text.contains("\"read\""));
        assert!(!text.contains("\"delete\""));
    }

    #[test]
    fn slims_long_descriptions() {
        let long = "x".repeat(MAX_DESCRIPTION_LEN + 50);
        let body = list_json(&[("a", long.as_str())]);
        let out = transform_with(&body, Some("application/json"), None, true);
        let v: Value = serde_json::from_slice(&out).unwrap();
        let desc = v["result"]["tools"][0]["description"].as_str().unwrap();
        assert!(desc.chars().count() <= MAX_DESCRIPTION_LEN + 1); // +1 for the ellipsis
        assert!(desc.ends_with('…'));
    }

    #[test]
    fn no_op_when_nothing_requested() {
        let body = list_json(&[("a", "d")]);
        let out = transform_with(&body, Some("application/json"), None, false);
        assert_eq!(out, body);
    }

    #[test]
    fn fail_open_on_garbage() {
        let body = b"not json at all".to_vec();
        let allow = |_: &str| false;
        let out = transform_with(&body, Some("application/json"), Some(&allow), true);
        assert_eq!(out, body);
    }

    #[test]
    fn leaves_non_list_messages_untouched() {
        // A tools/call result (no result.tools) must pass through unchanged.
        let body = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "result": { "content": [{"type":"text","text":"hi"}] }
        }))
        .unwrap();
        let allow = |_: &str| false;
        let out = transform_with(&body, Some("application/json"), Some(&allow), true);
        assert_eq!(out, body);
    }

    #[test]
    fn extract_tools_json_and_sse() {
        let body = list_json(&[("read", "r"), ("write", "w")]);
        let from_json = extract_tools(&body, Some("application/json")).unwrap();
        assert_eq!(from_json.len(), 2);
        assert_eq!(from_json[0].name, "read");

        let inner = String::from_utf8(body).unwrap();
        let sse = format!("event: message\ndata: {}\n\n", inner).into_bytes();
        let from_sse = extract_tools(&sse, Some("text/event-stream")).unwrap();
        assert_eq!(from_sse, from_json);
    }
}

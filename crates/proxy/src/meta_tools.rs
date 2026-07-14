//! Search-first "meta-tools" for token-efficient tool discovery.
//!
//! When a server has search mode enabled, the proxy replaces its full `tools/list`
//! with just two synthetic tools:
//!
//! - `search_tools(query)` — searches the server's tool catalog (the `tools` table,
//!   populated by the proxy from observed `tools/list` responses) and returns the
//!   matching tool names, descriptions and input schemas.
//! - `call_tool(name, arguments)` — invokes a real tool by name; the proxy rewrites
//!   the request into an ordinary `tools/call` and forwards it upstream.
//!
//! This keeps the upfront tool-schema token cost roughly constant regardless of how
//! many tools the server exposes (the Cloudflare "portal" pattern). Search here is
//! lexical (keyword) — semantic/embedding search can layer on later.

use mcp_db::{ProxyMetaTool, Tool};
use serde_json::{json, Value};

/// Stable internal handler identifiers — these never change even if the exposed name does.
pub const SEARCH_TOOLS: &str = "search_tools";
pub const CALL_TOOL: &str = "call_tool";

/// Default number of tools returned by a `search_tools` call.
const SEARCH_LIMIT: usize = 10;

/// Build a tool JSON definition from a DB row.
fn def_from_db(m: &ProxyMetaTool) -> Value {
    json!({
        "name": m.name,
        "description": m.description,
        "inputSchema": m.input_schema,
    })
}

/// The two meta-tool definitions built from DB rows (search_tools + call_tool).
pub fn definitions_from_db(db_tools: &[ProxyMetaTool]) -> Vec<Value> {
    db_tools.iter()
        .filter(|m| m.handler_type == SEARCH_TOOLS || m.handler_type == CALL_TOOL)
        .map(def_from_db)
        .collect()
}

/// The `search_tools` definition from the DB row (used by code mode too).
pub fn search_tools_def_from_db(db_tools: &[ProxyMetaTool]) -> Option<Value> {
    db_tools.iter()
        .find(|m| m.handler_type == SEARCH_TOOLS)
        .map(def_from_db)
}

/// Fallback definitions used when the proxy_meta_tools table is not yet populated.
pub fn default_definitions() -> Vec<ProxyMetaTool> {
    vec![
        ProxyMetaTool {
            handler_type: SEARCH_TOOLS.to_string(),
            name: SEARCH_TOOLS.to_string(),
            description: "Search this server's available tools by keyword. Returns matching tool names, descriptions, and input schemas. Use this to discover which tool to call, then invoke it with `call_tool`.".to_string(),
            input_schema: json!({"type":"object","properties":{"query":{"type":"string","description":"Keywords describing the tool or capability you need. Leave empty to list all tools."}},"required":[]}),
            is_enabled: true,
            sort_order: 0,
        },
        ProxyMetaTool {
            handler_type: CALL_TOOL.to_string(),
            name: CALL_TOOL.to_string(),
            description: "Invoke one of this server's tools by name. First use `search_tools` to find the tool's name and input schema.".to_string(),
            input_schema: json!({"type":"object","properties":{"name":{"type":"string","description":"The exact name of the tool to call (as returned by search_tools)."},"arguments":{"type":"object","description":"Arguments object matching the tool's input schema."}},"required":["name"]}),
            is_enabled: true,
            sort_order: 1,
        },
        ProxyMetaTool {
            handler_type: crate::code_mode::RUN_CODE.to_string(),
            name: crate::code_mode::RUN_CODE.to_string(),
            description: "Execute JavaScript that orchestrates this server's tools and return only the result.".to_string(),
            input_schema: json!({"type":"object","properties":{"code":{"type":"string","description":"JavaScript to run."}},"required":["code"]}),
            is_enabled: true,
            sort_order: 2,
        },
    ]
}

/// Rank `tools` against a lexical `query` and return up to `limit` matches.
/// An empty query returns all tools (capped). Name matches outweigh description
/// matches. Ties break alphabetically for stable output.
pub fn rank_tools<'a>(tools: &'a [Tool], query: &str, limit: usize) -> Vec<&'a Tool> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    let mut scored: Vec<(i32, &Tool)> = tools
        .iter()
        .filter_map(|t| {
            if terms.is_empty() {
                return Some((0, t));
            }
            let name = t.name.to_lowercase();
            let desc = t.description.as_deref().unwrap_or("").to_lowercase();
            let mut score = 0;
            for term in &terms {
                if name.contains(term) {
                    score += 2;
                }
                if desc.contains(term) {
                    score += 1;
                }
            }
            if score > 0 {
                Some((score, t))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(limit).map(|(_, t)| t).collect()
}

/// Default search limit (exposed so the caller doesn't hard-code it).
pub fn search_limit() -> usize {
    SEARCH_LIMIT
}

/// Build the JSON-RPC response body for a `search_tools` call.
pub fn search_result_json(matched: &[&Tool], id: Option<&Value>) -> Vec<u8> {
    let tools_json: Vec<Value> = matched
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();

    // MCP tool results carry text content; we return the matches as a JSON document so
    // the model can read the names/schemas and then issue a `call_tool`.
    let text = serde_json::to_string(&json!({ "tools": tools_json }))
        .unwrap_or_else(|_| "{\"tools\":[]}".to_string());

    let response = json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "result": {
            "content": [{ "type": "text", "text": text }],
            "isError": false
        }
    });
    serde_json::to_vec(&response).unwrap_or_default()
}

/// Pull the `query` argument out of a `search_tools` request body. Missing/!string
/// query yields an empty string (which `rank_tools` treats as "list all").
pub fn extract_search_query(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("params")
                .and_then(|p| p.get("arguments"))
                .and_then(|a| a.get("query"))
                .and_then(|q| q.as_str())
                .map(String::from)
        })
        .unwrap_or_default()
}

/// Rewrite a `call_tool` wrapper request into an ordinary `tools/call` for the real
/// tool. Accepts the real tool name under `name` (or `tool`) and its arguments under
/// `arguments` (or `args`/`input`). Returns `None` if the shape doesn't match, in
/// which case the caller leaves the request untouched (fail-open).
pub fn rewrite_call_tool_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut v: Value = serde_json::from_slice(body).ok()?;

    let (real_name, real_args) = {
        let params = v.get("params")?.as_object()?;
        let wrapper = params
            .get("arguments")
            .or_else(|| params.get("args"))?
            .as_object()?;
        let real_name = wrapper
            .get("name")
            .or_else(|| wrapper.get("tool"))
            .and_then(|n| n.as_str())?
            .to_string();
        let real_args = wrapper
            .get("arguments")
            .or_else(|| wrapper.get("args"))
            .or_else(|| wrapper.get("input"))
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        (real_name, real_args)
    };

    let obj = v.as_object_mut()?;
    let mut new_params = serde_json::Map::new();
    new_params.insert("name".to_string(), Value::String(real_name));
    new_params.insert("arguments".to_string(), real_args);
    obj.insert("params".to_string(), Value::Object(new_params));

    serde_json::to_vec(&v).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, desc: &str) -> Tool {
        Tool {
            id: uuid::Uuid::nil(),
            server_id: uuid::Uuid::nil(),
            name: name.to_string(),
            description: Some(desc.to_string()),
            input_schema: Some(json!({"type": "object"})),
            enabled: true,
            permission_level: "normal".to_string(),
            rate_limit_per_minute: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn definitions_are_the_two_meta_tools() {
        let defaults = default_definitions();
        let defs = definitions_from_db(&defaults);
        let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec![SEARCH_TOOLS, CALL_TOOL]);
    }

    #[test]
    fn rank_prioritizes_name_matches_and_filters() {
        let tools = vec![
            tool("create_issue", "Open a new ticket"),
            tool("list_repos", "List repositories"),
            tool("search_code", "Search code in a repo"),
        ];
        let hits = rank_tools(&tools, "repo", 10);
        // "list_repos" (name+desc) outranks "search_code" (desc only); "create_issue" excluded.
        let names: Vec<&str> = hits.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["list_repos", "search_code"]);
    }

    #[test]
    fn empty_query_returns_all_capped() {
        let tools: Vec<Tool> = (0..15).map(|i| tool(&format!("t{i}"), "d")).collect();
        let hits = rank_tools(&tools, "   ", 10);
        assert_eq!(hits.len(), 10);
    }

    #[test]
    fn rewrites_call_tool_wrapper_into_real_call() {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": "call_tool", "arguments": { "name": "create_issue", "arguments": { "title": "bug" } } }
        }))
        .unwrap();
        let out = rewrite_call_tool_body(&body).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["params"]["name"], "create_issue");
        assert_eq!(v["params"]["arguments"]["title"], "bug");
        assert_eq!(v["id"], 7); // id preserved for correlation
    }

    #[test]
    fn rewrite_returns_none_on_bad_shape() {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "call_tool", "arguments": { "no_name": true } }
        }))
        .unwrap();
        assert!(rewrite_call_tool_body(&body).is_none());
    }

    #[test]
    fn search_result_has_text_content_listing_tools() {
        let tools = vec![tool("a", "first"), tool("b", "second")];
        let refs: Vec<&Tool> = tools.iter().collect();
        let out = search_result_json(&refs, Some(&json!(3)));
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["id"], 3);
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["tools"].as_array().unwrap().len(), 2);
    }
}

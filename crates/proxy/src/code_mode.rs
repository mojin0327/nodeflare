//! Code-execution mode surface.
//!
//! In code mode the proxy exposes two tools instead of the raw catalog:
//! - `search_tools` (reused from [`crate::meta_tools`]) for discovery, and
//! - `run_code`, which executes AI-written JavaScript in a locked-down sandbox. The
//!   code calls tools via an injected `tools.<name>(args)` API; the sandbox can only
//!   reach the proxy's internal tool-call endpoint, and that endpoint — not the
//!   injected wrapper — is the security boundary (it re-checks scope per call).
//!
//! This module holds the pure pieces (tool definitions, TS API generation, argument
//! extraction, result shaping); execution is delegated to [`crate::code_runner`].

use crate::meta_tools;
use mcp_db::{ProxyMetaTool, Tool};
use serde_json::{json, Value};

pub const RUN_CODE: &str = "run_code";

/// Tools exposed in code mode from DB: discovery (`search_tools`) + execution (`run_code`).
pub fn definitions_from_db(db_tools: &[ProxyMetaTool]) -> Vec<Value> {
    let mut defs = Vec::new();
    if let Some(s) = meta_tools::search_tools_def_from_db(db_tools) {
        defs.push(s);
    }
    if let Some(r) = db_tools.iter().find(|m| m.handler_type == RUN_CODE) {
        defs.push(json!({
            "name": r.name,
            "description": r.description,
            "inputSchema": r.input_schema,
        }));
    }
    defs
}

/// Extract the `code` argument from a `run_code` tools/call body.
pub fn extract_code(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body).ok().and_then(|v| {
        v.get("params")
            .and_then(|p| p.get("arguments"))
            .and_then(|a| a.get("code"))
            .and_then(|c| c.as_str())
            .map(String::from)
    })
}

/// Generate a TypeScript-ish API surface from the tool catalog, for the model to read
/// (via search/describe) before writing code. Keeps the JSON Schema in a doc comment
/// rather than fully translating it — enough for the model, cheap to produce.
pub fn generate_ts_api(tools: &[Tool]) -> String {
    let mut out = String::from("// Available tools (call as `await tools.<name>(args)`):\n\n");
    for t in tools {
        if let Some(desc) = &t.description {
            if !desc.is_empty() {
                out.push_str("/** ");
                out.push_str(&desc.replace('\n', " "));
                out.push_str(" */\n");
            }
        }
        let schema = t
            .input_schema
            .as_ref()
            .and_then(|s| serde_json::to_string(s).ok())
            .unwrap_or_else(|| "{}".to_string());
        out.push_str(&format!(
            "declare function {name}(args: /* {schema} */ Record<string, unknown>): Promise<unknown>;\n\n",
            name = t.name,
            schema = schema,
        ));
    }
    out
}

/// Build a successful `run_code` tools/call result wrapping the runner's output text.
pub fn result_json(output: &str, id: Option<&Value>) -> Vec<u8> {
    let response = json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "result": {
            "content": [{ "type": "text", "text": output }],
            "isError": false
        }
    });
    serde_json::to_vec(&response).unwrap_or_default()
}

/// Build an error `run_code` tools/call result (e.g. runner unavailable / code failed).
pub fn error_json(message: &str, id: Option<&Value>) -> Vec<u8> {
    let response = json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "result": {
            "content": [{ "type": "text", "text": message }],
            "isError": true
        }
    });
    serde_json::to_vec(&response).unwrap_or_default()
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
            input_schema: Some(json!({"type": "object", "properties": {"title": {"type": "string"}}})),
            enabled: true,
            permission_level: "normal".to_string(),
            rate_limit_per_minute: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn definitions_are_search_and_run_code() {
        let defaults = meta_tools::default_definitions();
        let defs = definitions_from_db(&defaults);
        let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec![meta_tools::SEARCH_TOOLS, RUN_CODE]);
    }

    #[test]
    fn extracts_code_argument() {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "run_code", "arguments": { "code": "return 1+1" } }
        }))
        .unwrap();
        assert_eq!(extract_code(&body).as_deref(), Some("return 1+1"));
    }

    #[test]
    fn extract_code_none_when_missing() {
        let body = serde_json::to_vec(&json!({
            "params": { "name": "run_code", "arguments": {} }
        }))
        .unwrap();
        assert!(extract_code(&body).is_none());
    }

    #[test]
    fn ts_api_lists_tools_with_descriptions() {
        let tools = vec![tool("create_issue", "Open a new issue"), tool("list_repos", "List repos")];
        let ts = generate_ts_api(&tools);
        assert!(ts.contains("function create_issue("));
        assert!(ts.contains("Open a new issue"));
        assert!(ts.contains("function list_repos("));
    }

    #[test]
    fn result_and_error_shapes() {
        let ok: Value = serde_json::from_slice(&result_json("hi", Some(&json!(5)))).unwrap();
        assert_eq!(ok["id"], 5);
        assert_eq!(ok["result"]["isError"], false);
        assert_eq!(ok["result"]["content"][0]["text"], "hi");

        let err: Value = serde_json::from_slice(&error_json("boom", None)).unwrap();
        assert_eq!(err["result"]["isError"], true);
        assert_eq!(err["result"]["content"][0]["text"], "boom");
    }
}

//! Client for the sandboxed code runner (Deno service on bare-metal).
//!
//! Optional: if `PROXY_CODE_RUNNER_URL` is unset the client is `None` and code mode
//! degrades gracefully (run_code returns an "unavailable" result). The runner executes
//! AI-written JavaScript with locked-down permissions and calls tools back through the
//! proxy's scope-enforced internal endpoint using the per-execution `token`.

use serde::Serialize;
use serde_json::Value;

#[derive(Clone)]
pub struct CodeRunnerClient {
    http: reqwest::Client,
    /// Base URL of the runner service, e.g. https://mcp-code-runner.internal:8080
    runner_url: String,
    /// Base URL the *sandboxed code* uses to call tools back into this proxy. The
    /// runner injects `tools.*` to POST here; the endpoint re-checks scope per call.
    tools_callback_base: String,
    /// Shared secret presented to the runner's `/run` as `Authorization: Bearer`. The
    /// runner enforces it when set on its side. `None` = not configured (open runner).
    auth_token: Option<String>,
    timeout_secs: u64,
    max_tool_calls: u32,
}

/// Request sent to the runner's `/run` endpoint.
#[derive(Serialize)]
pub struct RunRequest {
    /// JavaScript to execute.
    pub code: String,
    /// Per-execution bearer token the sandbox presents to the tools callback endpoint.
    pub token: String,
    /// Full URL the sandbox calls to invoke a tool (scope-enforced server-side).
    pub tools_endpoint: String,
    /// Limits enforced by the runner.
    pub timeout_secs: u64,
    pub max_tool_calls: u32,
}

impl CodeRunnerClient {
    /// Build from environment. `None` (code mode disabled) when `PROXY_CODE_RUNNER_URL`
    /// is unset/empty.
    pub fn from_env(http: reqwest::Client) -> Option<Self> {
        let runner_url = std::env::var("PROXY_CODE_RUNNER_URL")
            .ok()
            .filter(|u| !u.trim().is_empty())?;
        // Where the sandbox calls back to reach this proxy's tool endpoint. Defaults to
        // the public proxy base if not separately configured.
        let tools_callback_base = std::env::var("PROXY_CODE_TOOLS_CALLBACK_BASE")
            .ok()
            .filter(|u| !u.trim().is_empty())
            .or_else(|| std::env::var("PROXY_PUBLIC_URL").ok())
            .unwrap_or_default();
        let timeout_secs = std::env::var("PROXY_CODE_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(600);
        let max_tool_calls = std::env::var("PROXY_CODE_MAX_TOOL_CALLS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50);
        let auth_token = std::env::var("CODE_RUNNER_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty());
        if auth_token.is_none() {
            tracing::warn!("CODE_RUNNER_TOKEN unset: code runner requests are unauthenticated");
        }
        tracing::info!("code execution enabled: runner={}", runner_url);
        Some(Self {
            http,
            runner_url,
            tools_callback_base,
            auth_token,
            timeout_secs,
            max_tool_calls,
        })
    }

    /// The tools callback URL the sandbox uses for a given server.
    pub fn tools_endpoint(&self, server_id: uuid::Uuid) -> String {
        format!(
            "{}/internal/code-exec/{}/tools-call",
            self.tools_callback_base.trim_end_matches('/'),
            server_id
        )
    }

    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs
    }
    pub fn max_tool_calls(&self) -> u32 {
        self.max_tool_calls
    }

    /// Execute code in the sandbox. Returns the textual result, or an error string.
    pub async fn run(&self, req: RunRequest) -> Result<String, String> {
        let url = format!("{}/run", self.runner_url.trim_end_matches('/'));
        let mut builder = self.http.post(&url).json(&req);
        if let Some(token) = &self.auth_token {
            builder = builder.bearer_auth(token);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| format!("runner unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("runner returned HTTP {}", resp.status()));
        }
        let body: Value = resp.json().await.map_err(|e| format!("bad runner response: {e}"))?;
        if let Some(err) = body.get("error").and_then(|e| e.as_str()) {
            return Err(err.to_string());
        }
        // `output` may be a string or any JSON value; stringify non-strings.
        match body.get("output") {
            Some(Value::String(s)) => Ok(s.clone()),
            Some(other) => Ok(other.to_string()),
            None => Ok(String::new()),
        }
    }
}

# Code-execution mode (layer 3)

Lets the AI write JavaScript that orchestrates a server's tools; the proxy runs it in a
locked-down sandbox and returns only the final result. Cuts tokens for multi-step tasks
by keeping tool schemas and intermediate data out of the model context.

## What's implemented

- **Per-server flag** `tool_code_mode` (default false) — DB, API, settings UI (en/ja).
- **Tool surface** (`crates/proxy/src/code_mode.rs`): in code mode `tools/list` returns
  `search_tools` + `run_code`. `search_tools` returns a generated TypeScript API of
  matching tools (semantic/lexical, scope-filtered) for the model to write code against.
- **run_code handling** (`main.rs::run_code_response`): extracts `code`, calls the
  runner client, wraps the result. Degrades to an error result when no runner is set.
- **Runner client** (`crates/proxy/src/code_runner.rs`): POSTs to `PROXY_CODE_RUNNER_URL`;
  `None` (disabled) when unset.
- **Sandbox service** (`services/code-runner/`): Deno process running on bare-metal via
  `nerdctl run`. Each request runs in a fresh child:
  `deno run --no-prompt --allow-net=<tools-endpoint-host> -` — no fs/env/run, network
  restricted to the proxy tool endpoint only. Injects `tools.*`, enforces tool-call
  count + wall-clock timeout, returns a GUID-framed result.

## Scope-enforced tool-call endpoint (implemented)

The sandbox calls back to `POST /internal/code-exec/{server_id}/tools-call` with
`Authorization: Bearer <exec-token>` and `{tool, arguments}`. This endpoint
(`main.rs::code_exec_tools_call`) is the **security boundary** (not the injected wrapper —
Deno permissions are process-global, so user code can call it directly):

1. **Validates the exec token** → `CodeExecContext { server_id, target_url, scopes }`
   stored in Redis with a short TTL (issued in `run_code_response`, opaque, run-scoped).
   Rejects expired/unknown tokens and server_id mismatch.
2. **Enforces scope per call**: rebuilds `ScopeChecker` from the stored scope strings and
   checks `tools:call:<tool>`; 403 otherwise.
3. **Forwards** a JSON (non-SSE) `tools/call` upstream via `execute_upstream_request` and
   returns the JSON-RPC `result` to the sandbox (error → non-2xx, so the wrapper throws).

Code mode requires NodeFlare auth (the scopes come from the credential); pass-through
servers return an "authentication required" result.

**End-to-end verification still needs a deploy** (Deno runner on Fly + Neon). The Rust
side compiles and unit tests pass; the sandbox runtime and this callback can only be
exercised against the deployed runner.

## Known limitations (v1)

- **Stateful tools**: callbacks are independent of the client's MCP session
  (`initialize`/`mcp-session-id`), so tools that require an established session may not
  work from code.
- **Lexical/semantic discovery** reuses the catalog; brand-new servers populate it on the
  first `tools/list`.

## Config

- `PROXY_CODE_RUNNER_URL` — runner base URL (unset = code mode disabled).
- `PROXY_CODE_TOOLS_CALLBACK_BASE` (or `PROXY_PUBLIC_URL`) — base the sandbox calls back to.
- `PROXY_CODE_TIMEOUT_SECS` (default 15), `PROXY_CODE_MAX_TOOL_CALLS` (default 50).

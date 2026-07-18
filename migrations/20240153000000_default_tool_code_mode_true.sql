-- Change tool_code_mode default to true so newly created servers opt in automatically.
-- Existing servers are also updated since the opt-in barrier is low (runtime disables
-- it anyway when PROXY_CODE_RUNNER_URL is not configured).
ALTER TABLE mcp_servers ALTER COLUMN tool_code_mode SET DEFAULT true;
UPDATE mcp_servers SET tool_code_mode = true WHERE tool_code_mode = false;

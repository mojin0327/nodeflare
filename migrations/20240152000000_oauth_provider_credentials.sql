-- Add per-provider credentials and config to upstream_oauth_providers,
-- replacing the hardcoded env-var lookup and google-specific special cases.
ALTER TABLE upstream_oauth_providers
    ADD COLUMN IF NOT EXISTS client_id     TEXT DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS client_secret TEXT DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS extra_auth_params    JSONB NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS requires_refresh_token BOOLEAN NOT NULL DEFAULT FALSE;

-- Google requires offline access params + refresh token validation.
UPDATE upstream_oauth_providers
SET extra_auth_params       = '{"access_type":"offline","prompt":"consent"}'::jsonb,
    requires_refresh_token  = TRUE
WHERE name = 'google';

-- Trusted redirect hostnames for OAuth dynamic client registration,
-- replacing the hardcoded claude.ai / claude.com allow-list.
CREATE TABLE IF NOT EXISTS oauth_trusted_redirect_hosts (
    hostname   TEXT PRIMARY KEY,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO oauth_trusted_redirect_hosts (hostname)
VALUES ('claude.ai'), ('claude.com')
ON CONFLICT (hostname) DO NOTHING;

-- Configurable proxy meta-tool definitions, replacing hardcoded tool names/schemas
-- in the proxy. Each row maps a stable handler_type to a user-visible name +
-- description + input_schema. The proxy routes by handler_type; the name/description
-- can be changed in the DB without a code deploy.
CREATE TABLE IF NOT EXISTS proxy_meta_tools (
    handler_type  TEXT PRIMARY KEY,          -- stable internal key (search_tools, call_tool, run_code)
    name          TEXT NOT NULL,             -- exposed MCP tool name (may differ from handler_type)
    description   TEXT NOT NULL,
    input_schema  JSONB NOT NULL DEFAULT '{}',
    is_enabled    BOOLEAN NOT NULL DEFAULT TRUE,
    sort_order    INT NOT NULL DEFAULT 0
);

INSERT INTO proxy_meta_tools (handler_type, name, description, input_schema, sort_order)
VALUES
  ('search_tools', 'search_tools',
   'Search this server''s available tools by keyword. Returns matching tool names, descriptions, and input schemas. Use this to discover which tool to call, then invoke it with `call_tool`.',
   '{"type":"object","properties":{"query":{"type":"string","description":"Keywords describing the tool or capability you need. Leave empty to list all tools."}},"required":[]}'::jsonb,
   0),
  ('call_tool', 'call_tool',
   'Invoke one of this server''s tools by name. First use `search_tools` to find the tool''s name and input schema.',
   '{"type":"object","properties":{"name":{"type":"string","description":"The exact name of the tool to call (as returned by search_tools)."},"arguments":{"type":"object","description":"Arguments object matching the tool''s input schema."}},"required":["name"]}'::jsonb,
   1),
  ('run_code', 'run_code',
   'Execute a code snippet in an isolated sandbox and return the output.',
   '{"type":"object","properties":{"language":{"type":"string","description":"Programming language (e.g. python, javascript)."},"code":{"type":"string","description":"The code to execute."}},"required":["language","code"]}'::jsonb,
   2)
ON CONFLICT (handler_type) DO NOTHING;

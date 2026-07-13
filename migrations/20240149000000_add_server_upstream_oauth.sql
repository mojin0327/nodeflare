-- Add upstream OAuth provider configuration to MCP servers.
-- upstream_oauth_provider: 'google', 'github', or NULL (no upstream OAuth needed)
-- upstream_oauth_scopes:   OAuth scopes requested from the upstream provider

ALTER TABLE mcp_servers
    ADD COLUMN upstream_oauth_provider VARCHAR(32)  DEFAULT NULL,
    ADD COLUMN upstream_oauth_scopes   TEXT[]       DEFAULT NULL;

CREATE TABLE server_templates (
    id                      UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    server_id               UUID        NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    owner_id                UUID        NOT NULL REFERENCES users(id),
    workspace_id            UUID        NOT NULL REFERENCES workspaces(id),
    name                    TEXT        NOT NULL,
    description             TEXT,
    github_repo             TEXT        NOT NULL,
    github_branch           TEXT        NOT NULL DEFAULT 'main',
    runtime                 TEXT        NOT NULL DEFAULT 'node',
    transport               TEXT        NOT NULL DEFAULT 'sse',
    mcp_path                TEXT        NOT NULL DEFAULT '/mcp',
    entry_command           TEXT,
    build_command           TEXT,
    root_directory          TEXT        NOT NULL DEFAULT '',
    auth_enabled            BOOLEAN     NOT NULL DEFAULT TRUE,
    memory_mb               INT,
    port                    INT,
    upstream_oauth_provider TEXT,
    upstream_oauth_scopes   TEXT[]      NOT NULL DEFAULT '{}',
    -- env var key names only (no values)
    required_env_var_keys   TEXT[]      NOT NULL DEFAULT '{}',
    use_count               INT         NOT NULL DEFAULT 0,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- 1 template per server
    CONSTRAINT uq_server_templates_server_id UNIQUE (server_id)
);

CREATE INDEX idx_server_templates_created_at ON server_templates (created_at DESC);

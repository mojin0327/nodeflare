-- Provider catalog for upstream OAuth.
-- Managed providers (is_managed=true) use platform env vars:
--   {NAME_UPPER}_CLIENT_ID / {NAME_UPPER}_CLIENT_SECRET
-- Custom provider (is_managed=false) uses per-server credentials
-- stored in the new mcp_servers columns below.

CREATE TABLE upstream_oauth_providers (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name              VARCHAR(64) NOT NULL UNIQUE,   -- slug used in code/DB
    display_name      VARCHAR(128) NOT NULL,
    authorization_url TEXT        NOT NULL,
    token_url         TEXT        NOT NULL,
    default_scopes    TEXT[]      NOT NULL DEFAULT '{}',
    -- true  → NodeFlare holds the OAuth app credentials in env vars
    -- false → per-server credentials required (upstream_oauth_client_* columns)
    is_managed        BOOLEAN     NOT NULL DEFAULT TRUE,
    is_enabled        BOOLEAN     NOT NULL DEFAULT TRUE,
    sort_order        INT         NOT NULL DEFAULT 0,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO upstream_oauth_providers
    (name, display_name, authorization_url, token_url, default_scopes, is_managed, is_enabled, sort_order)
VALUES
    ('google',    'Google',    'https://accounts.google.com/o/oauth2/v2/auth',                          'https://oauth2.googleapis.com/token',                    ARRAY['https://www.googleapis.com/auth/drive.readonly'], TRUE,  TRUE, 10),
    ('github',    'GitHub',    'https://github.com/login/oauth/authorize',                              'https://github.com/login/oauth/access_token',            ARRAY['repo'],                                           TRUE,  TRUE, 20),
    ('microsoft', 'Microsoft', 'https://login.microsoftonline.com/common/oauth2/v2.0/authorize',        'https://login.microsoftonline.com/common/oauth2/v2.0/token', ARRAY['https://graph.microsoft.com/.default'],        TRUE,  TRUE, 30),
    ('slack',     'Slack',     'https://slack.com/oauth/v2/authorize',                                  'https://slack.com/api/oauth.v2.access',                  ARRAY['channels:read'],                                  TRUE,  TRUE, 40),
    ('notion',    'Notion',    'https://api.notion.com/v1/oauth/authorize',                             'https://api.notion.com/v1/oauth/token',                  ARRAY[]::TEXT[],                                         TRUE,  TRUE, 50),
    ('linear',    'Linear',    'https://linear.app/oauth/authorize',                                    'https://api.linear.app/oauth/token',                     ARRAY['read'],                                           TRUE,  TRUE, 60),
    ('custom',    'Custom',    '',                                                                       '',                                                       ARRAY[]::TEXT[],                                         FALSE, TRUE, 999);

-- Per-server custom OAuth credentials (only needed when provider = 'custom').
-- For managed providers these stay NULL.
ALTER TABLE mcp_servers
    ADD COLUMN upstream_oauth_authorization_url TEXT DEFAULT NULL,
    ADD COLUMN upstream_oauth_token_url         TEXT DEFAULT NULL,
    ADD COLUMN upstream_oauth_client_id         TEXT DEFAULT NULL,
    ADD COLUMN upstream_oauth_client_secret     TEXT DEFAULT NULL;

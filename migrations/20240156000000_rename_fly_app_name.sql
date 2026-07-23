-- Rename fly_app_name to container_name now that servers run on bare-metal containerd
-- rather than Fly.io. Value format stays the same: mcp-<uuid-nohyphens>.
ALTER TABLE mcp_servers RENAME COLUMN fly_app_name TO container_name;

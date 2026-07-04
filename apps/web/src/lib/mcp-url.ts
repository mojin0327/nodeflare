/**
 * Builds the public proxy URL for a deployed MCP server.
 *
 * The routing scheme is `https://<workspace-slug>--<server-slug>.<base><mcp-path>`.
 * The two slugs are joined by a DOUBLE hyphen `--`; this separator MUST match the
 * proxy's host-splitting logic in `crates/proxy/src/main.rs` (resolve_server), which
 * splits the leftmost DNS label on `--` to recover the workspace and server slugs.
 */

/** Proxy base domain (host suffix), configurable via env, defaulting to nodeflare.tech. */
function proxyBaseDomain(): string {
  return process.env.NEXT_PUBLIC_PROXY_BASE_DOMAIN || 'nodeflare.tech';
}

/** Normalizes the mcp_path so it always starts with `/`; falsy paths default to `/mcp`. */
function normalizeMcpPath(mcpPath?: string | null): string {
  if (!mcpPath) return '/mcp';
  return mcpPath.startsWith('/') ? mcpPath : `/${mcpPath}`;
}

/**
 * Host-only variant (no scheme, no path) for compact display, e.g.
 * `acme--my-server.nodeflare.tech`.
 */
export function mcpPublicHost(workspaceSlug: string, serverSlug: string): string {
  return `${workspaceSlug}--${serverSlug}.${proxyBaseDomain()}`;
}

/**
 * Full public URL for a deployed MCP server, including scheme and mcp path, e.g.
 * `https://acme--my-server.nodeflare.tech/mcp`.
 */
export function mcpPublicUrl(
  workspaceSlug: string,
  serverSlug: string,
  mcpPath?: string | null
): string {
  return `https://${mcpPublicHost(workspaceSlug, serverSlug)}${normalizeMcpPath(mcpPath)}`;
}

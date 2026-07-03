//! Parsers for the two standardized MCP manifests that *declare* deploy config, the
//! strongest detection signal when present:
//!   - MCP Registry `server.json` (schema 2025-12-11, camelCase)
//!   - Smithery `smithery.yaml`
//!
//! NOTE: these manifests describe how to RUN a *published* package (npx/uvx) or a
//! container — not how to build from source. NodeFlare builds the repo from source and
//! forbids npx in the start command, so we take runtime / transport / mcp_path / env
//! from here (authoritative) but still derive the entry & build commands from the real
//! source files. `is_docker` short-circuits to the repo Dockerfile.

use crate::{EnvVar, Runtime, Transport};

/// Config declared by a manifest. All fields optional — a manifest may declare only some.
#[derive(Debug, Default, Clone)]
pub struct ManifestHints {
    pub runtime: Option<Runtime>,
    pub transport: Option<Transport>,
    pub mcp_path: Option<String>,
    pub port: Option<u16>,
    pub env_vars: Vec<EnvVar>,
    /// Smithery `runtime: container` — build/run is fully owned by the repo Dockerfile.
    pub is_docker: bool,
    /// Which manifest produced these hints, for provenance reporting.
    pub source: &'static str,
}

fn transport_from_type(t: &str) -> Option<Transport> {
    match t.trim().to_ascii_lowercase().as_str() {
        "stdio" => Some(Transport::Stdio),
        "streamable-http" | "http" | "streamablehttp" | "sse" => Some(Transport::Sse),
        _ => None,
    }
}

fn runtime_from_registry_type(rt: &str) -> Option<Runtime> {
    match rt.trim().to_ascii_lowercase().as_str() {
        "npm" => Some(Runtime::Node),
        "pypi" => Some(Runtime::Python),
        "cargo" => Some(Runtime::Rust),
        "oci" => Some(Runtime::Docker),
        _ => None, // nuget (.NET) / mcpb — not a NodeFlare runtime
    }
}

fn runtime_from_hint(hint: &str) -> Option<Runtime> {
    match hint.trim().to_ascii_lowercase().as_str() {
        "npx" => Some(Runtime::Node),
        "uvx" => Some(Runtime::Python),
        _ => None,
    }
}

/// Pull the URL path (e.g. `/mcp`) out of a declared HTTP endpoint URL.
fn path_from_url(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let slash = after_scheme.find('/')?;
    let path = &after_scheme[slash..];
    let path = path.split(['?', '#']).next().unwrap_or(path);
    (!path.is_empty() && path != "/").then(|| path.trim_end_matches('/').to_string())
}

fn env_vars_from_json(arr: &serde_json::Value) -> Vec<EnvVar> {
    arr.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|e| {
                    let key = e.get("name").and_then(|v| v.as_str())?.to_string();
                    Some(EnvVar {
                        key,
                        description: e
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        required: e.get("isRequired").and_then(|v| v.as_bool()).unwrap_or(false),
                        secret: e.get("isSecret").and_then(|v| v.as_bool()).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse an MCP Registry `server.json`. Reads the first `packages[]` entry (and, for
/// HTTP path, the first `remotes[]` entry).
pub fn parse_server_json(content: &str) -> Option<ManifestHints> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    let mut h = ManifestHints { source: "server.json", ..Default::default() };

    if let Some(pkg) = v.get("packages").and_then(|p| p.as_array()).and_then(|a| a.first()) {
        if let Some(rt) = pkg.get("registryType").and_then(|v| v.as_str()) {
            h.runtime = runtime_from_registry_type(rt);
        }
        if h.runtime.is_none() {
            if let Some(hint) = pkg.get("runtimeHint").and_then(|v| v.as_str()) {
                h.runtime = runtime_from_hint(hint);
            }
        }
        if let Some(t) = pkg.get("transport").and_then(|t| t.get("type")).and_then(|v| v.as_str()) {
            h.transport = transport_from_type(t);
        }
        if let Some(url) = pkg.get("transport").and_then(|t| t.get("url")).and_then(|v| v.as_str()) {
            h.mcp_path = path_from_url(url);
        }
        if let Some(env) = pkg.get("environmentVariables") {
            h.env_vars = env_vars_from_json(env);
        }
    }

    // A remote-only server declares an HTTP endpoint under `remotes[]`.
    if let Some(remote) = v.get("remotes").and_then(|r| r.as_array()).and_then(|a| a.first()) {
        if h.transport.is_none() {
            if let Some(t) = remote.get("type").and_then(|v| v.as_str()) {
                h.transport = transport_from_type(t);
            }
        }
        if h.mcp_path.is_none() {
            if let Some(url) = remote.get("url").and_then(|v| v.as_str()) {
                h.mcp_path = path_from_url(url);
            }
        }
    }

    (h.runtime.is_some() || h.transport.is_some() || !h.env_vars.is_empty()).then_some(h)
}

/// Parse a Smithery `smithery.yaml`.
pub fn parse_smithery_yaml(content: &str) -> Option<ManifestHints> {
    let v: serde_json::Value = serde_yaml::from_str(content).ok()?;
    let mut h = ManifestHints { source: "smithery.yaml", ..Default::default() };

    match v.get("runtime").and_then(|r| r.as_str()) {
        Some("container") => {
            h.is_docker = true;
            h.runtime = Some(Runtime::Docker);
        }
        Some("typescript") => h.runtime = Some(Runtime::Node),
        _ => {}
    }

    if let Some(start) = v.get("startCommand") {
        if let Some(t) = start.get("type").and_then(|v| v.as_str()) {
            h.transport = transport_from_type(t);
        }
    }

    // Smithery declares required config keys under configSchema.properties; treat them
    // as env var keys (Smithery injects them into the process env).
    if let Some(props) = v
        .get("startCommand")
        .and_then(|s| s.get("configSchema"))
        .and_then(|c| c.get("properties"))
        .and_then(|p| p.as_object())
    {
        let required: Vec<&str> = v
            .get("startCommand")
            .and_then(|s| s.get("configSchema"))
            .and_then(|c| c.get("required"))
            .and_then(|r| r.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        for (key, spec) in props {
            let up = to_env_key(key);
            h.env_vars.push(EnvVar {
                description: spec.get("description").and_then(|v| v.as_str()).map(str::to_string),
                required: required.contains(&key.as_str()),
                secret: spec
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|d| {
                        let d = d.to_ascii_lowercase();
                        d.contains("key") || d.contains("token") || d.contains("secret")
                    })
                    .unwrap_or(false)
                    || key.to_ascii_lowercase().contains("key")
                    || key.to_ascii_lowercase().contains("token"),
                key: up,
            });
        }
    }

    (h.runtime.is_some() || h.transport.is_some() || h.is_docker || !h.env_vars.is_empty())
        .then_some(h)
}

/// Smithery config keys are usually camelCase (e.g. `e2bApiKey`); env vars are
/// UPPER_SNAKE. Convert as a best-effort so the key matches conventional env naming.
fn to_env_key(k: &str) -> String {
    let mut out = String::new();
    for (i, ch) in k.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        if ch == '-' || ch == ' ' {
            out.push('_');
        } else {
            out.push(ch.to_ascii_uppercase());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_json_stdio_npm() {
        let s = r#"{"packages":[{"registryType":"npm","identifier":"@x/y",
            "transport":{"type":"stdio"},
            "environmentVariables":[{"name":"API_KEY","isRequired":true,"isSecret":true}]}]}"#;
        let h = parse_server_json(s).unwrap();
        assert_eq!(h.runtime, Some(Runtime::Node));
        assert_eq!(h.transport, Some(Transport::Stdio));
        assert_eq!(h.env_vars.len(), 1);
        assert!(h.env_vars[0].required && h.env_vars[0].secret);
    }

    #[test]
    fn server_json_http_path() {
        let s = r#"{"packages":[{"registryType":"pypi",
            "transport":{"type":"streamable-http","url":"https://ex.com/mcp?x=1"}}]}"#;
        let h = parse_server_json(s).unwrap();
        assert_eq!(h.runtime, Some(Runtime::Python));
        assert_eq!(h.transport, Some(Transport::Sse));
        assert_eq!(h.mcp_path.as_deref(), Some("/mcp"));
    }

    #[test]
    fn smithery_container() {
        let y = "runtime: container\nbuild:\n  dockerfile: Dockerfile\n";
        let h = parse_smithery_yaml(y).unwrap();
        assert!(h.is_docker);
        assert_eq!(h.runtime, Some(Runtime::Docker));
    }

    #[test]
    fn smithery_stdio() {
        let y = "startCommand:\n  type: stdio\n";
        let h = parse_smithery_yaml(y).unwrap();
        assert_eq!(h.transport, Some(Transport::Stdio));
    }
}

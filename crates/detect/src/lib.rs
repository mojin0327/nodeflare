//! Single source of truth for auto-detecting an MCP server's deploy configuration from
//! a repository. The detection *rules* are written once over a [`RepoFiles`] abstraction
//! so the exact same algorithm runs against a locally-cloned repo (the builder) and
//! against the GitHub Contents API (the server-create form's `/inspect` endpoint) —
//! guaranteeing the form shows what will actually build.
//!
//! Signal priority per field (authoritative → heuristic): declared manifest
//! (`server.json` / `smithery.yaml`) → manifest files (package.json/pyproject/…) →
//! source scan. Manual form input is the last resort. See `manifest`/`scan`/`parse`.

pub mod manifest;
pub mod parse;
pub mod scan;

use async_trait::async_trait;
use parse::NodePm;
use serde::Serialize;

/// Runtime NodeFlare can build. Serializes to the form's lowercase spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Node,
    Python,
    Go,
    Rust,
    Docker,
}

impl Runtime {
    pub fn as_str(self) -> &'static str {
        match self {
            Runtime::Node => "node",
            Runtime::Python => "python",
            Runtime::Go => "go",
            Runtime::Rust => "rust",
            Runtime::Docker => "docker",
        }
    }
}

/// Transport, in the form's vocabulary: `Sse` is the HTTP-exposed side (Streamable HTTP
/// or legacy SSE), `Stdio` is the stdin/stdout adapter side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
    Sse,
}

/// A required/optional environment variable, with whatever metadata the source offered.
#[derive(Debug, Clone, Serialize)]
pub struct EnvVar {
    pub key: String,
    pub description: Option<String>,
    pub required: bool,
    pub secret: bool,
}

/// One provenance record: which field a value came from, and the signal that produced it.
#[derive(Debug, Clone, Serialize)]
pub struct Signal {
    pub field: &'static str,
    pub source: String,
}

/// The fully-resolved, form-ready detection result.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Detection {
    pub runtime: Option<Runtime>,
    pub transport: Option<Transport>,
    /// "" means the repo root; otherwise the subdirectory to build in.
    pub root_directory: Option<String>,
    pub build_command: Option<String>,
    pub entry_command: Option<String>,
    pub mcp_path: Option<String>,
    pub port: Option<u16>,
    pub env_vars: Vec<EnvVar>,
    /// True when the target is a monorepo workspace member built from the repo root.
    pub is_monorepo_member: bool,
    pub signals: Vec<Signal>,
    pub warnings: Vec<String>,
}

impl Detection {
    fn signal(&mut self, field: &'static str, source: impl Into<String>) {
        self.signals.push(Signal { field, source: source.into() });
    }
}

/// Filesystem access for the detector — implemented over local FS (builder) or the
/// GitHub Contents API (inspect endpoint). All paths are repo-relative with `/`.
#[async_trait]
pub trait RepoFiles: Send + Sync {
    /// File contents, or None if absent/unreadable/binary.
    async fn read(&self, path: &str) -> Option<String>;
    /// Whether a path exists (file or directory).
    async fn exists(&self, path: &str) -> bool;
    /// Immediate child directory names of `path` (best-effort; may be empty).
    async fn list_dir(&self, path: &str) -> Vec<String>;
}

/// Join a repo-relative base dir (possibly empty = root) with a relative path.
fn join(base: &str, rel: &str) -> String {
    let base = base.trim_matches('/');
    if base.is_empty() {
        rel.to_string()
    } else {
        format!("{}/{}", base, rel)
    }
}

async fn detect_node_pm<F: RepoFiles + ?Sized>(files: &F, dir: &str) -> NodePm {
    if files.exists(&join(dir, "pnpm-lock.yaml")).await
        || files.exists(&join(dir, "pnpm-workspace.yaml")).await
        || files.exists(&join(dir, "pnpm-workspace.yml")).await
    {
        return NodePm::Pnpm;
    }
    if files.exists(&join(dir, "bun.lockb")).await || files.exists(&join(dir, "bun.lock")).await {
        return NodePm::Bun;
    }
    if files.exists(&join(dir, "yarn.lock")).await {
        return NodePm::Yarn;
    }
    if let Some(content) = files.read(&join(dir, "package.json")).await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(pm) = v.get("packageManager").and_then(|p| p.as_str()) {
                let pm = pm.trim_start();
                if pm.starts_with("pnpm") {
                    return NodePm::Pnpm;
                }
                if pm.starts_with("yarn") {
                    return NodePm::Yarn;
                }
                if pm.starts_with("bun") {
                    return NodePm::Bun;
                }
            }
        }
    }
    NodePm::Npm
}

/// Concatenate a bounded set of candidate entry-source files for heuristic scanning.
async fn read_sources<F: RepoFiles + ?Sized>(files: &F, base: &str, runtime: Runtime) -> String {
    let candidates: &[&str] = match runtime {
        Runtime::Node => &[
            "src/index.ts", "src/server.ts", "src/main.ts", "index.ts", "server.ts",
            "src/index.js", "index.js", "server.js", "build/index.js", "dist/index.js",
        ],
        Runtime::Python => &[
            "server.py", "main.py", "app.py", "__main__.py", "src/server.py", "src/main.py",
        ],
        _ => &[],
    };
    let mut out = String::new();
    for c in candidates {
        if out.len() > 200_000 {
            break;
        }
        if let Some(text) = files.read(&join(base, c)).await {
            out.push_str(&text);
            out.push('\n');
        }
    }
    out
}

/// Detect deploy configuration for `subdir` (or the repo root when None).
pub async fn inspect<F: RepoFiles + ?Sized>(files: &F, subdir: Option<&str>) -> Detection {
    let member = subdir.unwrap_or("").trim_matches('/').to_string();
    let base = member.clone(); // directory the server code lives in
    let mut d = Detection::default();

    // --- 1. Manifests (strongest signal) — check the target dir, then the repo root. ---
    let mut hints = manifest::ManifestHints::default();
    let parsers: [(&str, fn(&str) -> Option<manifest::ManifestHints>); 4] = [
        ("server.json", manifest::parse_server_json),
        ("mcp.json", manifest::parse_server_json),
        ("smithery.yaml", manifest::parse_smithery_yaml),
        ("smithery.yml", manifest::parse_smithery_yaml),
    ];
    for dir in dedup_dirs(&[base.as_str(), ""]) {
        for (name, parse) in parsers {
            if let Some(text) = files.read(&join(&dir, name)).await {
                if let Some(h) = parse(&text) {
                    merge_hints(&mut hints, h);
                }
            }
        }
    }

    // --- 2. Runtime ---
    let mut runtime = hints.runtime;
    if runtime.is_some() {
        d.signal("runtime", format!("manifest ({})", hints.source));
    } else if let Some((r, why)) = detect_runtime(files, &base).await {
        d.signal("runtime", why);
        runtime = Some(r);
    }
    let Some(runtime) = runtime else {
        d.warnings.push("Could not determine runtime from the repository.".into());
        return d;
    };
    d.runtime = Some(runtime);

    // --- Docker short-circuit: the Dockerfile owns build + start. ---
    if runtime == Runtime::Docker {
        d.root_directory = Some(base.clone());
        d.transport = hints.transport;
        d.env_vars = merge_env(hints.env_vars, vec![], vec![]);
        d.warnings
            .push("Docker runtime: build and start come from the repository's Dockerfile.".into());
        return d;
    }

    // --- 3. Monorepo membership → decide build context (方法Y). ---
    let root_pkg = files.read("package.json").await.unwrap_or_default();
    let mut globs = parse::parse_workspaces(&root_pkg);
    if let Some(y) = files
        .read("pnpm-workspace.yaml")
        .await
        .or(files.read("pnpm-workspace.yml").await)
    {
        globs.extend(parse::parse_pnpm_workspace(&y));
    }
    // The build-from-repo-root treatment exists because of Node's workspace hoisting and
    // `tsconfig extends` — both Node-specific. Python/Go/Rust members build standalone in
    // their own directory (each has its own pyproject/go.mod/Cargo.toml + lockfile), so an
    // `src/*` glob that also matches them must NOT drag them to the repo root.
    let is_member = runtime == Runtime::Node
        && !member.is_empty()
        && (parse::subdir_is_workspace_member(&member, &globs) || {
            match files.read(&join(&base, "tsconfig.json")).await {
                Some(ts) => parse::tsconfig_extends_parent(&ts),
                None => false,
            }
        });
    d.is_monorepo_member = is_member;

    // The package manager signal (lockfile / pnpm-workspace) lives at the ROOT for a
    // workspace member, and in the target dir otherwise.
    let pm_dir = if is_member { "" } else { base.as_str() };
    let pm = detect_node_pm(files, pm_dir).await;

    // --- 4/5. entry + build + root_directory ---
    resolve_commands(&mut d, files, runtime, &base, &member, is_member, pm).await;

    // Read candidate source files ONCE — each read is a network call on the API path.
    let sources = read_sources(files, &base, runtime).await;

    // --- 6. Transport (manifest → source scan → stdio default) ---
    let transport = if let Some(t) = hints.transport {
        d.signal("transport", format!("manifest ({})", hints.source));
        t
    } else {
        match scan::scan_transport(&sources) {
            Some(t) => {
                d.signal("transport", "source scan");
                t
            }
            None => {
                d.signal("transport", "default (MCP stdio)");
                Transport::Stdio
            }
        }
    };
    d.transport = Some(transport);

    // --- 7. HTTP-only fields ---
    if transport == Transport::Sse {
        d.mcp_path = Some(hints.mcp_path.clone().unwrap_or_else(|| "/mcp".to_string()));
        d.port = Some(hints.port.or_else(|| scan::scan_port(&sources)).unwrap_or(8080));
    }

    // --- 8. Env vars: manifest ∪ .env.example ∪ source scan ---
    let example = files
        .read(&join(&base, ".env.example"))
        .await
        .or(files.read(".env.example").await)
        .map(|c| scan::env_keys_from_example(&c))
        .unwrap_or_default();
    let scanned = scan::scan_env_keys(&sources);
    d.env_vars = merge_env(hints.env_vars, example, scanned);
    if !d.env_vars.is_empty() {
        d.signal("env_vars", "manifest / .env.example / source scan");
    }

    d
}

/// Runtime from manifest files present in `base`. Returns the runtime and a provenance.
async fn detect_runtime<F: RepoFiles + ?Sized>(
    files: &F,
    base: &str,
) -> Option<(Runtime, String)> {
    if files.exists(&join(base, "package.json")).await {
        return Some((Runtime::Node, "package.json".into()));
    }
    if files.exists(&join(base, "pyproject.toml")).await
        || files.exists(&join(base, "requirements.txt")).await
        || files.exists(&join(base, "setup.py")).await
    {
        return Some((Runtime::Python, "python manifest".into()));
    }
    if files.exists(&join(base, "go.mod")).await {
        return Some((Runtime::Go, "go.mod".into()));
    }
    if files.exists(&join(base, "Cargo.toml")).await {
        return Some((Runtime::Rust, "Cargo.toml".into()));
    }
    if files.exists(&join(base, "Dockerfile")).await {
        return Some((Runtime::Docker, "Dockerfile".into()));
    }
    None
}

/// Fill entry_command, build_command and root_directory for a non-Docker runtime.
async fn resolve_commands<F: RepoFiles + ?Sized>(
    d: &mut Detection,
    files: &F,
    runtime: Runtime,
    base: &str,
    member: &str,
    is_member: bool,
    pm: NodePm,
) {
    // root_directory: workspace member builds from the repo root; anything else builds
    // in its own directory.
    d.root_directory = Some(if is_member { String::new() } else { base.to_string() });

    match runtime {
        Runtime::Node => {
            let pkg = files.read(&join(base, "package.json")).await.unwrap_or_default();
            let local_entry = parse::parse_node_entry(&pkg, pm);
            let has_build = parse::node_has_build_script(&pkg);

            if is_member {
                if let Some(e) = &local_entry {
                    d.entry_command = Some(parse::prefix_entry_with_subdir(e, member));
                    d.signal("entry_command", "package.json (workspace member)");
                }
                if has_build {
                    d.build_command = Some(match pm {
                        NodePm::Pnpm => {
                            format!("pnpm install && pnpm --filter ./{} run build", member)
                        }
                        _ => format!("{} install && {} run build -w {}", pm.cli(), pm.cli(), member),
                    });
                    d.signal("build_command", "workspace-scoped build");
                }
            } else {
                if let Some(e) = local_entry {
                    d.entry_command = Some(e);
                    d.signal("entry_command", "package.json");
                }
                if has_build {
                    d.build_command = Some(format!("{} build", pm.runner()));
                    d.signal("build_command", "package.json build script");
                }
            }
            if d.entry_command.is_none() {
                d.warnings
                    .push("Could not infer the Node start command — please set it.".into());
            }
        }
        Runtime::Python => {
            let py = files.read(&join(base, "pyproject.toml")).await;
            if let Some(script) = py.as_deref().and_then(parse::parse_pyproject_script) {
                d.entry_command = Some(script);
                d.signal("entry_command", "pyproject [project.scripts]");
            } else {
                for cand in ["server.py", "main.py", "app.py", "__main__.py"] {
                    if files.exists(&join(base, cand)).await {
                        d.entry_command = Some(format!("python {}", cand));
                        d.signal("entry_command", "entry file");
                        break;
                    }
                }
            }
            d.build_command = if files.exists(&join(base, "uv.lock")).await {
                Some("uv sync".into())
            } else if files.exists(&join(base, "requirements.txt")).await {
                Some("pip install -r requirements.txt".into())
            } else if py.is_some() {
                Some("pip install .".into())
            } else {
                None
            };
            if d.build_command.is_some() {
                d.signal("build_command", "python dependency install");
            }
        }
        Runtime::Go => {
            d.entry_command = Some("./server".into());
            d.build_command = Some("go build -o server .".into());
            d.signal("entry_command", "go binary");
            d.signal("build_command", "go build");
        }
        Runtime::Rust => {
            let cargo = files.read(&join(base, "Cargo.toml")).await.unwrap_or_default();
            let bin = parse::parse_cargo_bin(&cargo).unwrap_or_else(|| "server".into());
            d.entry_command = Some(format!("./{}", bin));
            d.build_command = Some("cargo build --release".into());
            d.signal("entry_command", "Cargo [[bin]]");
            d.signal("build_command", "cargo build");
        }
        Runtime::Docker => {}
    }
}

/// De-duplicate directory candidates while preserving order.
fn dedup_dirs(dirs: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for d in dirs {
        let d = d.trim_matches('/').to_string();
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// Merge manifest hints, preferring already-set (target-dir) values over root values.
fn merge_hints(acc: &mut manifest::ManifestHints, next: manifest::ManifestHints) {
    if acc.runtime.is_none() {
        acc.runtime = next.runtime;
    }
    if acc.transport.is_none() {
        acc.transport = next.transport;
    }
    if acc.mcp_path.is_none() {
        acc.mcp_path = next.mcp_path;
    }
    if acc.port.is_none() {
        acc.port = next.port;
    }
    acc.is_docker = acc.is_docker || next.is_docker;
    if acc.env_vars.is_empty() {
        acc.env_vars = next.env_vars;
    }
    if acc.source.is_empty() {
        acc.source = next.source;
    }
}

/// Union env vars from the three signal tiers by key, richest metadata winning.
fn merge_env(manifest: Vec<EnvVar>, example: Vec<String>, scanned: Vec<String>) -> Vec<EnvVar> {
    let mut out = manifest;
    for key in example.into_iter().chain(scanned) {
        if !out.iter().any(|e| e.key == key) {
            let secret = {
                let k = key.to_ascii_lowercase();
                k.contains("key") || k.contains("token") || k.contains("secret") || k.contains("password")
            };
            out.push(EnvVar { key, description: None, required: false, secret });
        }
    }
    out
}

/// A synchronous local-filesystem [`RepoFiles`] over a cloned repo root.
pub struct LocalFs {
    root: std::path::PathBuf,
}

impl LocalFs {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[async_trait]
impl RepoFiles for LocalFs {
    async fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(path)).ok()
    }
    async fn exists(&self, path: &str) -> bool {
        self.root.join(path).exists()
    }
    async fn list_dir(&self, path: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.root.join(path)) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    if let Some(n) = e.file_name().to_str() {
                        out.push(n.to_string());
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemFs {
        files: HashMap<String, String>,
    }
    impl MemFs {
        fn with(mut self, path: &str, content: &str) -> Self {
            self.files.insert(path.to_string(), content.to_string());
            self
        }
    }
    #[async_trait]
    impl RepoFiles for MemFs {
        async fn read(&self, path: &str) -> Option<String> {
            self.files.get(path).cloned()
        }
        async fn exists(&self, path: &str) -> bool {
            self.files.keys().any(|k| k == path || k.starts_with(&format!("{}/", path)))
        }
        async fn list_dir(&self, _path: &str) -> Vec<String> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn node_workspace_member_uses_root_build_and_full_entry() {
        let fs = MemFs::default()
            .with("package.json", r#"{"workspaces":["src/*"]}"#)
            .with(
                "src/seq/package.json",
                r#"{"name":"seq","bin":{"seq":"dist/index.js"},"scripts":{"build":"tsc"}}"#,
            )
            .with("src/seq/tsconfig.json", r#"{"extends":"../../tsconfig.json"}"#)
            .with("src/seq/src/index.ts", "new StdioServerTransport()");
        let d = inspect(&fs, Some("src/seq")).await;
        assert_eq!(d.runtime, Some(Runtime::Node));
        assert!(d.is_monorepo_member);
        assert_eq!(d.root_directory.as_deref(), Some(""));
        assert_eq!(d.entry_command.as_deref(), Some("node src/seq/dist/index.js"));
        assert_eq!(d.build_command.as_deref(), Some("npm install && npm run build -w src/seq"));
        assert_eq!(d.transport, Some(Transport::Stdio));
    }

    #[tokio::test]
    async fn standalone_http_node_gets_port_and_path() {
        let fs = MemFs::default()
            .with("package.json", r#"{"main":"dist/index.js","scripts":{"build":"tsc"}}"#)
            .with("dist/index.js", "const app = express(); app.listen(3000)");
        let d = inspect(&fs, None).await;
        assert!(!d.is_monorepo_member);
        assert_eq!(d.root_directory.as_deref(), Some(""));
        assert_eq!(d.transport, Some(Transport::Sse));
        assert_eq!(d.mcp_path.as_deref(), Some("/mcp"));
        assert_eq!(d.port, Some(3000));
        assert_eq!(d.build_command.as_deref(), Some("npm run build"));
    }

    #[tokio::test]
    async fn server_json_manifest_wins() {
        let fs = MemFs::default()
            .with("package.json", r#"{"main":"index.js"}"#)
            .with(
                "server.json",
                r#"{"packages":[{"registryType":"npm","transport":{"type":"streamable-http","url":"https://x/mcp"},
                    "environmentVariables":[{"name":"TOKEN","isRequired":true,"isSecret":true}]}]}"#,
            );
        let d = inspect(&fs, None).await;
        assert_eq!(d.transport, Some(Transport::Sse));
        assert_eq!(d.mcp_path.as_deref(), Some("/mcp"));
        assert!(d.env_vars.iter().any(|e| e.key == "TOKEN" && e.required));
    }
}

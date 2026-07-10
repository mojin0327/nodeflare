use anyhow::{Context, Result};
use mcp_common::AppConfig;
use mcp_queue::{BuildJob, SecretEnv};
use serde::Deserialize;
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Minimum secret-value length we will substring-redact. Replacing a 1-3 char value
/// (e.g. a port number used as a "secret") would corrupt unrelated log text, so we
/// only redact values long enough to be unambiguous; shorter values still get caught
/// by the token-shape patterns below when they look like credentials.
const MIN_REDACTED_SECRET_LEN: usize = 4;

/// Regexes matching well-known credential shapes, so we redact tokens that leak via a
/// path we don't hold the value for (e.g. `flyctl` echoing its own auth token, a
/// `Bearer` header in a traced request). Compiled once.
fn token_redaction_patterns() -> &'static [regex::Regex] {
    use std::sync::OnceLock;
    static PATTERNS: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            // Fly.io tokens.
            r"FlyV1\s+[A-Za-z0-9_\-,/.=]+",
            r"f[om][0-9]_[A-Za-z0-9_\-]{10,}",
            // GitHub tokens (ghp_, gho_, ghs_, ghr_, ghu_, github_pat_).
            r"gh[posru]_[A-Za-z0-9]{20,}",
            r"github_pat_[A-Za-z0-9_]{20,}",
            // OpenAI/Anthropic-style keys.
            r"sk-[A-Za-z0-9_\-]{16,}",
            // AWS access key id.
            r"AKIA[0-9A-Z]{12,}",
            // Authorization: Bearer <token>.
            r"(?i)bearer\s+[A-Za-z0-9._\-]{8,}",
        ]
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect()
    })
}

/// Sanitize log output to prevent leaking secret values.
/// Replaces known secret values, and any credential-shaped token, with "[REDACTED]".
fn sanitize_log_output(output: &str, secrets: &[SecretEnv]) -> String {
    let mut sanitized = output.to_string();
    for secret in secrets {
        if secret.value.len() >= MIN_REDACTED_SECRET_LEN {
            sanitized = sanitized.replace(&secret.value, "[REDACTED]");
        }
    }
    for re in token_redaction_patterns() {
        sanitized = re.replace_all(&sanitized, "[REDACTED]").into_owned();
    }
    sanitized
}

/// Validate secret key to prevent injection attacks
/// Keys must be valid environment variable names
fn validate_secret_key(key: &str) -> Result<()> {
    if key.is_empty() {
        anyhow::bail!("Secret key cannot be empty");
    }

    // Must start with letter or underscore
    let first_char = key.chars().next().unwrap();
    if !first_char.is_ascii_alphabetic() && first_char != '_' {
        anyhow::bail!("Secret key must start with a letter or underscore");
    }

    // Must contain only alphanumeric characters and underscores
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        anyhow::bail!("Secret key must contain only letters, numbers, and underscores");
    }

    // Reasonable length limit
    if key.len() > 256 {
        anyhow::bail!("Secret key exceeds maximum length of 256 characters");
    }

    Ok(())
}

/// Validate secret value to prevent injection attacks
/// Returns error if value contains dangerous characters
fn validate_secret_value(value: &str) -> Result<()> {
    // Block newlines which could inject additional KEY=VALUE pairs
    if value.contains('\n') || value.contains('\r') {
        anyhow::bail!("Secret value cannot contain newline characters");
    }

    // Block null bytes which can truncate strings
    if value.contains('\0') {
        anyhow::bail!("Secret value cannot contain null bytes");
    }

    // Value length limit (flyctl has limits, and very long values could cause issues)
    if value.len() > 65536 {
        anyhow::bail!("Secret value exceeds maximum length of 65536 bytes");
    }

    Ok(())
}

const FLY_API_URL: &str = "https://api.machines.dev/v1";

// ============================================================================
// Centralized constants (single source of truth)
//
// Base image versions, internal ports, the probed MCP protocol version and the
// external-command timeouts used to live as magic literals scattered across the
// Dockerfile templates and the deploy path. Centralizing them here keeps a bump
// (e.g. node 20 -> 22) to one edit and makes the timeouts auditable. Values are
// unchanged from the originals.
// ============================================================================

/// Base images used by the generated Dockerfiles.
const NODE_IMAGE: &str = "node:20-slim";
const PYTHON_IMAGE: &str = "python:3.11-slim";
const GO_BUILDER_IMAGE: &str = "golang:1.22";
const RUST_BUILDER_IMAGE: &str = "rust:1.75-slim";
const DEBIAN_RUNTIME_IMAGE: &str = "debian:bookworm-slim";

/// Internal ports per transport/runtime.
const STDIO_PORT: u16 = 8000;
const NODE_PORT: u16 = 3000;
const PYTHON_PORT: u16 = 8000;
const GO_RUST_PORT: u16 = 8080;

/// MCP protocol version sent by the post-deploy verification probe.
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Post-deploy probe budget.
const PROBE_ATTEMPTS: usize = 8;
const PROBE_RETRY_DELAY_SECS: u64 = 5;
const PROBE_REQUEST_TIMEOUT_SECS: u64 = 12;

/// Timeouts for external `flyctl` invocations. `apps create` / `secrets` are quick
/// control-plane calls; `deploy` runs a full remote image build.
const FLYCTL_CREATE_TIMEOUT_SECS: u64 = 60;
const FLYCTL_SECRETS_TIMEOUT_SECS: u64 = 60;
const FLYCTL_DEPLOY_TIMEOUT_SECS: u64 = 20 * 60;

/// STDIO-to-SSE adapter script content (embedded at compile time)
const STDIO_ADAPTER_JS: &str = include_str!("../assets/stdio-adapter.cjs");

/// Extract the entry command (ENTRYPOINT or CMD) from an existing Dockerfile
fn extract_dockerfile_entry_command(dockerfile_content: &str) -> Option<String> {
    let mut entrypoint: Option<String> = None;
    let mut cmd: Option<String> = None;

    for line in dockerfile_content.lines() {
        let line = line.trim();

        // Handle ENTRYPOINT
        if line.to_uppercase().starts_with("ENTRYPOINT") {
            let rest = line[10..].trim();
            if let Some(command) = parse_docker_command(rest) {
                entrypoint = Some(command);
            }
        }

        // Handle CMD
        if line.to_uppercase().starts_with("CMD") {
            let rest = line[3..].trim();
            if let Some(command) = parse_docker_command(rest) {
                cmd = Some(command);
            }
        }
    }

    // ENTRYPOINT takes precedence, then CMD
    entrypoint.or(cmd)
}

/// Wrap a token in double quotes if it contains whitespace (or is empty), so that a
/// multi-word argument decoded from a JSON exec-form survives the round-trip through
/// the `String`-typed entry command and is re-tokenized as a single argument by
/// `tokenize_shell`. Tokens without whitespace pass through unchanged.
fn shell_quote_if_needed(token: &str) -> String {
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        format!("\"{}\"", token.replace('"', "\\\""))
    } else {
        token.to_string()
    }
}

/// Parse a Docker command (JSON exec form or shell form) into a command string.
/// The JSON exec form is decoded with `serde_json` (correctly handling escaped quotes,
/// embedded commas and multi-word arguments) rather than a naive comma split; multi-word
/// elements are re-quoted so they round-trip as single arguments.
fn parse_docker_command(s: &str) -> Option<String> {
    let s = s.trim();

    // JSON array (exec) form: ["executable", "arg with spaces", "arg2"]
    if s.starts_with('[') {
        if let Ok(parts) = serde_json::from_str::<Vec<String>>(s) {
            let parts: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
            if !parts.is_empty() {
                return Some(
                    parts
                        .iter()
                        .map(|p| shell_quote_if_needed(p))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
        }
        return None;
    }

    // Shell form: executable arg1 arg2
    if !s.is_empty() {
        return Some(s.to_string());
    }

    None
}

// ============================================================================
// Dockerfile context-fit check (judgment B)
//
// A repo's own Dockerfile is only usable when built from a given context if all
// of its COPY/ADD sources resolve inside that context. A Dockerfile written for
// a monorepo root (e.g. `COPY src/filesystem /app`, `COPY tsconfig.json …`) cannot
// build from the `src/filesystem` subdirectory — those paths aren't there. Adopting
// it anyway breaks the build and pins a stale entry command. We detect that and
// fall back to generating our own Dockerfile instead.
//
// Direction of the check: we only discard the repo Dockerfile on a *provable*
// escape (a literal path that isn't present, or a `..` traversal). Anything we
// can't resolve statically (unresolved ${VARS}, remote URLs, globs) is left to the
// author — a false "usable" is caught loudly by the build / the post-deploy probe,
// whereas a false "unusable" would silently replace a working build.
// ============================================================================

/// A build-context-relative source path referenced by a COPY/ADD instruction.
#[derive(Debug, Clone, PartialEq)]
struct ContextSource {
    /// The source path as written (may contain globs), e.g. "src/filesystem".
    raw: String,
    /// The full logical instruction, for diagnostics, e.g. "COPY src/filesystem /app".
    instruction: String,
}

#[derive(Debug, PartialEq)]
enum EscapeReason {
    /// The path does not exist inside the build context.
    NotFound,
    /// The path reaches outside the context via `..`.
    ParentTraversal,
}

struct EscapingSource {
    instruction: String,
    source: String,
    #[allow(dead_code)]
    reason: EscapeReason,
}

enum DockerfileContextFit {
    Usable,
    Unusable { escaping: Vec<EscapingSource> },
}

/// Join physical Dockerfile lines into logical instructions, honoring `\` line
/// continuations and dropping full-line comments / blank lines. A `#` only starts
/// a comment when it is not in the middle of a continued instruction.
fn logical_instructions(dockerfile: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for raw_line in dockerfile.lines() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        if current.is_empty() && (trimmed.is_empty() || trimmed.starts_with('#')) {
            continue;
        }
        let continues = line.trim_end().ends_with('\\');
        let segment = if continues {
            let s = line.trim_end();
            s[..s.len() - 1].trim()
        } else {
            trimmed
        };
        if current.is_empty() {
            current.push_str(segment);
        } else if !segment.is_empty() {
            current.push(' ');
            current.push_str(segment);
        }
        if !continues {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Split the text after a COPY/ADD keyword into (leading flags, operands).
/// Handles shell form (`--flag a b dest`) and JSON exec form (`--flag ["a","dest"]`).
fn split_copy_tokens(rest: &str) -> (Vec<String>, Vec<String>) {
    let mut flags = Vec::new();
    let mut remainder = rest.trim();
    while let Some(stripped) = remainder.strip_prefix("--") {
        let end = stripped.find(char::is_whitespace).unwrap_or(stripped.len());
        flags.push(format!("--{}", &stripped[..end]));
        remainder = stripped[end..].trim_start();
    }
    let operands = if remainder.starts_with('[') {
        parse_json_string_array(remainder)
    } else {
        tokenize_shell(remainder)
    };
    (flags, operands)
}

/// Extract string elements from a JSON array literal: `["a", "b"]` -> [a, b].
fn parse_json_string_array(s: &str) -> Vec<String> {
    let s = s.trim();
    if !(s.starts_with('[') && s.ends_with(']')) {
        return Vec::new();
    }
    s[1..s.len() - 1]
        .split(',')
        .map(|p| p.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Quote-aware whitespace tokenizer for the shell form of COPY/ADD.
fn tokenize_shell(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut in_token = false;
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                    in_token = true;
                } else if c.is_whitespace() {
                    if in_token {
                        tokens.push(std::mem::take(&mut cur));
                        in_token = false;
                    }
                } else {
                    cur.push(c);
                    in_token = true;
                }
            }
        }
    }
    if in_token {
        tokens.push(cur);
    }
    tokens
}

fn has_glob(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

/// True for ADD sources that come from the network / a git ref, not the context.
fn is_remote_source(src: &str) -> bool {
    let s = src.to_lowercase();
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("ftp://")
        || s.starts_with("git@")
        || s.starts_with("git://")
        || (s.contains("github.com/") && s.contains('#'))
}

/// True if the path contains an unresolved build variable (`$VAR`/`${VAR}`),
/// treating `$$` as an escaped literal dollar.
fn contains_unresolved_var(src: &str) -> bool {
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            if bytes.get(i + 1) == Some(&b'$') {
                i += 2;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

/// Collect every build-context-relative COPY/ADD source in a Dockerfile.
/// Excludes `--from=<stage>` copies, remote ADD URLs, heredocs and `$VAR` paths.
fn parse_context_sources(dockerfile: &str) -> Vec<ContextSource> {
    let mut sources = Vec::new();
    for instr in logical_instructions(dockerfile) {
        let mut it = instr.splitn(2, char::is_whitespace);
        let keyword = it.next().unwrap_or("").to_uppercase();
        if keyword != "COPY" && keyword != "ADD" {
            continue;
        }
        let rest = it.next().unwrap_or("").trim();
        let (flags, operands) = split_copy_tokens(rest);
        // `--from=` -> source is a build stage / external image, not the context.
        if flags.iter().any(|f| f == "--from" || f.starts_with("--from=")) {
            continue;
        }
        // Need at least one source plus a destination to interpret reliably.
        if operands.len() < 2 {
            continue;
        }
        let is_add = keyword == "ADD";
        for src in &operands[..operands.len() - 1] {
            if src.starts_with("<<") {
                continue; // heredoc inline content
            }
            if is_add && is_remote_source(src) {
                continue;
            }
            if contains_unresolved_var(src) {
                continue;
            }
            sources.push(ContextSource {
                raw: src.clone(),
                instruction: instr.clone(),
            });
        }
    }
    sources
}

/// Longest leading path with no glob metacharacter (drops the globbed segment on).
fn glob_static_prefix(norm: &str) -> String {
    let mut parts = Vec::new();
    for seg in norm.split('/') {
        if has_glob(seg) {
            break;
        }
        parts.push(seg);
    }
    parts.join("/")
}

/// Decide whether a single context source escapes `context_dir`.
fn source_escapes(
    raw: &str,
    context_dir: &Path,
    canonical_ctx: Option<&Path>,
) -> Option<EscapeReason> {
    let norm = raw.trim_start_matches('/').replace('\\', "/");
    if norm.is_empty() || norm == "." || norm == "./" {
        return None; // whole context
    }
    if norm.split('/').any(|seg| seg == "..") {
        return Some(EscapeReason::ParentTraversal);
    }
    let check_path = if has_glob(&norm) {
        glob_static_prefix(&norm)
    } else {
        norm.clone()
    };
    if check_path.is_empty() {
        return None; // glob whose parent is the context root
    }
    let full = context_dir.join(&check_path);
    if !full.exists() {
        return Some(EscapeReason::NotFound);
    }
    // Symlink safety: a link pointing outside the context is not "inside" it.
    if let (Ok(canon), Some(ctx)) = (full.canonicalize(), canonical_ctx) {
        if !canon.starts_with(ctx) {
            return Some(EscapeReason::NotFound);
        }
    }
    None
}

/// Judgment B: is the repo's Dockerfile buildable from `context_dir`?
fn dockerfile_context_fit(dockerfile: &str, context_dir: &Path) -> DockerfileContextFit {
    let canonical_ctx = context_dir.canonicalize().ok();
    let mut escaping = Vec::new();
    for cs in parse_context_sources(dockerfile) {
        if let Some(reason) = source_escapes(&cs.raw, context_dir, canonical_ctx.as_deref()) {
            escaping.push(EscapingSource {
                instruction: cs.instruction,
                source: cs.raw,
                reason,
            });
        }
    }
    if escaping.is_empty() {
        DockerfileContextFit::Usable
    } else {
        DockerfileContextFit::Unusable { escaping }
    }
}

/// Workspace member globs declared in a package.json (npm/yarn workspaces).
/// Handles both the array form and the `{ "packages": [...] }` object form.
/// Empty when the field is absent.
pub(crate) fn parse_workspaces(package_json: &str) -> Vec<String> {
    mcp_detect::parse::parse_workspaces(package_json)
}

/// Extract package globs from a pnpm-workspace.yaml `packages:` list. Minimal and
/// line-based (avoids pulling in a YAML dependency): collects `- <glob>` items under
/// the `packages:` key (block form) or `packages: [...]` (inline flow form), strips
/// quotes, and skips negation (`!`) exclusion patterns.
pub(crate) fn parse_pnpm_workspace(yaml: &str) -> Vec<String> {
    mcp_detect::parse::parse_pnpm_workspace(yaml)
}

/// Detect workspace member globs for a repo directory across npm/yarn
/// (package.json `workspaces`) and pnpm (pnpm-workspace.yaml). Empty when the repo
/// is not a workspaces monorepo.
pub(crate) fn detect_workspace_globs(dir: &Path) -> Vec<String> {
    let mut globs = Vec::new();
    if let Ok(pkg) = std::fs::read_to_string(dir.join("package.json")) {
        globs.extend(parse_workspaces(&pkg));
    }
    if let Ok(y) = std::fs::read_to_string(dir.join("pnpm-workspace.yaml"))
        .or_else(|_| std::fs::read_to_string(dir.join("pnpm-workspace.yml")))
    {
        globs.extend(parse_pnpm_workspace(&y));
    }
    globs.sort();
    globs.dedup();
    globs
}

/// Expand `prefix/*` and `prefix/**` workspace globs into the actual subdirectories
/// present, for guiding the user toward a concrete target. Literal members pass through.
fn list_workspace_members(root: &Path, globs: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for g in globs {
        if let Some(prefix) = g.strip_suffix("/**").or_else(|| g.strip_suffix("/*")) {
            if let Ok(rd) = std::fs::read_dir(root.join(prefix)) {
                for entry in rd.flatten() {
                    if entry.path().is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            out.push(format!("{}/{}", prefix, name));
                        }
                    }
                }
            }
        } else if !has_glob(g) {
            out.push(g.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether `subdir` (repo-relative, e.g. "src/filesystem") is matched by one of the
/// workspace globs. Supports `prefix/**` (any depth), `prefix/*` (direct child),
/// `*` and literal forms.
pub(crate) fn subdir_is_workspace_member(subdir: &str, globs: &[String]) -> bool {
    mcp_detect::parse::subdir_is_workspace_member(subdir, globs)
}

/// Startup entry for a workspace member, resolved with the workspace ROOT's package
/// manager. For node, the pnpm/npm signal (lockfile, pnpm-workspace.yaml,
/// packageManager) lives at the root, not in the member, so detecting it from the
/// member would mislabel a pnpm member as npm. Non-node runtimes are runner-agnostic
/// and fall back to the member's own detection.
pub(crate) async fn detect_member_entry(root: &Path, member: &Path, runtime: &str) -> Option<String> {
    if runtime == "node" {
        let pm = detect_node_pm(root);
        let content = std::fs::read_to_string(member.join("package.json")).ok()?;
        return parse_node_entry(&content, pm);
    }
    detect_project_structure(member).await.detected_entry(runtime)
}

/// Rewrite an entry command so it works when the build context is the repo ROOT but
/// the server lives in `subdir`. Handles `node <script> [args]` (the dominant TS-MCP
/// shape) by prefixing the script path, `npm <...>` via `--prefix`, and `pnpm <...>`
/// via `--filter ./<subdir>` (pnpm's workspace selector). Any other shape is returned
/// unchanged (we can't safely rewrite it).
pub(crate) fn prefix_entry_with_subdir(entry: &str, subdir: &str) -> String {
    mcp_detect::parse::prefix_entry_with_subdir(entry, subdir)
}

#[derive(Debug, Deserialize)]
struct MachineInfo {
    id: String,
    #[serde(default)]
    state: String,
}

/// Result of a successful deployment
pub struct DeployResult {
    pub endpoint_url: String,
    pub machine_id: Option<String>,
    /// Fly app name (e.g. "mcp-ef62e1a4"), used to fetch the server's own logs when
    /// post-deploy verification fails.
    pub app_name: String,
}

/// Generate fly.toml content for a server
fn generate_fly_toml(app_name: &str, region: &str, runtime: &str, transport: &str, memory_mb: u64, port: Option<i32>) -> String {
    // For STDIO transport, always use the STDIO adapter's port (the user's port choice,
    // if any, applies only to native HTTP/SSE servers and is ignored here).
    let internal_port = if transport == "stdio" {
        STDIO_PORT
    } else {
        // A user-supplied port (e.g. an existing HTTP server that hardcodes its own port
        // instead of reading $PORT) wins; otherwise fall back to the runtime default.
        // fly.toml sets BOTH `internal_port` and `[env] PORT` to this value, so servers
        // that read $PORT and servers that hardcode the same port both work.
        port.and_then(|p| u16::try_from(p).ok())
            .filter(|p| *p > 0)
            .unwrap_or(match runtime {
                "node" => NODE_PORT,
                "python" => PYTHON_PORT,
                "go" | "rust" => GO_RUST_PORT,
                _ => NODE_PORT,
            })
    };

    let oauth_env = if transport == "stdio" {
        format!(
            "\n  OAUTH_CALLBACK_PORT = \"9090\"\n  OAUTH_REDIRECT_BASE_URL = \"https://{app_name}.fly.dev\""
        )
    } else {
        String::new()
    };

    format!(
        r#"app = "{app_name}"
primary_region = "{region}"

[build]

[env]
  PORT = "{internal_port}"{oauth_env}

[http_service]
  internal_port = {internal_port}
  force_https = true
  auto_stop_machines = "stop"
  auto_start_machines = true
  min_machines_running = 0

  [http_service.concurrency]
    type = "connections"
    hard_limit = 100
    soft_limit = 80

[[vm]]
  memory = "{memory_mb}mb"
  cpu_kind = "shared"
  cpus = 1
"#
    )
}

/// Node package manager a project uses, which drives the install/build commands.
/// Only npm and pnpm are distinguished; yarn projects install fine via npm and are
/// left as Npm to avoid changing their (working) behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum NodePm {
    #[default]
    Npm,
    Pnpm,
}

impl NodePm {
    /// The `run` prefix used to invoke package scripts (`npm run` / `pnpm run`).
    fn runner(self) -> &'static str {
        match self {
            NodePm::Npm => "npm run",
            NodePm::Pnpm => "pnpm run",
        }
    }

    /// Map to the shared detector's package-manager enum, so entry detection uses the
    /// single shared rule (crates/detect). Only npm/pnpm are distinguished here.
    fn to_shared(self) -> mcp_detect::parse::NodePm {
        match self {
            NodePm::Npm => mcp_detect::parse::NodePm::Npm,
            NodePm::Pnpm => mcp_detect::parse::NodePm::Pnpm,
        }
    }
}

/// Project structure information detected from source directory
#[derive(Debug, Default)]
pub struct ProjectStructure {
    /// Detected Python entry point file (e.g., "server.py", "main.py")
    pub python_entry: Option<String>,
    /// Console-script name from pyproject.toml `[project.scripts]` (installed on PATH).
    /// Preferred over `python_entry` when present, mirroring how pip/uv run the package.
    pub python_script: Option<String>,
    /// Whether pyproject.toml exists
    pub has_pyproject: bool,
    /// Whether uv.lock exists (indicates uv is used)
    pub has_uv_lock: bool,
    /// Whether requirements.txt exists
    pub has_requirements_txt: bool,
    /// Whether package.json exists
    pub has_package_json: bool,
    /// Auto-detected Node start command derived from package.json
    /// (`npm start` / `node <main>` / `node <bin>`). None when undetectable.
    pub node_entry: Option<String>,
    /// Binary name produced by `cargo build` (from `[[bin]]` or `[package].name`).
    /// Used to run the correct executable instead of assuming `./server`.
    pub rust_bin: Option<String>,
    /// Dependency names from package.json (dependencies + devDependencies).
    /// Used by system-dependency provisioning to detect e.g. Playwright/Puppeteer.
    pub node_deps: Vec<String>,
    /// Dependency names from requirements.txt / pyproject.toml.
    pub python_deps: Vec<String>,
    /// Module paths from go.mod `require` entries.
    pub go_deps: Vec<String>,
    /// Detected Node package manager (npm vs pnpm), driving install/build commands.
    pub(crate) node_pm: NodePm,
}

impl ProjectStructure {
    /// Best-guess startup command (full command string) for a runtime, based purely
    /// on detected project files. Returns None when nothing could be inferred — callers
    /// fall back to the user-supplied `entry_command` or the runtime's hardcoded default.
    ///
    /// This is the source of "auto-detection": it lets stdio servers deploy without an
    /// explicit entry command, and lets SSE servers stop assuming `index.js`.
    pub(crate) fn detected_entry(&self, runtime: &str) -> Option<String> {
        match runtime {
            "node" => self.node_entry.clone(),
            "python" => self
                .python_script
                .clone()
                .or_else(|| self.python_entry.clone().map(|f| format!("python {}", f))),
            // Go always builds to /app/server in our generated Dockerfile.
            "go" => Some("./server".to_string()),
            "rust" => self.rust_bin.clone().map(|b| format!("./{}", b)),
            _ => None,
        }
    }
}

/// Detect project structure from source directory
pub async fn detect_project_structure(source_dir: &Path) -> ProjectStructure {
    let mut structure = ProjectStructure::default();

    // Check for Python project files
    structure.has_pyproject = source_dir.join("pyproject.toml").exists();
    structure.has_uv_lock = source_dir.join("uv.lock").exists();
    structure.has_requirements_txt = source_dir.join("requirements.txt").exists();
    structure.has_package_json = source_dir.join("package.json").exists();
    if structure.has_package_json {
        structure.node_pm = detect_node_pm(source_dir);
    }

    // Detect Python entry point (in priority order)
    let python_entry_candidates = [
        "server.py",
        "main.py",
        "app.py",
        "__main__.py",
        "run.py",
        "index.py",
        "mcp_server.py",
    ];

    for candidate in python_entry_candidates {
        if source_dir.join(candidate).exists() {
            structure.python_entry = Some(candidate.to_string());
            break;
        }
    }

    // Also check src directory for entry points
    if structure.python_entry.is_none() {
        let src_dir = source_dir.join("src");
        if src_dir.exists() {
            for candidate in python_entry_candidates {
                if src_dir.join(candidate).exists() {
                    structure.python_entry = Some(format!("src/{}", candidate));
                    break;
                }
            }
        }
    }

    // Parse manifests for richer start-command detection (language-aware).
    // Reads are best-effort: a malformed/absent manifest simply yields None.
    if structure.has_package_json {
        if let Ok(content) = std::fs::read_to_string(source_dir.join("package.json")) {
            structure.node_entry = parse_node_entry(&content, structure.node_pm);
        }
    }
    if structure.has_pyproject {
        if let Ok(content) = std::fs::read_to_string(source_dir.join("pyproject.toml")) {
            structure.python_script = parse_pyproject_script(&content);
        }
    }
    if source_dir.join("Cargo.toml").exists() {
        if let Ok(content) = std::fs::read_to_string(source_dir.join("Cargo.toml")) {
            structure.rust_bin = parse_cargo_bin(&content);
        }
    }

    // Dependency names — used by system-dependency provisioning. Best-effort: any
    // read/parse failure simply leaves the list empty (no provisioning, no change).
    if structure.has_package_json {
        if let Ok(content) = std::fs::read_to_string(source_dir.join("package.json")) {
            structure.node_deps = parse_node_deps(&content);
        }
    }
    if structure.has_requirements_txt {
        if let Ok(content) = std::fs::read_to_string(source_dir.join("requirements.txt")) {
            structure.python_deps.extend(parse_requirements_deps(&content));
        }
    }
    if structure.has_pyproject {
        if let Ok(content) = std::fs::read_to_string(source_dir.join("pyproject.toml")) {
            structure.python_deps.extend(parse_pyproject_deps(&content));
        }
    }
    if source_dir.join("go.mod").exists() {
        if let Ok(content) = std::fs::read_to_string(source_dir.join("go.mod")) {
            structure.go_deps = parse_go_deps(&content);
        }
    }

    structure
}

/// Collect dependency names from package.json (dependencies + devDependencies +
/// optional/peer). Lower-cased. Best-effort: returns empty on parse failure.
fn parse_node_deps(package_json: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(pkg) = serde_json::from_str::<serde_json::Value>(package_json) else {
        return out;
    };
    for section in ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"] {
        if let Some(map) = pkg.get(section).and_then(|v| v.as_object()) {
            for name in map.keys() {
                out.push(name.to_lowercase());
            }
        }
    }
    out
}

/// Extract package names from a requirements.txt (strip version specifiers/extras/markers).
fn parse_requirements_deps(requirements: &str) -> Vec<String> {
    requirements
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('-'))
        .filter_map(|l| {
            let name: String = l
                .chars()
                .take_while(|c| !matches!(c, '=' | '<' | '>' | '~' | '!' | ' ' | ';' | '[' | '(' ))
                .collect();
            let name = name.trim().to_lowercase();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

/// Extract dependency names from pyproject.toml (PEP 621 `[project].dependencies`
/// and Poetry `[tool.poetry.dependencies]`). Best-effort.
fn parse_pyproject_deps(pyproject: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(val) = toml::from_str::<toml::Value>(pyproject) else {
        return out;
    };
    // PEP 621: [project].dependencies = ["name>=1.0", ...]
    if let Some(arr) = val
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for item in arr {
            if let Some(s) = item.as_str() {
                let name: String = s
                    .chars()
                    .take_while(|c| !matches!(c, '=' | '<' | '>' | '~' | '!' | ' ' | ';' | '['))
                    .collect();
                let name = name.trim().to_lowercase();
                if !name.is_empty() {
                    out.push(name);
                }
            }
        }
    }
    // Poetry: [tool.poetry.dependencies] is a table of name = "version".
    if let Some(table) = val
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for name in table.keys() {
            out.push(name.to_lowercase());
        }
    }
    out
}

/// Collect module paths referenced in go.mod `require` entries. Best-effort.
fn parse_go_deps(go_mod: &str) -> Vec<String> {
    go_mod
        .lines()
        .map(|l| l.trim())
        .filter_map(|l| {
            // Lines look like `require github.com/x/y v1.2.3` or, inside a
            // `require ( ... )` block, just `github.com/x/y v1.2.3`.
            let l = l.strip_prefix("require ").unwrap_or(l);
            let first = l.split_whitespace().next()?;
            (first.contains('/') && first.contains('.')).then(|| first.to_lowercase())
        })
        .collect()
}

/// Derive a Node start command from `package.json` contents.
/// Priority: `scripts.start` (→ `<pm> start`) > `main` (→ `node <main>`) > `bin` (→ `node <path>`).
/// `scripts.start`/`main` are the conventional "run this app" entry; `bin` is the
/// fallback that covers stdio CLI packages which only declare an executable.
/// `pm` selects the script runner so a pnpm project runs `pnpm start`, matching how
/// its dependencies were installed (the `node <path>` forms are runner-agnostic).
fn parse_node_entry(package_json: &str, pm: NodePm) -> Option<String> {
    mcp_detect::parse::parse_node_entry(package_json, pm.to_shared())
}

/// Extract the first console-script name from a pyproject.toml `[project.scripts]` table.
/// These names are installed on PATH by pip/uv, so running the bare name starts the server.
fn parse_pyproject_script(pyproject: &str) -> Option<String> {
    mcp_detect::parse::parse_pyproject_script(pyproject)
}

/// Determine the binary name produced by `cargo build`.
/// Uses the first `[[bin]]` entry's name, falling back to `[package].name`.
fn parse_cargo_bin(cargo_toml: &str) -> Option<String> {
    mcp_detect::parse::parse_cargo_bin(cargo_toml)
}

/// Generate Dockerfile if not present
/// For STDIO transport, wraps the original command with the stdio-adapter.cjs
// ============================================================================
// System-dependency provisioning
//
// Some MCP servers need a native binary / system library that `npm|pip install`
// alone does not provide at runtime (headless browsers, ffmpeg, image libs…).
// A set of detectors inspect the project's dependency names and emit a Provision
// describing what the image needs; it is then injected into whichever Dockerfile
// (generated or user-supplied) is being built.
//
// Design guarantees:
//   - Non-regression: if no detector matches, the Provision is empty and the
//     Dockerfile is left byte-for-byte unchanged.
//   - Version-safe: browser detectors delegate to the library's own installer
//     (`npx playwright install`), so the browser always matches the installed
//     package version — it is derived, never pinned/hardcoded.
//   - Idempotent: if the Dockerfile already provisions the dependency, injection
//     is skipped (`skip_if_contains`).
//   - Fail-safe: anything ambiguous (no FROM, etc.) results in no change.
// ============================================================================

/// What an image needs to satisfy a detected system dependency.
#[derive(Debug, Default)]
pub struct Provision {
    /// apt packages to install into the (final) image.
    pub apt_packages: Vec<String>,
    /// Commands to RUN *after* application dependencies are installed
    /// (e.g. `npx playwright install --with-deps chromium`).
    pub post_install: Vec<String>,
    /// Minimum machine memory (MB) this workload needs.
    pub min_memory_mb: Option<u64>,
    /// Notes surfaced to the user that the platform cannot auto-fix
    /// (e.g. the app must launch the browser with `--no-sandbox`).
    pub warnings: Vec<String>,
    /// If the Dockerfile already contains any of these substrings, skip the
    /// `post_install` steps (the project already provisions it itself).
    pub skip_if_contains: Vec<String>,
}

impl Provision {
    pub fn is_empty(&self) -> bool {
        self.apt_packages.is_empty() && self.post_install.is_empty() && self.min_memory_mb.is_none()
    }

    fn merge(&mut self, other: Provision) {
        for p in other.apt_packages {
            if !self.apt_packages.contains(&p) {
                self.apt_packages.push(p);
            }
        }
        self.post_install.extend(other.post_install);
        self.min_memory_mb = match (self.min_memory_mb, other.min_memory_mb) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        self.warnings.extend(other.warnings);
        for s in other.skip_if_contains {
            if !self.skip_if_contains.contains(&s) {
                self.skip_if_contains.push(s);
            }
        }
    }
}

fn deps_contain(deps: &[String], needles: &[&str]) -> bool {
    deps.iter()
        .any(|d| needles.iter().any(|n| d == n || d.contains(n)))
}

type Detector = fn(&ProjectStructure) -> Option<Provision>;

/// Ordered list of detectors. Add a new one here to support another dependency.
const DETECTORS: &[Detector] = &[
    detect_playwright_node,
    detect_puppeteer_node,
    detect_playwright_python,
    detect_selenium_python,
    detect_browser_go,
    detect_ffmpeg_node,
    detect_opencv_python,
];

/// Common headless-Chromium memory floor (Chromium ~0.5–1GB; suspend cap is 2GB).
const BROWSER_MEMORY_MB: u64 = 2048;

fn detect_playwright_node(p: &ProjectStructure) -> Option<Provision> {
    deps_contain(&p.node_deps, &["playwright", "@playwright/test", "playwright-core"]).then(|| {
        Provision {
            // Installs the browser matching the *installed* Playwright version,
            // plus its OS libraries — independent of the base image.
            post_install: vec!["npx playwright install --with-deps chromium".to_string()],
            min_memory_mb: Some(BROWSER_MEMORY_MB),
            skip_if_contains: vec!["playwright install".to_string()],
            ..Default::default()
        }
    })
}

fn detect_puppeteer_node(p: &ProjectStructure) -> Option<Provision> {
    deps_contain(&p.node_deps, &["puppeteer", "puppeteer-core"]).then(|| Provision {
        post_install: vec!["npx puppeteer browsers install chrome".to_string()],
        apt_packages: chromium_runtime_libs(),
        min_memory_mb: Some(BROWSER_MEMORY_MB),
        warnings: vec![
            "Puppeteer in a container must launch with `--no-sandbox` (and ideally \
             `--disable-dev-shm-usage`). Ensure your server passes these args."
                .to_string(),
        ],
        skip_if_contains: vec![
            "puppeteer browsers install".to_string(),
            "playwright install".to_string(),
        ],
    })
}

fn detect_playwright_python(p: &ProjectStructure) -> Option<Provision> {
    deps_contain(&p.python_deps, &["playwright"]).then(|| Provision {
        post_install: vec!["playwright install --with-deps chromium".to_string()],
        min_memory_mb: Some(BROWSER_MEMORY_MB),
        skip_if_contains: vec!["playwright install".to_string()],
        ..Default::default()
    })
}

fn detect_selenium_python(p: &ProjectStructure) -> Option<Provision> {
    deps_contain(&p.python_deps, &["selenium"]).then(|| Provision {
        // Selenium Manager (>=4.6) resolves the driver; we just provide a browser.
        apt_packages: vec!["chromium".to_string(), "chromium-driver".to_string()],
        min_memory_mb: Some(BROWSER_MEMORY_MB),
        warnings: vec![
            "Selenium in a container should launch Chrome with `--no-sandbox`."
                .to_string(),
        ],
        ..Default::default()
    })
}

fn detect_browser_go(p: &ProjectStructure) -> Option<Provision> {
    deps_contain(&p.go_deps, &["go-rod/rod", "chromedp/chromedp"]).then(|| Provision {
        apt_packages: vec!["chromium".to_string()],
        min_memory_mb: Some(BROWSER_MEMORY_MB),
        ..Default::default()
    })
}

fn detect_ffmpeg_node(p: &ProjectStructure) -> Option<Provision> {
    // fluent-ffmpeg shells out to a system ffmpeg binary.
    deps_contain(&p.node_deps, &["fluent-ffmpeg"]).then(|| Provision {
        apt_packages: vec!["ffmpeg".to_string()],
        ..Default::default()
    })
}

fn detect_opencv_python(p: &ProjectStructure) -> Option<Provision> {
    deps_contain(&p.python_deps, &["opencv-python", "opencv-contrib-python"]).then(|| Provision {
        apt_packages: vec!["libgl1".to_string(), "libglib2.0-0".to_string()],
        ..Default::default()
    })
}

/// Runtime shared libraries Chromium needs when not provided by a Playwright base image.
fn chromium_runtime_libs() -> Vec<String> {
    [
        "ca-certificates", "fonts-liberation", "libasound2", "libatk-bridge2.0-0",
        "libatk1.0-0", "libcups2", "libdbus-1-3", "libdrm2", "libgbm1", "libnspr4",
        "libnss3", "libxcomposite1", "libxdamage1", "libxfixes3", "libxkbcommon0",
        "libxrandr2", "libpango-1.0-0", "libcairo2",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Run all detectors and merge their requirements.
pub fn detect_provision(project: &ProjectStructure) -> Provision {
    let mut merged = Provision::default();
    for detect in DETECTORS {
        if let Some(p) = detect(project) {
            merged.merge(p);
        }
    }
    merged
}

/// Inject a Provision into a Dockerfile (generated or user-supplied).
///
/// Returns `Some(new_dockerfile)` if anything was added, or `None` when there is
/// nothing to do / nothing safe to do (caller then leaves the file untouched).
///   - apt packages → a `RUN apt-get install …` right after the final `FROM`.
///   - post_install → `RUN …` right before the final `CMD`/`ENTRYPOINT`.
pub fn apply_provision(dockerfile: &str, provision: &Provision) -> Option<String> {
    if provision.apt_packages.is_empty() && provision.post_install.is_empty() {
        return None;
    }

    let lines: Vec<&str> = dockerfile.lines().collect();
    let upper = |l: &str| l.trim_start().to_uppercase();

    let last_from = lines.iter().rposition(|l| upper(l).starts_with("FROM "))?;
    let last_cmd = lines.iter().rposition(|l| {
        let u = upper(l);
        u.starts_with("CMD") || u.starts_with("ENTRYPOINT")
    });

    // Only add apt packages not already mentioned anywhere in the Dockerfile.
    let apt_needed: Vec<String> = provision
        .apt_packages
        .iter()
        .filter(|pkg| !dockerfile.contains(pkg.as_str()))
        .cloned()
        .collect();
    let apt_line = (!apt_needed.is_empty()).then(|| {
        format!(
            "RUN apt-get update && apt-get install -y --no-install-recommends {} && rm -rf /var/lib/apt/lists/*",
            apt_needed.join(" ")
        )
    });

    // Skip post_install entirely if the project already provisions it.
    let already_provisions = provision
        .skip_if_contains
        .iter()
        .any(|m| dockerfile.contains(m.as_str()));
    let post_install: Vec<String> = if already_provisions {
        Vec::new()
    } else {
        provision.post_install.clone()
    };

    if apt_line.is_none() && post_install.is_empty() {
        return None;
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 6);
    for (i, line) in lines.iter().enumerate() {
        // post_install goes just before the final CMD/ENTRYPOINT (deps exist by now).
        if Some(i) == last_cmd && !post_install.is_empty() {
            out.push("# nodeflare: provision system dependency (matches installed library version)".to_string());
            for cmd in &post_install {
                out.push(format!("RUN {}", cmd));
            }
        }
        out.push((*line).to_string());
        // apt packages go right after the final FROM (the runtime stage).
        if i == last_from {
            if let Some(ref a) = apt_line {
                out.push("# nodeflare: system packages".to_string());
                out.push(a.clone());
            }
        }
    }
    // No CMD/ENTRYPOINT found: append post_install as trailing build steps.
    if last_cmd.is_none() && !post_install.is_empty() {
        out.push("# nodeflare: provision system dependency".to_string());
        for cmd in &post_install {
            out.push(format!("RUN {}", cmd));
        }
    }

    Some(format!("{}\n", out.join("\n")))
}

fn generate_dockerfile(
    runtime: &str,
    transport: &str,
    mcp_path: &str,
    existing_dockerfile: Option<&str>,
    existing_entry_command: Option<&str>,
    entry_command: Option<&str>,
    build_command: Option<&str>,
    project: &ProjectStructure,
) -> String {
    // For SSE transport, use standard Dockerfiles
    if transport != "stdio" {
        return generate_sse_dockerfile(runtime, build_command, project);
    }

    // For STDIO transport, generate Dockerfile that includes the adapter
    generate_stdio_dockerfile(runtime, mcp_path, existing_dockerfile, existing_entry_command, entry_command, build_command, project)
}

/// Detect the Node package manager from lockfiles, the pnpm workspace file, or the
/// package.json `packageManager` field. Defaults to npm.
fn detect_node_pm(dir: &Path) -> NodePm {
    if dir.join("pnpm-lock.yaml").exists()
        || dir.join("pnpm-workspace.yaml").exists()
        || dir.join("pnpm-workspace.yml").exists()
    {
        return NodePm::Pnpm;
    }
    if let Ok(content) = std::fs::read_to_string(dir.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(pm) = v.get("packageManager").and_then(|p| p.as_str()) {
                if pm.trim_start().starts_with("pnpm") {
                    return NodePm::Pnpm;
                }
            }
        }
    }
    NodePm::Npm
}

/// Build step for the Node image-build stage.
/// If the user supplied a custom `build_command`, run exactly that; otherwise run the
/// project's own `build` script when present (a no-op if the script is absent).
fn node_build_step(build_command: Option<&str>, pm: NodePm) -> String {
    match build_command {
        Some(cmd) if !cmd.trim().is_empty() => format!("RUN {}", cmd.trim()),
        _ => format!("RUN {} build --if-present", pm.runner()),
    }
}

/// Dependency-install + source-copy + build section of the Node build stage.
/// npm copies manifests first for layer caching, then the source; pnpm copies the
/// whole context first because its lockfile, workspace file and member manifests are
/// all needed before `pnpm install` (which matters for workspace monorepos).
fn node_setup_section(pm: NodePm, build_command: Option<&str>) -> String {
    let build = node_build_step(build_command, pm);
    match pm {
        NodePm::Pnpm => format!(
            "COPY . .\n\
             # Install ALL deps (incl. devDependencies) so a TypeScript/bundler build can run.\n\
             RUN corepack enable && (pnpm install --frozen-lockfile 2>/dev/null || pnpm install)\n\
             # Run the project's own build script if it declares one (tsc/esbuild/etc.).\n\
             {build}"
        ),
        NodePm::Npm => format!(
            "COPY package*.json ./\n\
             # Install ALL deps (incl. devDependencies) so a TypeScript/bundler build can run.\n\
             RUN npm ci 2>/dev/null || npm install\n\
             COPY . .\n\
             # Run the project's own build script if it declares one (tsc/esbuild/etc.).\n\
             {build}"
        ),
    }
}

/// Optional build step for runtimes that have no sensible default build.
/// Emits a `RUN <build_command>` line only when the user supplied one, else nothing.
fn optional_build_step(build_command: Option<&str>) -> String {
    match build_command {
        Some(cmd) if !cmd.trim().is_empty() => format!("RUN {}\n", cmd.trim()),
        _ => String::new(),
    }
}

/// Generate standard Dockerfile for SSE transport
fn generate_sse_dockerfile(runtime: &str, build_command: Option<&str>, project: &ProjectStructure) -> String {
    match runtime {
        "node" => {
            // Install/copy/build, using the detected package manager (npm or pnpm).
            let setup = node_setup_section(project.node_pm, build_command);
            // Auto-detected start command from package.json; fall back to the historical default.
            let node_cmd = project
                .node_entry
                .as_deref()
                .map(format_entry_command_as_args)
                .unwrap_or_else(|| r#""node", "index.js""#.to_string());
            format!(r#"FROM {NODE_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
{setup}
EXPOSE {NODE_PORT}
CMD [{node_cmd}]
"#)
        },
        "python" => {
            let install_deps = generate_python_install_deps(project);
            // Optional custom build (e.g. a codegen/compile step) only when set.
            let build_step = optional_build_step(build_command);
            let python_cmd = python_default_cmd(project);
            format!(r#"FROM {PYTHON_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
{install_deps}
{build_step}EXPOSE {PYTHON_PORT}
CMD [{python_cmd}]
"#)
        },
        "go" => format!(r#"FROM {GO_BUILDER_IMAGE} AS builder
WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 GOOS=linux go build -o /app/server .
FROM {DEBIAN_RUNTIME_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/server .
EXPOSE {GO_RUST_PORT}
CMD ["./server"]
"#),
        "rust" => {
            // Copy & run the actual compiled binary (its name is the crate/bin name),
            // instead of assuming a binary literally called `server`.
            let (copy_bin, run_cmd) = rust_binary_copy_and_cmd(project);
            format!(r#"FROM {RUST_BUILDER_IMAGE} AS builder
RUN apt-get update && apt-get install -y --no-install-recommends build-essential && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {{}}" > src/main.rs
RUN cargo build --release
RUN rm -rf src
COPY . .
RUN touch src/main.rs
RUN cargo build --release
FROM {DEBIAN_RUNTIME_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
{copy_bin}
EXPOSE {GO_RUST_PORT}
CMD [{run_cmd}]
"#)
        },
        _ => format!(r#"FROM {DEBIAN_RUNTIME_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
CMD ["./start.sh"]
"#),
    }
}

/// Generate Python dependency installation commands based on project structure
fn generate_python_install_deps(project: &ProjectStructure) -> String {
    // Priority: uv (if uv.lock exists) > pyproject.toml > requirements.txt
    if project.has_uv_lock {
        r#"RUN pip install --no-cache-dir uv && uv sync --frozen && uv pip install keyrings.alt"#.to_string()
    } else if project.has_pyproject {
        r#"RUN pip install --no-cache-dir . keyrings.alt"#.to_string()
    } else if project.has_requirements_txt {
        r#"RUN pip install --no-cache-dir keyrings.alt -r requirements.txt"#.to_string()
    } else {
        r#"RUN pip install --no-cache-dir keyrings.alt"#.to_string()
    }
}

/// Default Python CMD arguments (already JSON-quoted, comma-joined) for a project.
/// Prefers a console script from `[project.scripts]`, else `python <entry-file>`,
/// else `python main.py`.
fn python_default_cmd(project: &ProjectStructure) -> String {
    if let Some(script) = project.python_script.as_deref() {
        format!("\"{}\"", script)
    } else {
        let entry = project.python_entry.as_deref().unwrap_or("main.py");
        format!("\"python\", \"{}\"", entry)
    }
}

/// Resolve the Rust binary `COPY` line and CMD arguments for the runtime stage.
/// When the binary name is known we copy exactly that file; otherwise we fall back
/// to copying every release artifact and running `./server` (legacy behaviour).
fn rust_binary_copy_and_cmd(project: &ProjectStructure) -> (String, String) {
    match project.rust_bin.as_deref() {
        Some(bin) => (
            format!("COPY --from=builder /app/target/release/{bin} ./{bin}"),
            format!("\"./{bin}\""),
        ),
        None => (
            "COPY --from=builder /app/target/release/* .".to_string(),
            "\"./server\"".to_string(),
        ),
    }
}

/// Generate Dockerfile for STDIO transport with adapter
/// The adapter wraps the STDIO MCP server and exposes it as HTTP/SSE
fn generate_stdio_dockerfile(
    runtime: &str,
    mcp_path: &str,
    existing_dockerfile: Option<&str>,
    existing_entry_command: Option<&str>,
    entry_command: Option<&str>,
    build_command: Option<&str>,
    project: &ProjectStructure,
) -> String {
    // An entry command the user (or their committed Dockerfile) stated *explicitly*.
    // Only an explicit command triggers wrapping the project's own Dockerfile — we
    // never wrap someone's Dockerfile around a merely-guessed command.
    let explicit_entry = entry_command
        .or(existing_entry_command)
        .map(|s| s.to_string());

    // If there's an existing Dockerfile with an explicit entry command, use a
    // multi-stage build that preserves the original build process.
    if let (Some(dockerfile), Some(ref entry_cmd)) = (existing_dockerfile, &explicit_entry) {
        return generate_stdio_dockerfile_with_existing(runtime, mcp_path, entry_cmd, dockerfile);
    }

    // Priority for entry command in the generated Dockerfiles below:
    // 1. User-specified entry_command (from BuildJob)
    // 2. Entry command from existing Dockerfile
    // 3. Auto-detected from project manifests (package.json / pyproject / Cargo.toml)
    let effective_entry_command = explicit_entry.or_else(|| project.detected_entry(runtime));

    // Otherwise use default Dockerfiles with auto-detected entry points
    match runtime {
        "node" => {
            // Check if entry command is an npx command that needs special handling
            if let Some(ref cmd) = effective_entry_command {
                // `parse_npx_command` only returns Some for a package spec that passes the
                // npm name grammar, so `{package}` below can never carry shell metacharacters
                // into the `RUN npm install -g` line. Args/package are additionally
                // JSON-encoded for the exec-form CMD so they can't break out of the array.
                if let Some((package, extra_args)) = parse_npx_command(cmd) {
                    // For npx commands:
                    // 1. Install package globally (puts binary in /usr/local/bin)
                    // 2. Use npx to run it (npx resolves binary name from package.json)
                    // This avoids hardcoding binary names which vary per package
                    let package_json = serde_json::to_string(&package).unwrap_or_else(|_| "\"\"".into());
                    let extra_args_str = if extra_args.is_empty() {
                        String::new()
                    } else {
                        format!(", {}", extra_args.iter().map(|a| serde_json::to_string(a).unwrap_or_else(|_| "\"\"".into())).collect::<Vec<_>>().join(", "))
                    };

                    // npx packages are prebuilt; only run a build when the user explicitly set one.
                    let build_step = optional_build_step(build_command);
                    return format!(r#"FROM {NODE_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
# Install the package globally - binary goes to /usr/local/bin which is in PATH
RUN npm install -g {package}
COPY package*.json ./
RUN npm ci --only=production 2>/dev/null || npm install --only=production || true
COPY . .
{build_step}# STDIO adapter is already copied to source directory
ENV PORT={NODE_PORT}
ENV MCP_PATH="{mcp_path}"
ENV MCP_HTTP_HOST=0.0.0.0
EXPOSE {NODE_PORT}
# Use npx to run the globally installed package
# npx resolves the correct binary name from package.json
CMD ["node", "stdio-adapter.cjs", "npx", {package_json}{extra_args_str}]
"#);
                }
            }

            // Determine Node entry command (non-npx case)
            let node_cmd = if let Some(ref cmd) = effective_entry_command {
                // Parse entry command and format for CMD
                format_entry_command_as_args(cmd)
            } else {
                // Default to node index.js
                r#""node", "index.js""#.to_string()
            };

            // Install/copy/build, using the detected package manager (npm or pnpm).
            let setup = node_setup_section(project.node_pm, build_command);
            format!(r#"FROM {NODE_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
{setup}
# STDIO adapter is already copied to source directory
ENV PORT={NODE_PORT}
ENV MCP_PATH="{mcp_path}"
ENV MCP_HTTP_HOST=0.0.0.0
EXPOSE {NODE_PORT}
CMD ["node", "stdio-adapter.cjs", {node_cmd}]
"#)
        },
        "python" => {
            // Determine Python entry command
            let python_cmd = if let Some(ref cmd) = effective_entry_command {
                // Parse entry command and format for CMD
                format_python_entry_command(cmd)
            } else {
                // Auto-detect entry point
                let entry = project.python_entry.as_deref().unwrap_or("main.py");
                format!(r#""python", "{}""#, entry)
            };

            let install_deps = generate_python_install_deps(project);
            // Optional custom build (e.g. a codegen/compile step) only when set.
            let build_step = optional_build_step(build_command);

            format!(r#"FROM {PYTHON_IMAGE}
# Install Node.js for the STDIO-to-SSE adapter
RUN apt-get update && apt-get install -y --no-install-recommends procps curl nodejs npm && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
{install_deps}
{build_step}ENV PORT={PYTHON_PORT}
ENV MCP_PATH="{mcp_path}"
ENV MCP_HTTP_HOST=0.0.0.0
EXPOSE {PYTHON_PORT}
CMD ["node", "stdio-adapter.cjs", {python_cmd}]
"#)
        },
        "go" => format!(r#"FROM {GO_BUILDER_IMAGE} AS builder
WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 GOOS=linux go build -o /app/server .

FROM {NODE_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/server .
COPY stdio-adapter.cjs .
ENV PORT={GO_RUST_PORT}
ENV MCP_PATH="{mcp_path}"
ENV MCP_HTTP_HOST=0.0.0.0
EXPOSE {GO_RUST_PORT}
CMD ["node", "stdio-adapter.cjs", "./server"]
"#),
        "rust" => {
            // Copy & run the actual compiled binary instead of assuming `./server`.
            let (copy_bin, run_cmd) = rust_binary_copy_and_cmd(project);
            format!(r#"FROM {RUST_BUILDER_IMAGE} AS builder
RUN apt-get update && apt-get install -y --no-install-recommends build-essential && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {{}}" > src/main.rs
RUN cargo build --release
RUN rm -rf src
COPY . .
RUN touch src/main.rs
RUN cargo build --release

FROM {NODE_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
{copy_bin}
COPY stdio-adapter.cjs .
ENV PORT={GO_RUST_PORT}
ENV MCP_PATH="{mcp_path}"
ENV MCP_HTTP_HOST=0.0.0.0
EXPOSE {GO_RUST_PORT}
CMD ["node", "stdio-adapter.cjs", {run_cmd}]
"#)
        },
        _ => format!(r#"FROM {NODE_IMAGE}
RUN apt-get update && apt-get install -y --no-install-recommends procps curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
ENV PORT={NODE_PORT}
ENV MCP_PATH="{mcp_path}"
ENV MCP_HTTP_HOST=0.0.0.0
EXPOSE {NODE_PORT}
CMD ["node", "stdio-adapter.cjs", "./start.sh"]
"#),
    }
}

/// Format a Python entry command for use in Dockerfile CMD.
/// Handles various formats: "python server.py", "uv run mcp-server", etc.
/// Identical encoding rules to `format_entry_command_as_args` (quote-aware split +
/// JSON-escaped elements) — see that function for the injection-safety rationale.
fn format_python_entry_command(cmd: &str) -> String {
    format_entry_command_as_args(cmd)
}

/// Generate STDIO Dockerfile that wraps an existing Dockerfile
/// Uses multi-stage build: first stage builds using original Dockerfile,
/// second stage adds Node.js and STDIO adapter
fn generate_stdio_dockerfile_with_existing(_runtime: &str, mcp_path: &str, entry_command: &str, original_dockerfile: &str) -> String {
    // Standard port for STDIO adapter
    let port = STDIO_PORT;

    // Create a multi-stage build:
    // 1. First stage: build the original Dockerfile, renamed to the "app" stage.
    // 2. Second stage: run *that* image, adding only Node.js + the STDIO adapter.

    // Modify the original Dockerfile to be a named stage
    let app_stage = convert_to_named_stage(original_dockerfile, "app");

    // Extract ENV lines from original Dockerfile to preserve PATH and other settings.
    // `FROM app` already inherits the stage's ENV, but re-stating them is harmless and
    // keeps the adapter's PATH expectations explicit.
    let env_lines = extract_env_lines(original_dockerfile);

    // Previously this stage was a fresh `node:` base with `COPY --from=app / /`, which
    // unioned the app's entire root filesystem over a different distro's libc/toolchain
    // — non-deterministic and prone to glibc/library clashes. Instead we base the
    // runtime stage directly on the app stage (so its libc, dependencies, binaries and
    // ENV are exactly what the app was built with) and only add Node.js for the adapter.
    // Node install is a no-op when the base already ships it (e.g. node-based servers).
    format!(r#"# ============================================
# Stage 1: Build using original Dockerfile
# ============================================
{app_stage}

# ============================================
# Stage 2: Run the original image + STDIO adapter (Node.js)
# ============================================
FROM app AS runtime

# Inherit the app stage's WORKDIR — do NOT reset it to /app. The wrapped Dockerfile
# built the server in its own working directory (Smithery's default is /usr/src/app)
# and its entry command is relative to that directory; forcing WORKDIR /app here would
# run the command where nothing was built (ENOENT at runtime, then restart-loop → the
# machine reports unhealthy). The adapter is referenced by absolute path below, so it
# needs no fixed WORKDIR of its own.

# Ensure Node.js is available for the adapter. No-op if the base already has it;
# otherwise install via the base image's own package manager (apt or apk).
RUN sh -c 'command -v node >/dev/null 2>&1 || \
    (command -v apt-get >/dev/null 2>&1 && apt-get update && apt-get install -y --no-install-recommends nodejs npm ca-certificates && rm -rf /var/lib/apt/lists/*) || \
    (command -v apk >/dev/null 2>&1 && apk add --no-cache nodejs npm ca-certificates) || true'

# Preserve ENV variables from original Dockerfile (especially PATH)
{env_lines}

# Copy STDIO adapter from the build context
COPY stdio-adapter.cjs /app/stdio-adapter.cjs

ENV PORT={port}
ENV MCP_PATH="{mcp_path}"
ENV MCP_HTTP_HOST=0.0.0.0
EXPOSE {port}

# Run the original entry command through the STDIO adapter
CMD ["node", "/app/stdio-adapter.cjs", {entry_cmd_json}]
"#,
        app_stage = app_stage,
        env_lines = env_lines,
        entry_cmd_json = format_entry_command_as_args(entry_command)
    )
}

/// Extract ENV lines from a Dockerfile
/// This is used to preserve environment variables like PATH when wrapping existing Dockerfiles
fn extract_env_lines(dockerfile: &str) -> String {
    let mut env_lines = Vec::new();

    for line in dockerfile.lines() {
        let trimmed = line.trim();
        if trimmed.to_uppercase().starts_with("ENV ") {
            // Skip PORT and MCP_PATH as we set them ourselves
            let upper = trimmed.to_uppercase();
            if !upper.contains("PORT=") && !upper.contains("MCP_PATH=") {
                env_lines.push(trimmed.to_string());
            }
        }
    }

    // `FROM app` already inherits the wrapped stage's ENV (including PATH), so when the
    // original Dockerfile declares no ENV of its own there is nothing to restate. We must
    // NOT fabricate a `/app`-based PATH here: it is wrong for any image built in a
    // different WORKDIR (e.g. /usr/src/app) — the same hardcoded-/app assumption that
    // broke the runtime WORKDIR. Emit nothing and rely on stage inheritance.
    env_lines.join("\n")
}

/// Convert a Dockerfile to a named build stage
/// Handles both single-stage and multi-stage Dockerfiles
fn convert_to_named_stage(dockerfile: &str, stage_name: &str) -> String {
    let lines: Vec<&str> = dockerfile.lines().collect();
    let mut result = Vec::new();
    let mut first_from_found = false;
    let mut last_from_index = 0;

    // Find the last FROM instruction (for multi-stage builds)
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.to_uppercase().starts_with("FROM ") {
            last_from_index = i;
        }
    }

    // A physical line ending in `\` continues onto the next physical line as
    // part of the same logical instruction. We must honor the keep/skip
    // decision made for the instruction's first line across its continuations,
    // otherwise a continuation that happens to start with a skipped keyword
    // (e.g. the `CMD` part of `HEALTHCHECK ... \` `CMD ...`) gets dropped while
    // its leading line is kept, leaving a dangling backslash that swallows the
    // following instruction.
    #[derive(PartialEq)]
    enum Cont {
        No,
        Keep,
        Skip,
    }
    let mut cont = Cont::No;

    // Process each line
    for (i, line) in lines.iter().enumerate() {
        let continues = line.trim_end().ends_with('\\');

        // Mid-instruction continuation: stick with the prior decision.
        match cont {
            Cont::Skip => {
                cont = if continues { Cont::Skip } else { Cont::No };
                continue;
            }
            Cont::Keep => {
                result.push(line.to_string());
                cont = if continues { Cont::Keep } else { Cont::No };
                continue;
            }
            Cont::No => {}
        }

        let trimmed = line.trim();

        // Skip ENTRYPOINT and CMD from original (we'll use our own), and
        // EXPOSE (we'll use our own port) — including any continuation lines.
        if trimmed.to_uppercase().starts_with("ENTRYPOINT") ||
           trimmed.to_uppercase().starts_with("CMD") ||
           trimmed.to_uppercase().starts_with("EXPOSE") {
            cont = if continues { Cont::Skip } else { Cont::No };
            continue;
        }

        // This line is kept; remember if its instruction continues.
        cont = if continues { Cont::Keep } else { Cont::No };

        // Add stage name to the last FROM instruction
        if trimmed.to_uppercase().starts_with("FROM ") && i == last_from_index {
            // Check if it already has an AS clause
            if trimmed.to_uppercase().contains(" AS ") {
                // Replace existing AS clause with our stage name
                let parts: Vec<&str> = trimmed.splitn(2, " AS ").collect();
                if let Some(base) = parts.first() {
                    // For multi-stage, get the part before AS
                    let base_trimmed = base.split(" AS ").next().unwrap_or(base);
                    result.push(format!("{} AS {}", base_trimmed.trim(), stage_name));
                } else {
                    result.push(format!("{} AS {}", trimmed, stage_name));
                }
            } else {
                result.push(format!("{} AS {}", trimmed, stage_name));
            }
            first_from_found = true;
        } else {
            result.push(line.to_string());
        }
    }

    // If no FROM was found, this is invalid but return as-is
    if !first_from_found {
        return dockerfile.to_string();
    }

    result.join("\n")
}

/// Format an entry command as JSON arguments for a Dockerfile exec-form CMD.
/// Uses a quote-aware tokenizer (so `node "my file.js"` stays two args, not three) and
/// JSON-encodes every token via `serde_json`. The JSON encoding is the injection guard:
/// a token containing `"`, `\` or `]` is escaped and can no longer break out of the
/// `CMD [...]` array, so an attacker-controlled entry command can't append instructions.
fn format_entry_command_as_args(cmd: &str) -> String {
    tokenize_shell(cmd)
        .iter()
        .map(|p| serde_json::to_string(p).unwrap_or_else(|_| "\"\"".to_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Validate an npx package spec ("name", "@scope/name", "name@version") against a
/// conservative subset of the npm name grammar. Anything outside this charset is
/// rejected so it can never be interpolated into the `RUN npm install -g …` shell line.
fn is_valid_npx_package(spec: &str) -> bool {
    if spec.is_empty() || spec.len() > 214 {
        return false;
    }
    // Split off an optional @version / @tag suffix (the '@' that isn't a leading scope).
    let (name, version) = match spec.rfind('@') {
        None | Some(0) => (spec, None), // leading '@' = scope marker, not a version
        Some(i) => (&spec[..i], Some(&spec[i + 1..])),
    };
    let seg_ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
    };
    let name_ok = if let Some(scoped) = name.strip_prefix('@') {
        match scoped.split_once('/') {
            Some((scope, pkg)) => seg_ok(scope) && seg_ok(pkg),
            None => false, // a scope with no package name
        }
    } else {
        seg_ok(name)
    };
    let version_ok = version.map_or(true, |v| {
        !v.is_empty()
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    });
    name_ok && version_ok
}

/// Check if command is an npx command and extract package info.
/// Returns (package_name, extra_args) only when the package spec is a valid npm name;
/// an invalid/suspicious spec returns None so the caller falls back to the plain
/// (JSON-escaped, no global-install) command path instead of shelling out with it.
fn parse_npx_command(cmd: &str) -> Option<(String, Vec<String>)> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() || parts[0] != "npx" {
        return None;
    }

    // Find the package name (skip flags like -y, --yes, etc.)
    let mut package_idx = 1;
    while package_idx < parts.len() {
        let part = parts[package_idx];
        if part == "-y" || part == "--yes" || part == "-q" || part == "--quiet" {
            package_idx += 1;
        } else if part.starts_with('-') {
            package_idx += 1;
        } else {
            break;
        }
    }

    if package_idx >= parts.len() {
        return None;
    }

    let package = parts[package_idx].to_string();

    // Reject anything that isn't a well-formed npm package spec before it can reach a
    // shell `RUN` line. (CRITICAL: prevents Dockerfile command injection via npx.)
    if !is_valid_npx_package(&package) {
        return None;
    }

    // Collect any extra arguments after the package name
    let extra_args: Vec<String> = parts[package_idx + 1..]
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Don't try to derive binary name - let npx resolve it from package.json
    Some((package, extra_args))
}

/// Build and deploy using flyctl CLI with remote builder
pub async fn build_and_deploy(
    config: &AppConfig,
    job: &BuildJob,
    source_dir: &Path,
    secrets: &[SecretEnv],
    plan_memory_ceiling_mb: u64,
    on_log: impl Fn(&str),
) -> Result<DeployResult> {
    // Use the persisted, collision-free app name carried on the job. Never recompute it
    // from a truncated UUID prefix — that mapped distinct servers onto the same Fly app.
    let app_name = job.app_name.clone();

    on_log(&format!("Preparing deployment for app: {}", app_name));

    // Detect project structure for smarter Dockerfile generation
    let project = detect_project_structure(source_dir).await;
    on_log(&format!(
        "Detected project structure: pyproject={}, uv={}, requirements={}, python_entry={:?}",
        project.has_pyproject, project.has_uv_lock, project.has_requirements_txt, project.python_entry
    ));

    // System-dependency provisioning (headless browsers, ffmpeg, native libs…).
    // Empty when nothing is detected — in that case every downstream step behaves
    // exactly as before (no Dockerfile change, default memory).
    let provision = detect_provision(&project);
    if !provision.is_empty() {
        on_log(&format!(
            "Detected system dependencies: apt={:?}, post_install={:?}, min_memory={:?}MB",
            provision.apt_packages, provision.post_install, provision.min_memory_mb
        ));
        for w in &provision.warnings {
            on_log(&format!("⚠️  {}", w));
        }
    }

    // Resolve machine memory: start from the user's choice (or 256MB default), raise
    // it to any detection floor (headless browsers need ~2GB), then clamp to what the
    // plan allows. Clamping the floor LAST is deliberate — a free-tier browser server
    // stays at its small ceiling and fails honestly via the OOM hint, instead of being
    // silently handed a 2GB machine the plan doesn't pay for.
    let fly_toml_path = source_dir.join("fly.toml");
    let requested_mb = job.memory_mb.map(|m| m as u64).unwrap_or(256);
    let floor_mb = provision.min_memory_mb.unwrap_or(0);
    let memory_mb = requested_mb
        .max(floor_mb)
        .min(plan_memory_ceiling_mb)
        .max(256);
    on_log(&format!(
        "Machine memory: {}MB (requested {}MB, detection floor {}MB, plan ceiling {}MB)",
        memory_mb, requested_mb, floor_mb, plan_memory_ceiling_mb
    ));
    let fly_toml_content =
        generate_fly_toml(&app_name, &job.region, &job.runtime, &job.transport, memory_mb, job.port);
    tokio::fs::write(&fly_toml_path, &fly_toml_content)
        .await
        .context("Failed to write fly.toml")?;
    on_log("Generated fly.toml");

    // For STDIO transport, copy the adapter script to source directory
    if job.transport == "stdio" {
        let adapter_path = source_dir.join("stdio-adapter.cjs");
        tokio::fs::write(&adapter_path, STDIO_ADAPTER_JS)
            .await
            .context("Failed to write stdio-adapter.cjs")?;
        on_log("Copied STDIO-to-SSE adapter script");
    }

    // Generate Dockerfile
    let dockerfile_path = source_dir.join("Dockerfile");
    let existing_dockerfile = if dockerfile_path.exists() {
        tokio::fs::read_to_string(&dockerfile_path).await.ok()
    } else {
        None
    };

    // Judgment B: only adopt the repo's Dockerfile if it can actually build from THIS
    // context. A Dockerfile written for a monorepo root (e.g. `COPY src/<pkg> /app`)
    // references paths that don't exist when built from a subdirectory; adopting it
    // would break the build and pin a stale entry command. Discard it — and, with it,
    // any entry command we'd otherwise extract from it — and generate our own instead.
    let existing_dockerfile = match existing_dockerfile {
        Some(df) => match dockerfile_context_fit(&df, source_dir) {
            DockerfileContextFit::Usable => Some(df),
            DockerfileContextFit::Unusable { escaping } => {
                for e in &escaping {
                    on_log(&format!(
                        "Ignoring repo Dockerfile: `{}` references `{}` which is not in the build context — generating a Dockerfile instead.",
                        e.instruction, e.source
                    ));
                }
                None
            }
        },
        None => None,
    };

    // For STDIO transport with existing Dockerfile, extract the entry command
    let existing_entry_command = if job.transport == "stdio" {
        if let Some(ref content) = existing_dockerfile {
            let cmd = extract_dockerfile_entry_command(content);
            if cmd.is_some() {
                on_log("Found existing Dockerfile with entry command, will preserve it");
            }
            cmd
        } else {
            None
        }
    } else {
        None
    };

    // Auto-detected startup command from project manifests (used when neither the user
    // nor an existing Dockerfile supplied one).
    let detected_entry = project.detected_entry(&job.runtime);

    // Log entry command source
    if let Some(ref cmd) = job.entry_command {
        on_log(&format!("Using user-specified entry command: {}", cmd));
    } else if existing_entry_command.is_some() {
        on_log("Using entry command from existing Dockerfile");
    } else if let Some(ref detected) = detected_entry {
        on_log(&format!("Auto-detected startup command: {}", detected));
    }

    // For STDIO transport an entry command is required, but auto-detection from project
    // manifests now satisfies it. We only bail when nothing could be inferred.
    if job.transport == "stdio"
        && job.entry_command.is_none()
        && existing_entry_command.is_none()
        && detected_entry.is_none()
    {
        // A workspaces monorepo root has no runnable entry of its own — the build
        // output lives under a specific member (e.g. src/filesystem/dist/index.js).
        // Don't guess: tell the user which members exist and how to target one.
        let workspaces = detect_workspace_globs(source_dir);
        if !workspaces.is_empty() {
            let members = list_workspace_members(source_dir, &workspaces);
            let target_hint = if members.is_empty() {
                workspaces.join(", ")
            } else {
                members.join(", ")
            };
            anyhow::bail!(
                "This looks like a monorepo (package.json declares workspaces: {}). \
                Nodeflare can't tell which server to run, so it won't guess. \
                Set the Root Directory to the target package — one of: {} — \
                and/or set the startup command (e.g. `node {}/dist/index.js`).",
                workspaces.join(", "),
                target_hint,
                members.first().map(String::as_str).unwrap_or("<package>"),
            );
        }

        let example = match job.runtime.as_str() {
            "node" => "node index.js, npx @modelcontextprotocol/server-xxx",
            "python" => "python main.py, uv run mcp-server",
            "go" => "./your-binary-name",
            "rust" => "./your-binary-name stdio",
            _ => "./your-command",
        };
        anyhow::bail!(
            "Entry command is required for stdio transport and could not be auto-detected. \
            Please set the startup command in the server settings (e.g., {}).",
            example
        );
    }

    // Generate Dockerfile if needed
    let should_generate = existing_dockerfile.is_none() || job.transport == "stdio";
    if should_generate {
        let dockerfile_content = generate_dockerfile(
            &job.runtime,
            &job.transport,
            &job.mcp_path,
            existing_dockerfile.as_deref(),
            existing_entry_command.as_deref(),
            job.entry_command.as_deref(),
            job.build_command.as_deref(),
            &project,
        );
        // Inject system-dependency provisioning (no-op when nothing was detected).
        let dockerfile_content =
            apply_provision(&dockerfile_content, &provision).unwrap_or(dockerfile_content);
        tokio::fs::write(&dockerfile_path, &dockerfile_content)
            .await
            .context("Failed to write Dockerfile")?;
        on_log(&format!(
            "Generated Dockerfile for {} runtime (transport: {})",
            job.runtime, job.transport
        ));
    } else if !provision.is_empty() {
        // User-supplied Dockerfile is otherwise used as-is. When the project needs a
        // system dependency (e.g. a Playwright browser whose version must match the
        // installed package), patch it in before the final CMD without the user having
        // to change their repo. No change is written if nothing safe could be injected.
        if let Some(ref existing) = existing_dockerfile {
            if let Some(patched) = apply_provision(existing, &provision) {
                tokio::fs::write(&dockerfile_path, &patched)
                    .await
                    .context("Failed to write patched Dockerfile")?;
                on_log("Injected system-dependency provisioning into existing Dockerfile");
            }
        }
    }

    // Create app if it doesn't exist
    on_log(&format!("Creating/verifying Fly.io app: {}", app_name));
    let create_output = tokio::time::timeout(
        std::time::Duration::from_secs(FLYCTL_CREATE_TIMEOUT_SECS),
        Command::new("flyctl")
            .args(["apps", "create", &app_name, "--org", &config.flyio.org_slug])
            .env("FLY_API_TOKEN", &config.flyio.api_token)
            .current_dir(source_dir)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("flyctl apps create timed out after {}s", FLYCTL_CREATE_TIMEOUT_SECS))?
    .context("Failed to run flyctl apps create")?;

    // Idempotency: a create can fail simply because the app already exists. Decide via
    // the typed Machines REST API (authoritative) rather than CLI wording — only falling
    // back to the English "already exists" substring if the REST check is itself
    // inconclusive (last resort, since localized/changed CLI text would break it).
    if !create_output.status.success() {
        let stderr = String::from_utf8_lossy(&create_output.stderr);
        if app_exists(config, &app_name).await {
            tracing::info!("Fly app {} already exists; continuing", app_name);
        } else if stderr.contains("already exists") {
            tracing::info!("Fly app {} reported as already existing; continuing", app_name);
        } else {
            return Err(anyhow::anyhow!(
                "Failed to create Fly app {}: {}",
                app_name,
                sanitize_log_output(stderr.trim(), secrets)
            ));
        }
    }

    // Set secrets if any
    if !secrets.is_empty() {
        // Validate all secrets before processing
        for secret in secrets {
            validate_secret_key(&secret.key)
                .with_context(|| format!("Invalid secret key: {}", secret.key))?;
            validate_secret_value(&secret.value)
                .with_context(|| format!("Invalid secret value for key: {}", secret.key))?;
        }

        on_log(&format!("Setting {} secrets...", secrets.len()));
        // SECURITY: feed `KEY=VALUE` pairs to `flyctl secrets import` over the child's
        // stdin instead of passing them as argv. Secret values in argv are world-readable
        // via `/proc/<pid>/cmdline` and `ps` for the lifetime of the process.
        let stdin_payload: String = secrets
            .iter()
            .map(|s| format!("{}={}\n", s.key, s.value))
            .collect();

        let mut child = Command::new("flyctl")
            .args(["secrets", "import", "--app", &app_name])
            .env("FLY_API_TOKEN", &config.flyio.api_token)
            .current_dir(source_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Failed to spawn flyctl secrets import")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(stdin_payload.as_bytes())
                .await
                .context("Failed to write secrets to flyctl stdin")?;
            stdin.shutdown().await.ok(); // close stdin so flyctl proceeds
        }

        let secrets_output = tokio::time::timeout(
            std::time::Duration::from_secs(FLYCTL_SECRETS_TIMEOUT_SECS),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("flyctl secrets import timed out after {}s", FLYCTL_SECRETS_TIMEOUT_SECS))?
        .context("Failed to import secrets")?;

        if !secrets_output.status.success() {
            let stderr = String::from_utf8_lossy(&secrets_output.stderr);
            // Sanitize stderr to prevent leaking secret values in logs
            let sanitized_stderr = sanitize_log_output(&stderr, secrets);
            tracing::warn!("Secrets warning: {}", sanitized_stderr);
        }
    }

    // Deploy with remote builder
    // Note: Region is already set in fly.toml as primary_region
    on_log("Starting remote build and deploy...");
    let deploy_output = tokio::time::timeout(
        std::time::Duration::from_secs(FLYCTL_DEPLOY_TIMEOUT_SECS),
        Command::new("flyctl")
            .args([
                "deploy",
                "--remote-only",
                "--app",
                &app_name,
                "--yes",
            ])
            .env("FLY_API_TOKEN", &config.flyio.api_token)
            .current_dir(source_dir)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("flyctl deploy timed out after {}s", FLYCTL_DEPLOY_TIMEOUT_SECS))?
    .context("Failed to run flyctl deploy")?;

    let stdout = String::from_utf8_lossy(&deploy_output.stdout);
    let stderr = String::from_utf8_lossy(&deploy_output.stderr);

    // Log output (sanitized — stdout can echo env/secret values during a deploy).
    let sanitized_stdout = sanitize_log_output(&stdout, secrets);
    for line in sanitized_stdout.lines() {
        if !line.trim().is_empty() {
            on_log(line);
        }
    }

    if !deploy_output.status.success() {
        // Sanitize stderr to prevent leaking secret values in logs.
        let sanitized_stderr = sanitize_log_output(&stderr, secrets);
        // The full log is already streamed line-by-line to the deployment logs; the
        // error *message* shows only the meaningful diagnostic lines so the real cause
        // (e.g. an `npm error` or `Cannot read file '../../tsconfig.json'`) isn't buried
        // under noise like a `tsc --help` dump.
        for line in sanitized_stderr.lines() {
            if !line.trim().is_empty() {
                on_log(&format!("ERROR: {}", line));
            }
        }
        let summary = extract_error_lines(&sanitized_stderr, 15);
        let message = if summary.is_empty() { sanitized_stderr } else { summary };
        return Err(anyhow::anyhow!("Build failed:\n{}", message));
    }

    on_log("Deployment successful!");
    let endpoint_url = format!("https://{}.fly.dev", app_name);

    // Get machine ID from Fly.io API
    let machine_id = match get_machine_id(config, &app_name).await {
        Ok(id) => {
            on_log(&format!("Machine ID: {}", id));
            Some(id)
        }
        Err(e) => {
            // Non-fatal: affinity routing degrades gracefully without it, but we must
            // not silently swallow the cause.
            tracing::warn!("Could not resolve machine id for {}: {}", app_name, e);
            None
        }
    };

    Ok(DeployResult {
        endpoint_url,
        machine_id,
        app_name,
    })
}

/// Whether a Fly app exists, via the typed Machines REST API (authoritative, exit-code
/// free). Used to make `apps create` idempotent without parsing CLI text.
async fn app_exists(config: &AppConfig, app_name: &str) -> bool {
    let client = reqwest::Client::new();
    client
        .get(format!("{}/apps/{}", FLY_API_URL, app_name))
        .header("Authorization", format!("Bearer {}", config.flyio.api_token))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Destroy a Fly.io app via flyctl. Idempotent: a missing app is treated as success,
/// so retries and double-deletes are safe. Idempotency is decided primarily via the
/// typed REST API (the app is simply gone); the English substrings are only a
/// last-resort fallback when the REST probe is itself unavailable.
pub(crate) async fn destroy_app(config: &AppConfig, app_name: &str) -> Result<()> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(FLYCTL_CREATE_TIMEOUT_SECS),
        Command::new("flyctl")
            .args(["apps", "destroy", app_name, "--yes"])
            .env("FLY_API_TOKEN", &config.flyio.api_token)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("flyctl apps destroy {} timed out", app_name))?
    .context("Failed to run flyctl apps destroy")?;
    if output.status.success() {
        return Ok(());
    }
    // Authoritative check: if the app no longer exists, the destroy is effectively done.
    if !app_exists(config, app_name).await {
        return Ok(()); // already gone — idempotent
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Last-resort fallback: REST said it still exists (or was unreachable) but the CLI
    // explicitly reported "not found" — trust that wording rather than loop forever.
    let lower = stderr.to_lowercase();
    if lower.contains("not found") || lower.contains("could not find") || lower.contains("does not exist") {
        return Ok(());
    }
    Err(anyhow::anyhow!("flyctl apps destroy {} failed: {}", app_name, stderr.trim()))
}

/// Get the machine ID for an app from Fly.io API. Prefers a machine in the `started`
/// state (the one actually serving the current release) over an arbitrary entry, so
/// affinity routing doesn't pin sessions to a stopped/replaced machine.
async fn get_machine_id(config: &AppConfig, app_name: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{}/apps/{}/machines", FLY_API_URL, app_name))
        .header("Authorization", format!("Bearer {}", config.flyio.api_token))
        .send()
        .await
        .context("Failed to list machines")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to list machines: HTTP {}",
            response.status()
        ));
    }

    let machines: Vec<MachineInfo> = response.json().await?;
    // Prefer a started machine (current release, actually serving); fall back to any.
    machines
        .iter()
        .find(|m| m.state == "started")
        .or_else(|| machines.first())
        .map(|m| format!("{}:{}", app_name, m.id))
        .ok_or_else(|| anyhow::anyhow!("No machines found"))
}

// ============================================================================
// Surfacing the real failure reason
//
// Build and runtime failures produce large, noisy logs (a `tsc --help` dump on a
// build error; a crash-loop repeating the same stack on a runtime error). Rather
// than enumerate specific failure patterns, we surface ground truth: pull the lines
// that actually carry a diagnostic, de-duplicate them, and show those. This stays
// correct for any repo/runtime — new failure shapes don't need new code.
// ============================================================================

/// Strip ANSI/VT escape sequences (Fly logs are colorized) so the message is clean.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Consume up to the final byte of the escape sequence (a letter for CSI).
            while let Some(n) = chars.next() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// De-duplication key for a log line: strips a leading ISO-8601 timestamp token (a
/// universal log convention) so the same error emitted repeatedly — e.g. once per
/// crash-loop iteration, each with a different timestamp — collapses to one entry.
fn dedup_key(line: &str) -> &str {
    let b = line.as_bytes();
    let looks_like_ts = b.len() > 20
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b.get(10) == Some(&b'T');
    if looks_like_ts {
        if let Some(sp) = line.find(' ') {
            return line[sp + 1..].trim_start();
        }
    }
    line
}

/// Pull the meaningful diagnostic lines out of a noisy build/runtime log, keeping at
/// most `max_lines` of them (the tail — the decisive error is usually last). Lines
/// are matched by generic error markers and de-duplicated so a crash-loop or repeated
/// stack collapses to its distinct lines.
pub(crate) fn extract_error_lines(log: &str, max_lines: usize) -> String {
    const MARKERS: &[&str] = &[
        "error", "fatal", "panic", "exception", "traceback",
        "cannot find", "cannot read", "not found", "no such file",
        "did not complete", "exit code", "exited with", "failed to",
        "module_not_found", "modulenotfounderror", "permission denied",
        "out of memory", "oom", "killed process", "sigkill",
    ];
    let stripped = strip_ansi(log);
    let mut seen = std::collections::HashSet::new();
    let mut lines: Vec<String> = Vec::new();
    for raw in stripped.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if MARKERS.iter().any(|m| lower.contains(m)) && seen.insert(dedup_key(line).to_string()) {
            lines.push(line.to_string());
        }
    }
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

/// Best-effort fetch of a deployed app's recent logs (the builder image already ships
/// `flyctl`). Used to surface why a server failed its post-deploy check. Returns None
/// on any error or timeout — logs are an enrichment, never required.
pub(crate) async fn fetch_app_logs(config: &AppConfig, app_name: &str) -> Option<String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        Command::new("flyctl")
            .args(["logs", "-a", app_name, "--no-tail"])
            .env("FLY_API_TOKEN", &config.flyio.api_token)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    if text.trim().is_empty() {
        None
    } else {
        Some(text.into_owned())
    }
}

// ============================================================================
// Post-deploy MCP verification (judgment D)
//
// A built image + a started machine does NOT mean the MCP server runs: a wrong
// entry path leaves the stdio adapter listening (HTTP up) while the child process
// crash-loops, so the deploy looks "successful" but every real request 500s. We
// probe a real MCP `initialize` against the deployed endpoint and only trust the
// deploy when the child actually answers.
//
// Direction: fail the deploy on a *proven* failure, which is one of two shapes —
//   (a) the adapter returns 5xx across retries (it's up, but the child is dead), or
//   (b) not a single HTTP request ever completes across all retries (nothing is
//       listening on the MCP port — a crash-loop / OOM / wrong bind).
// Only the in-between stays Inconclusive: the endpoint *did* answer at the HTTP
// layer at least once but never with a valid initialize (4xx, partial handshake) —
// that can still be cold-start noise, so we don't fail on it.
// ============================================================================

#[derive(Debug, PartialEq)]
pub(crate) enum ProbeOutcome {
    /// The MCP server answered an initialize request.
    Verified,
    /// Proven failure: either the adapter returned 5xx across retries (child dead),
    /// or no HTTP request ever completed (nothing listening on the MCP port).
    Broken(String),
    /// The endpoint answered at the HTTP layer but never with a valid initialize
    /// (4xx / partial handshake) — could still be cold start, so don't fail on it.
    Inconclusive(String),
}

/// Decide the probe outcome from what the retry loop observed. Pure so it can be
/// unit-tested without a live endpoint.
fn classify_probe(saw_server_error: bool, saw_http_response: bool, detail: String) -> ProbeOutcome {
    // `saw_server_error` ⇒ adapter up but child dead. `!saw_http_response` ⇒ the
    // probe never even completed a request, so nothing is listening — a server that
    // can't return a single byte is broken, not "inconclusive". Folding the latter
    // into Inconclusive is exactly what let an OOM crash-loop be reported as success.
    if saw_server_error || !saw_http_response {
        ProbeOutcome::Broken(detail)
    } else {
        ProbeOutcome::Inconclusive(detail)
    }
}

/// Char-safe short preview of a response body for logs/errors.
fn snippet(s: &str) -> String {
    let s = s.trim();
    let short: String = s.chars().take(200).collect();
    if short.len() < s.len() {
        format!("{}…", short)
    } else {
        short
    }
}

/// True if `body` looks like a JSON-RPC reply (json, or an SSE `data:` line).
fn looks_like_jsonrpc(body: &str) -> bool {
    let json_part = body
        .lines()
        .find_map(|l| l.trim().strip_prefix("data:").map(str::trim))
        .unwrap_or_else(|| body.trim());
    serde_json::from_str::<serde_json::Value>(json_part)
        .map(|v| v.get("result").is_some() || v.get("error").is_some() || v.get("jsonrpc").is_some())
        .unwrap_or(false)
}

/// Probe the deployed MCP server with a real `initialize` request (judgment D).
pub(crate) async fn verify_mcp_initialize(
    endpoint_url: &str,
    mcp_path: &str,
    on_log: &impl Fn(&str),
) -> ProbeOutcome {
    let path = if mcp_path.is_empty() { "/mcp" } else { mcp_path };
    let url = format!(
        "{}/{}",
        endpoint_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    // The config is static and valid, so `build()` is effectively infallible; using
    // `expect` (rather than falling back to `Client::new()`) guarantees we never silently
    // drop the per-request timeout and hang the probe.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(PROBE_REQUEST_TIMEOUT_SECS))
        .build()
        .expect("reqwest client builds from a static, valid configuration");
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "nodeflare-probe", "version": "1.0" }
        }
    });

    const ATTEMPTS: usize = PROBE_ATTEMPTS;
    let mut saw_server_error = false;
    let mut saw_http_response = false;
    let mut last_detail = String::from("no response");

    for attempt in 1..=ATTEMPTS {
        match client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => {
                saw_http_response = true;
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                if status.is_success() && looks_like_jsonrpc(&text) {
                    on_log(&format!("MCP server verified: initialize responded ({})", status));
                    return ProbeOutcome::Verified;
                }
                if status.is_server_error() {
                    saw_server_error = true;
                }
                last_detail = format!("HTTP {}: {}", status.as_u16(), snippet(&text));
            }
            Err(e) => {
                last_detail = format!("request error: {}", e);
            }
        }
        if attempt < ATTEMPTS {
            on_log(&format!(
                "Verifying MCP server… not ready yet (attempt {}/{}: {})",
                attempt, ATTEMPTS, last_detail
            ));
            tokio::time::sleep(std::time::Duration::from_secs(PROBE_RETRY_DELAY_SECS)).await;
        }
    }

    classify_probe(saw_server_error, saw_http_response, last_detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Node: package.json start-command detection ----

    #[test]
    fn node_prefers_start_script() {
        let pkg = r#"{"scripts": {"start": "node dist/server.js"}, "main": "index.js"}"#;
        assert_eq!(parse_node_entry(pkg, NodePm::Npm).as_deref(), Some("npm start"));
        // pnpm projects run the start script with pnpm, matching how deps were installed.
        assert_eq!(parse_node_entry(pkg, NodePm::Pnpm).as_deref(), Some("pnpm start"));
    }

    #[test]
    fn node_falls_back_to_main() {
        let pkg = r#"{"main": "dist/index.js"}"#;
        // `node <path>` is runner-agnostic: same for npm and pnpm.
        assert_eq!(parse_node_entry(pkg, NodePm::Npm).as_deref(), Some("node dist/index.js"));
        assert_eq!(parse_node_entry(pkg, NodePm::Pnpm).as_deref(), Some("node dist/index.js"));
    }

    #[test]
    fn node_bin_string() {
        let pkg = r#"{"bin": "./cli.js"}"#;
        assert_eq!(parse_node_entry(pkg, NodePm::Npm).as_deref(), Some("node ./cli.js"));
    }

    #[test]
    fn node_bin_object_prefers_package_name() {
        let pkg = r#"{"name": "my-mcp", "bin": {"other": "o.js", "my-mcp": "server.js"}}"#;
        assert_eq!(parse_node_entry(pkg, NodePm::Npm).as_deref(), Some("node server.js"));
    }

    #[test]
    fn node_empty_start_skipped() {
        let pkg = r#"{"scripts": {"start": "  "}, "main": "index.js"}"#;
        assert_eq!(parse_node_entry(pkg, NodePm::Npm).as_deref(), Some("node index.js"));
    }

    #[test]
    fn node_no_signal_returns_none() {
        assert_eq!(parse_node_entry(r#"{"name": "x"}"#, NodePm::Npm), None);
        assert_eq!(parse_node_entry("not json", NodePm::Npm), None);
    }

    // ---- Python: pyproject [project.scripts] ----

    #[test]
    fn pyproject_first_script() {
        let toml = "[project]\nname = \"x\"\n[project.scripts]\nmcp-server = \"x.cli:main\"\n";
        assert_eq!(parse_pyproject_script(toml).as_deref(), Some("mcp-server"));
    }

    #[test]
    fn pyproject_no_scripts() {
        let toml = "[project]\nname = \"x\"\n";
        assert_eq!(parse_pyproject_script(toml), None);
    }

    // ---- Rust: Cargo.toml binary name ----

    #[test]
    fn cargo_package_name() {
        let toml = "[package]\nname = \"my-server\"\nversion = \"0.1.0\"\n";
        assert_eq!(parse_cargo_bin(toml).as_deref(), Some("my-server"));
    }

    #[test]
    fn cargo_explicit_bin_wins() {
        let toml = "[package]\nname = \"crate-name\"\n[[bin]]\nname = \"actual-bin\"\npath = \"src/main.rs\"\n";
        assert_eq!(parse_cargo_bin(toml).as_deref(), Some("actual-bin"));
    }

    // ---- detected_entry() per runtime ----

    #[test]
    fn detected_entry_python_script_over_file() {
        let p = ProjectStructure {
            python_script: Some("mcp-server".into()),
            python_entry: Some("main.py".into()),
            ..Default::default()
        };
        assert_eq!(p.detected_entry("python").as_deref(), Some("mcp-server"));
    }

    #[test]
    fn detected_entry_python_file_fallback() {
        let p = ProjectStructure {
            python_entry: Some("server.py".into()),
            ..Default::default()
        };
        assert_eq!(p.detected_entry("python").as_deref(), Some("python server.py"));
    }

    #[test]
    fn detected_entry_rust_and_go() {
        let p = ProjectStructure {
            rust_bin: Some("mybin".into()),
            ..Default::default()
        };
        assert_eq!(p.detected_entry("rust").as_deref(), Some("./mybin"));
        assert_eq!(p.detected_entry("go").as_deref(), Some("./server"));
        assert_eq!(ProjectStructure::default().detected_entry("rust"), None);
        assert_eq!(ProjectStructure::default().detected_entry("unknown"), None);
    }

    #[test]
    fn rust_binary_copy_uses_detected_name() {
        let p = ProjectStructure { rust_bin: Some("srv".into()), ..Default::default() };
        let (copy, cmd) = rust_binary_copy_and_cmd(&p);
        assert_eq!(copy, "COPY --from=builder /app/target/release/srv ./srv");
        assert_eq!(cmd, "\"./srv\"");

        let (copy, cmd) = rust_binary_copy_and_cmd(&ProjectStructure::default());
        assert_eq!(copy, "COPY --from=builder /app/target/release/* .");
        assert_eq!(cmd, "\"./server\"");
    }

    // ---- convert_to_named_stage: line-continuation handling ----

    #[test]
    fn named_stage_basic_tags_last_from_and_strips_cmd_expose() {
        let df = "FROM node:20-slim\nWORKDIR /app\nEXPOSE 3000\nCMD [\"node\", \"server.js\"]\n";
        let out = convert_to_named_stage(df, "app");
        assert!(out.contains("FROM node:20-slim AS app"));
        assert!(!out.to_uppercase().contains("EXPOSE"));
        assert!(!out.to_uppercase().contains("CMD"));
    }

    #[test]
    fn named_stage_healthcheck_continuation_kept_intact() {
        // The bug: `HEALTHCHECK ... \` was kept but its `CMD ...` continuation
        // line was dropped, leaving a dangling backslash.
        let df = "FROM node:20-slim\nHEALTHCHECK --interval=30s --timeout=3s \\\n    CMD curl -f http://localhost/health || exit 1\n";
        let out = convert_to_named_stage(df, "app");
        // Both physical lines of the HEALTHCHECK must survive together.
        assert!(out.contains("HEALTHCHECK --interval=30s --timeout=3s \\"));
        assert!(out.contains("CMD curl -f http://localhost/health || exit 1"));
        // No instruction line may end with a dangling backslash.
        for line in out.lines() {
            if line.trim_end().ends_with('\\') {
                // Only valid if the *next* line exists as its continuation;
                // the last line of the output must never dangle.
                assert_ne!(line, out.lines().last().unwrap());
            }
        }
        assert!(!out.trim_end().ends_with('\\'));
    }

    #[test]
    fn named_stage_multi_continuation_keep_held_to_end() {
        // Two consecutive continuations; the middle/last physical line starts
        // with CMD but must be kept because line 1 was kept.
        let df = "FROM node:20-slim\nHEALTHCHECK --interval=30s \\\n  --timeout=3s \\\n  CMD curl -f http://localhost/health || exit 1\nRUN echo done\n";
        let out = convert_to_named_stage(df, "app");
        assert!(out.contains("HEALTHCHECK --interval=30s \\"));
        assert!(out.contains("--timeout=3s \\"));
        assert!(out.contains("CMD curl -f http://localhost/health || exit 1"));
        // The instruction *after* the multi-line HEALTHCHECK is preserved.
        assert!(out.contains("RUN echo done"));
        assert!(!out.trim_end().ends_with('\\'));
    }

    #[test]
    fn named_stage_multiline_cmd_fully_skipped() {
        // A multi-line CMD must be skipped entirely — no dangling backslash.
        let df = "FROM node:20-slim\nCMD [\"node\", \\\n    \"server.js\"]\nRUN echo after\n";
        let out = convert_to_named_stage(df, "app");
        assert!(!out.to_uppercase().contains("CMD"));
        assert!(!out.contains("server.js"));
        assert!(out.contains("RUN echo after"));
        assert!(!out.trim_end().ends_with('\\'));
    }

    // ---- System-dependency provisioning ----

    #[test]
    fn provision_detects_playwright_node() {
        let project = ProjectStructure {
            node_deps: vec!["playwright".to_string(), "express".to_string()],
            ..Default::default()
        };
        let p = detect_provision(&project);
        assert!(!p.is_empty());
        assert_eq!(p.min_memory_mb, Some(2048));
        assert!(p
            .post_install
            .iter()
            .any(|c| c.contains("playwright install")));
    }

    #[test]
    fn provision_empty_for_plain_api_server() {
        // No browser / native deps → nothing provisioned → builds unchanged.
        let project = ProjectStructure {
            node_deps: vec!["express".to_string(), "axios".to_string()],
            ..Default::default()
        };
        assert!(detect_provision(&project).is_empty());
    }

    #[test]
    fn apply_provision_noop_when_empty() {
        let df = "FROM node:20-slim\nCMD [\"node\", \"index.js\"]\n";
        assert_eq!(apply_provision(df, &Provision::default()), None);
    }

    #[test]
    fn apply_provision_inserts_post_install_before_cmd() {
        let project = ProjectStructure {
            node_deps: vec!["playwright".to_string()],
            ..Default::default()
        };
        let p = detect_provision(&project);
        let df = "FROM node:20-slim\nWORKDIR /app\nCOPY . .\nRUN npm ci\nCMD [\"node\", \"index.js\"]\n";
        let out = apply_provision(df, &p).expect("should inject");
        assert!(out.contains("RUN npx playwright install --with-deps chromium"));
        let run_at = out.find("playwright install").unwrap();
        let cmd_at = out.find("CMD").unwrap();
        assert!(run_at < cmd_at, "install must come before CMD");
    }

    #[test]
    fn apply_provision_idempotent_when_already_provisioned() {
        let project = ProjectStructure {
            node_deps: vec!["playwright".to_string()],
            ..Default::default()
        };
        let p = detect_provision(&project);
        // Dockerfile already runs the installer → nothing to add → no change.
        let df = "FROM node:20-slim\nRUN npm ci\nRUN npx playwright install --with-deps chromium\nCMD [\"node\", \"index.js\"]\n";
        assert_eq!(apply_provision(df, &p), None);
    }

    #[test]
    fn apply_provision_patches_existing_user_dockerfile() {
        // The note-server case: own Dockerfile, no explicit browser install.
        let project = ProjectStructure {
            node_deps: vec!["playwright".to_string()],
            ..Default::default()
        };
        let p = detect_provision(&project);
        let df = "FROM mcr.microsoft.com/playwright:v1.40.0-jammy\nWORKDIR /app\nCOPY . .\nRUN npm ci && npm run build\nEXPOSE 3000\nCMD [\"npm\", \"run\", \"start:http\"]\n";
        let out = apply_provision(df, &p).expect("should patch");
        assert!(out.contains("RUN npx playwright install --with-deps chromium"));
        assert!(out.find("playwright install").unwrap() < out.find("CMD").unwrap());
    }

    #[test]
    fn apply_provision_adds_apt_after_final_from() {
        let project = ProjectStructure {
            python_deps: vec!["selenium".to_string()],
            ..Default::default()
        };
        let p = detect_provision(&project);
        let df = "FROM python:3.11-slim\nWORKDIR /app\nCOPY . .\nRUN pip install -r requirements.txt\nCMD [\"python\", \"main.py\"]\n";
        let out = apply_provision(df, &p).expect("should inject apt");
        assert!(out.contains("apt-get install"));
        assert!(out.contains("chromium"));
    }

    // ---- stdio adapter wrapping an existing (foreign) Dockerfile ----

    #[test]
    fn stdio_wrap_preserves_foreign_workdir() {
        // A wrapped Dockerfile that builds in a non-/app WORKDIR (Smithery's default is
        // /usr/src/app). The generated runtime stage must NOT reset WORKDIR to /app, or
        // the relative entry command runs where nothing was built → ENOENT at runtime.
        let df = "FROM node:lts-alpine\nWORKDIR /usr/src/app\nCOPY . .\n\
                  RUN npm install --ignore-scripts && npm run build\nCMD [\"node\", \"dist/index.js\"]\n";
        let out = generate_stdio_dockerfile_with_existing("node", "/mcp", "npm start", df);
        // Stage 1 keeps the foreign WORKDIR so the build lands in /usr/src/app.
        assert!(out.contains("WORKDIR /usr/src/app"));
        // Stage 2 must not clobber it with a hardcoded `WORKDIR /app` instruction
        // (match the instruction line exactly — a comment mentioning it is fine).
        assert!(!out.lines().any(|l| l.trim() == "WORKDIR /app"));
        // The adapter is still referenced by absolute path, independent of cwd.
        assert!(out.contains("COPY stdio-adapter.cjs /app/stdio-adapter.cjs"));
        assert!(out.contains(r#"CMD ["node", "/app/stdio-adapter.cjs", "npm", "start"]"#));
    }

    #[test]
    fn stdio_wrap_no_env_does_not_fabricate_app_path() {
        // With no ENV in the original Dockerfile we must not fabricate a /app-based PATH:
        // `FROM app` already inherits the wrapped stage's ENV.
        let df = "FROM node:lts-alpine\nWORKDIR /usr/src/app\nCOPY . .\nRUN npm install\n\
                  CMD [\"node\", \"dist/index.js\"]\n";
        let out = generate_stdio_dockerfile_with_existing("node", "/mcp", "node dist/index.js", df);
        assert!(!out.contains("/app/node_modules/.bin"));
        assert!(!out.contains("/app/.venv/bin"));
    }

    #[test]
    fn stdio_wrap_preserves_original_env() {
        // Real ENV lines from the wrapped Dockerfile (e.g. DesktopCommander's
        // MCP_CLIENT_DOCKER) must still be carried into the runtime stage.
        let df = "FROM node:lts-alpine\nENV MCP_CLIENT_DOCKER=true\nWORKDIR /usr/src/app\n\
                  COPY . .\nRUN npm run build\nCMD [\"node\", \"dist/index.js\"]\n";
        let out = generate_stdio_dockerfile_with_existing("node", "/mcp", "npm start", df);
        assert!(out.contains("ENV MCP_CLIENT_DOCKER=true"));
    }

    // ---- judgment B: COPY/ADD context-source parsing (pure) ----

    fn raws(df: &str) -> Vec<String> {
        parse_context_sources(df).into_iter().map(|c| c.raw).collect()
    }

    #[test]
    fn ctx_sources_literal_and_multi() {
        assert_eq!(raws("COPY src/filesystem /app"), vec!["src/filesystem"]);
        assert_eq!(raws("COPY a b /dst"), vec!["a", "b"]);
    }

    #[test]
    fn ctx_sources_json_exec_form() {
        assert_eq!(raws(r#"COPY ["x", "y"]"#), vec!["x"]);
    }

    #[test]
    fn ctx_sources_flags_and_from_and_remote() {
        assert_eq!(raws("COPY --chown=node:node app /app"), vec!["app"]);
        assert!(raws("COPY --from=builder /a /b").is_empty());
        assert!(raws("COPY --from=nginx:latest /a /b").is_empty());
        assert!(raws("ADD https://h/x.tar /x").is_empty());
        assert!(raws("ADD git@github.com:o/r.git#main /x").is_empty());
    }

    #[test]
    fn ctx_sources_vars_heredoc_and_dot() {
        assert!(raws("COPY ${SRC} /app").is_empty());
        assert_eq!(raws("COPY $$literal /app"), vec!["$$literal"]);
        assert_eq!(raws("COPY . ."), vec!["."]);
    }

    #[test]
    fn ctx_sources_line_continuation_and_comments() {
        let df = "# comment\nCOPY \\\n  a \\\n  b /dst\nRUN echo hi\n";
        assert_eq!(raws(df), vec!["a", "b"]);
    }

    #[test]
    fn ctx_sources_ignores_non_copy() {
        assert!(raws("RUN npm ci\nWORKDIR /app\nFROM node:20").is_empty());
    }

    #[test]
    fn ctx_sources_glob_preserved() {
        assert_eq!(raws("COPY package*.json ./"), vec!["package*.json"]);
    }

    #[test]
    fn parse_workspaces_array_and_object() {
        assert_eq!(parse_workspaces(r#"{"workspaces":["src/*"]}"#), vec!["src/*"]);
        assert_eq!(
            parse_workspaces(r#"{"workspaces":{"packages":["a","b/*"]}}"#),
            vec!["a", "b/*"]
        );
        assert!(parse_workspaces(r#"{"name":"x"}"#).is_empty());
    }

    // ---- judgment B: context fit against a real directory tree ----

    fn write(dir: &std::path::Path, rel: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }

    #[test]
    fn fit_usable_when_all_sources_present() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "package.json");
        write(tmp.path(), "src/index.ts");
        let df = "FROM node:20\nWORKDIR /app\nCOPY package.json ./\nCOPY src ./src\nRUN npm ci";
        assert!(matches!(
            dockerfile_context_fit(df, tmp.path()),
            DockerfileContextFit::Usable
        ));
    }

    #[test]
    fn fit_unusable_monorepo_root_dockerfile_in_subdir() {
        // Simulates building modelcontextprotocol/servers' src/filesystem/Dockerfile
        // from the src/filesystem subdirectory: `COPY src/filesystem /app` escapes.
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "package.json");
        write(tmp.path(), "index.ts");
        let df = "FROM node:22-alpine AS builder\nWORKDIR /app\nCOPY src/filesystem /app\nCOPY tsconfig.json /tsconfig.json\nRUN npm install\nFROM node:22-alpine\nCOPY --from=builder /app/dist /app/dist\nENTRYPOINT [\"node\", \"/app/dist/index.js\"]";
        match dockerfile_context_fit(df, tmp.path()) {
            DockerfileContextFit::Unusable { escaping } => {
                let srcs: Vec<_> = escaping.iter().map(|e| e.source.as_str()).collect();
                assert!(srcs.contains(&"src/filesystem"));
                // The --from=builder copy must NOT be flagged.
                assert!(!srcs.iter().any(|s| s.contains("dist")));
            }
            DockerfileContextFit::Usable => panic!("should be unusable"),
        }
    }

    #[test]
    fn fit_flags_parent_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let df = "FROM node:20\nCOPY ../shared /app";
        match dockerfile_context_fit(df, tmp.path()) {
            DockerfileContextFit::Unusable { escaping } => {
                assert_eq!(escaping[0].reason, EscapeReason::ParentTraversal);
            }
            _ => panic!("should be unusable"),
        }
    }

    #[test]
    fn fit_glob_ok_when_parent_present() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "package.json");
        let df = "FROM node:20\nCOPY package*.json ./";
        assert!(matches!(
            dockerfile_context_fit(df, tmp.path()),
            DockerfileContextFit::Usable
        ));
    }

    #[test]
    fn list_workspace_members_expands_star() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("src/filesystem")).unwrap();
        std::fs::create_dir_all(tmp.path().join("src/git")).unwrap();
        let members = list_workspace_members(tmp.path(), &["src/*".to_string()]);
        assert_eq!(members, vec!["src/filesystem", "src/git"]);
    }

    // ---- strategy 3: workspace membership + entry prefixing ----

    #[test]
    fn subdir_membership() {
        let ws = vec!["src/*".to_string()];
        assert!(subdir_is_workspace_member("src/filesystem", &ws));
        assert!(subdir_is_workspace_member("/src/filesystem/", &ws));
        assert!(!subdir_is_workspace_member("src/filesystem/sub", &ws));
        assert!(!subdir_is_workspace_member("packages/a", &ws));
        assert!(subdir_is_workspace_member("a", &["*".to_string()]));
        assert!(subdir_is_workspace_member("packages/a", &["packages/a".to_string()]));
        // pnpm recursive glob: any depth under the prefix.
        let rec = vec!["packages/**".to_string()];
        assert!(subdir_is_workspace_member("packages/a", &rec));
        assert!(subdir_is_workspace_member("packages/group/a", &rec));
        assert!(!subdir_is_workspace_member("packages", &rec));
        assert!(!subdir_is_workspace_member("apps/a", &rec));
    }

    // ---- pnpm: workspace parsing + package-manager detection ----

    #[test]
    fn parse_pnpm_workspace_block_inline_and_negation() {
        let block = "packages:\n  - 'packages/*'\n  - \"apps/*\"\n  - '!**/test/**'\nonlyBuiltDependencies:\n  - esbuild\n";
        assert_eq!(parse_pnpm_workspace(block), vec!["packages/*", "apps/*"]);
        let inline = "packages: ['packages/*', 'tools/*']\n";
        assert_eq!(parse_pnpm_workspace(inline), vec!["packages/*", "tools/*"]);
        assert!(parse_pnpm_workspace("name: x\n").is_empty());
    }

    #[test]
    fn detect_workspace_globs_merges_npm_and_pnpm() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("package.json"), r#"{"name":"x"}"#).unwrap();
        std::fs::write(tmp.path().join("pnpm-workspace.yaml"), "packages:\n  - 'src/*'\n").unwrap();
        assert_eq!(detect_workspace_globs(tmp.path()), vec!["src/*"]);
    }

    #[test]
    fn detect_node_pm_signals() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("package.json"), r#"{"name":"x"}"#).unwrap();
        assert_eq!(detect_node_pm(tmp.path()), NodePm::Npm);
        std::fs::write(tmp.path().join("pnpm-lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();
        assert_eq!(detect_node_pm(tmp.path()), NodePm::Pnpm);

        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::write(tmp2.path().join("package.json"), r#"{"packageManager":"pnpm@9.1.0"}"#).unwrap();
        assert_eq!(detect_node_pm(tmp2.path()), NodePm::Pnpm);
    }

    #[test]
    fn node_setup_section_switches_on_pm() {
        let npm = node_setup_section(NodePm::Npm, None);
        assert!(npm.contains("npm ci"));
        assert!(npm.contains("npm run build --if-present"));
        assert!(!npm.contains("pnpm"));

        let pnpm = node_setup_section(NodePm::Pnpm, None);
        assert!(pnpm.contains("corepack enable"));
        assert!(pnpm.contains("pnpm install"));
        assert!(pnpm.contains("pnpm run build --if-present"));
        assert!(!pnpm.contains("npm ci"));

        // A custom build command overrides the default for either manager.
        assert!(node_setup_section(NodePm::Pnpm, Some("pnpm compile")).contains("RUN pnpm compile"));
    }

    #[test]
    fn jsonrpc_detection() {
        assert!(looks_like_jsonrpc(r#"{"jsonrpc":"2.0","id":1,"result":{"serverInfo":{}}}"#));
        assert!(looks_like_jsonrpc(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32000}}"#));
        // SSE-framed reply.
        assert!(looks_like_jsonrpc("event: message\ndata: {\"jsonrpc\":\"2.0\",\"result\":{}}\n\n"));
        // Not JSON-RPC.
        assert!(!looks_like_jsonrpc("Internal Server Error"));
        assert!(!looks_like_jsonrpc("<html>502</html>"));
    }

    #[test]
    fn snippet_is_char_safe() {
        let s = "あ".repeat(300);
        let out = snippet(&s);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 201);
    }

    #[test]
    fn strip_ansi_removes_escapes() {
        assert_eq!(strip_ansi("\u{1b}[32mhello\u{1b}[0m world"), "hello world");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn extract_error_lines_cuts_through_noise() {
        // A tsc --help dump (no error markers) followed by the real npm error.
        let log = "\
--outDir
Specify an output folder for all emitted files.
type: boolean
default: false
npm error code 1
npm error command sh -c npm run build
npm error path /app
process \"/bin/sh -c npm install\" did not complete successfully: exit code: 1";
        let out = extract_error_lines(log, 15);
        assert!(out.contains("npm error code 1"));
        assert!(out.contains("did not complete successfully"));
        // The help-listing noise is dropped.
        assert!(!out.contains("Specify an output folder"));
        assert!(!out.contains("type: boolean"));
    }

    #[test]
    fn extract_error_lines_dedups_crash_loop() {
        // A crash-loop repeats the same lines; they collapse to the distinct ones.
        let log = "\
[Adapter] MCP process exited with code 1, signal null
Error: Cannot find module '/app/dist/index.js'
[Adapter] MCP process exited with code 1, signal null
Error: Cannot find module '/app/dist/index.js'
[Adapter] MCP process exited with code 1, signal null";
        let out = extract_error_lines(log, 12);
        assert_eq!(out.matches("Cannot find module").count(), 1);
        assert_eq!(out.matches("MCP process exited").count(), 1);
    }

    #[test]
    fn extract_error_lines_surfaces_oom() {
        // The OOM kill is the root-cause line; it must survive the marker filter so
        // the user sees *why* the server never started.
        let log = "\
[Adapter] Auto-initializing MCP process...
[  360.482640] Out of memory: Killed process 662 (python) total-vm:890996kB
[Adapter] MCP process exited with code null, signal SIGKILL";
        let out = extract_error_lines(log, 12);
        assert!(out.contains("Out of memory"));
        assert!(out.contains("SIGKILL"));
    }

    // ---- Post-deploy probe classification ----

    #[test]
    fn probe_5xx_is_broken() {
        // Adapter answered but with 5xx across retries — the child is dead.
        let out = classify_probe(true, true, "HTTP 502".into());
        assert_eq!(out, ProbeOutcome::Broken("HTTP 502".into()));
    }

    #[test]
    fn probe_never_reachable_is_broken() {
        // No HTTP request ever completed (connection errors/timeouts every attempt):
        // nothing is listening — an OOM crash-loop, not cold-start noise. This is the
        // case that used to be mislabeled Inconclusive and reported as success.
        let out = classify_probe(false, false, "request error: error sending request".into());
        assert_eq!(
            out,
            ProbeOutcome::Broken("request error: error sending request".into())
        );
    }

    #[test]
    fn probe_reachable_but_no_initialize_is_inconclusive() {
        // The endpoint answered at the HTTP layer (e.g. 4xx) but never a valid
        // initialize — could still be cold start, so we don't fail the deploy.
        let out = classify_probe(false, true, "HTTP 404".into());
        assert_eq!(out, ProbeOutcome::Inconclusive("HTTP 404".into()));
    }

    #[test]
    fn prefix_entry_node_and_npm() {
        assert_eq!(
            prefix_entry_with_subdir("node dist/index.js", "src/filesystem"),
            "node src/filesystem/dist/index.js"
        );
        assert_eq!(
            prefix_entry_with_subdir("node dist/index.js /tmp", "src/filesystem"),
            "node src/filesystem/dist/index.js /tmp"
        );
        assert_eq!(
            prefix_entry_with_subdir("npm start", "src/git"),
            "npm --prefix src/git start"
        );
        // pnpm workspace member: run via pnpm's path filter.
        assert_eq!(
            prefix_entry_with_subdir("pnpm start", "packages/mcp"),
            "pnpm --filter ./packages/mcp start"
        );
        // Unrewritable shapes pass through unchanged.
        assert_eq!(prefix_entry_with_subdir("./server", "src/x"), "./server");
        assert_eq!(prefix_entry_with_subdir("node dist/index.js", ""), "node dist/index.js");
    }

    // ---- Injection guards: npx package validation + CMD encoding ----

    #[test]
    fn npx_package_validation_accepts_valid_and_rejects_injection() {
        assert!(is_valid_npx_package("server-everything"));
        assert!(is_valid_npx_package("@modelcontextprotocol/server-filesystem"));
        assert!(is_valid_npx_package("pkg@1.2.3"));
        assert!(is_valid_npx_package("@scope/pkg@latest"));
        // Shell metacharacters / traversal / spaces must be rejected.
        assert!(!is_valid_npx_package("pkg; rm -rf /"));
        assert!(!is_valid_npx_package("pkg`whoami`"));
        assert!(!is_valid_npx_package("pkg$(touch x)"));
        assert!(!is_valid_npx_package("../evil"));
        assert!(!is_valid_npx_package("pkg && curl evil"));
        assert!(!is_valid_npx_package("@scope"));
        assert!(!is_valid_npx_package(""));
    }

    #[test]
    fn parse_npx_rejects_malicious_package() {
        assert_eq!(parse_npx_command("npx -y server-everything"), Some(("server-everything".into(), vec![])));
        // Injection attempt -> None (caller falls back to the safe JSON-escaped path).
        assert_eq!(parse_npx_command("npx -y evil; rm -rf /"), None);
        assert_eq!(parse_npx_command("npx \"$(curl evil)\""), None);
    }

    #[test]
    fn entry_command_args_are_json_escaped() {
        // Decisive property: the produced CMD args always form a VALID JSON array, so no
        // attacker-controlled token can break out of `CMD [...]` to inject instructions.
        let out = format_entry_command_as_args(r#"node "a\"]; RUN evil"#);
        let parsed: Vec<String> =
            serde_json::from_str(&format!("[{}]", out)).expect("args must form a valid JSON array");
        assert_eq!(parsed[0], "node");
        // Quote-aware split keeps a quoted multi-word arg as a single element.
        assert_eq!(format_entry_command_as_args("node \"my file.js\""), r#""node", "my file.js""#);
        assert_eq!(format_entry_command_as_args("python main.py"), r#""python", "main.py""#);
    }

    #[test]
    fn parse_docker_command_json_exec_form_uses_serde() {
        // Multi-word arg with spaces is preserved (re-quoted) rather than comma-split.
        assert_eq!(
            parse_docker_command(r#"["node", "my file.js"]"#).as_deref(),
            Some(r#"node "my file.js""#)
        );
        assert_eq!(parse_docker_command(r#"["node","index.js"]"#).as_deref(), Some("node index.js"));
        assert_eq!(parse_docker_command("python main.py").as_deref(), Some("python main.py"));
    }
}

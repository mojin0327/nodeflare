use anyhow::{Context, Result};
use bollard::image::{BuildImageOptions, TagImageOptions};
use bollard::Docker;
use flate2::read::GzDecoder;
use futures::StreamExt;
use mcp_queue::BuildJob;
use regex::Regex;
use std::io::{Cursor, Read};

/// Validate GitHub repository name (owner/repo format)
/// Prevents command injection by ensuring strict format
fn validate_github_repo(repo: &str) -> Result<()> {
    // GitHub repo format: owner/repo
    // Owner: alphanumeric, hyphens (but not starting/ending with hyphen)
    // Repo: alphanumeric, hyphens, underscores, dots
    let re = Regex::new(r"^[a-zA-Z0-9][-a-zA-Z0-9]*[a-zA-Z0-9]?/[a-zA-Z0-9][-a-zA-Z0-9._]*[a-zA-Z0-9]$")
        .expect("Invalid regex");

    if !re.is_match(repo) {
        return Err(anyhow::anyhow!(
            "Invalid GitHub repository format. Expected 'owner/repo'"
        ));
    }

    // Additional safety checks
    if repo.contains("..") || repo.contains("//") {
        return Err(anyhow::anyhow!("Invalid characters in repository name"));
    }

    // Check length limits
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        return Err(anyhow::anyhow!("Repository must be in 'owner/repo' format"));
    }
    if parts[0].len() > 39 || parts[1].len() > 100 {
        return Err(anyhow::anyhow!("Repository name too long"));
    }

    Ok(())
}

/// Validate Git branch name
/// Prevents command injection through malicious branch names
fn validate_git_branch(branch: &str) -> Result<()> {
    // Git branch naming rules:
    // - Cannot start with '-' or '.'
    // - Cannot contain: space, ~, ^, :, ?, *, [, \, control chars
    // - Cannot end with '.lock' or '/'
    // - Cannot contain '..' or '@{'

    if branch.is_empty() || branch.len() > 255 {
        return Err(anyhow::anyhow!("Invalid branch name length"));
    }

    // Check for disallowed patterns
    if branch.starts_with('-') || branch.starts_with('.') {
        return Err(anyhow::anyhow!("Branch name cannot start with '-' or '.'"));
    }

    if branch.ends_with('/') || branch.ends_with(".lock") {
        return Err(anyhow::anyhow!("Invalid branch name ending"));
    }

    if branch.contains("..") || branch.contains("@{") {
        return Err(anyhow::anyhow!("Branch name contains invalid sequence"));
    }

    // Disallowed characters (including shell metacharacters)
    let disallowed = ['~', '^', ':', '?', '*', '[', '\\', ' ', '\t', '\n', '\r',
                      '\'', '"', '`', '$', '!', '&', '|', ';', '<', '>', '(', ')'];
    for c in disallowed {
        if branch.contains(c) {
            return Err(anyhow::anyhow!(
                "Branch name contains invalid character: '{}'",
                c
            ));
        }
    }

    // Must be valid UTF-8 and printable
    if !branch.chars().all(|c| c.is_ascii_graphic() || c == '/') {
        return Err(anyhow::anyhow!("Branch name contains non-printable characters"));
    }

    Ok(())
}

// Security limits for tarball processing
const MAX_TARBALL_SIZE: usize = 500 * 1024 * 1024; // 500MB max tarball
const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024; // 100MB max single file
const MAX_FILE_COUNT: usize = 10000; // Maximum files in archive
const MAX_PATH_LENGTH: usize = 500; // Maximum path length

/// Validate tar entry path for security issues
fn validate_tar_path(path: &std::path::Path) -> Result<()> {
    let path_str = path.to_string_lossy();

    // Check path length
    if path_str.len() > MAX_PATH_LENGTH {
        return Err(anyhow::anyhow!("Path too long: {}", path_str.len()));
    }

    // Check for path traversal
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err(anyhow::anyhow!(
                    "Path traversal detected: '..' in path"
                ));
            }
            std::path::Component::Normal(s) => {
                let s_str = s.to_string_lossy();
                // Check for hidden files starting with ..
                if s_str.starts_with("..") {
                    return Err(anyhow::anyhow!(
                        "Suspicious path component: {}",
                        s_str
                    ));
                }
            }
            _ => {}
        }
    }

    // Check for absolute paths
    if path.is_absolute() {
        return Err(anyhow::anyhow!("Absolute paths not allowed"));
    }

    // Check for null bytes
    if path_str.contains('\0') {
        return Err(anyhow::anyhow!("Null bytes in path"));
    }

    Ok(())
}

/// Build Docker image from GitHub tarball
pub async fn build_image_from_tarball(
    docker: &Docker,
    tarball: &[u8],
    job: &BuildJob,
    image_tag: &str,
) -> Result<()> {
    // Check tarball size
    if tarball.len() > MAX_TARBALL_SIZE {
        return Err(anyhow::anyhow!(
            "Tarball too large: {} bytes (max: {} bytes)",
            tarball.len(),
            MAX_TARBALL_SIZE
        ));
    }

    // GitHub tarball is gzipped - decompress it
    let gz = GzDecoder::new(Cursor::new(tarball));
    let mut archive = tar::Archive::new(gz);

    // Create new tar for Docker build context
    let mut build_context = tar::Builder::new(Vec::new());

    // GitHub tarballs have a top-level directory like "owner-repo-sha/"
    // We need to strip this prefix
    let mut prefix: Option<String> = None;
    let mut file_count: usize = 0;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let header = entry.header();

        // SECURITY: Skip symlinks to prevent symlink attacks
        if header.entry_type().is_symlink() || header.entry_type().is_hard_link() {
            tracing::warn!("Skipping symlink/hardlink in tarball: {:?}", path);
            continue;
        }

        // SECURITY: Validate path
        validate_tar_path(&path)?;

        // SECURITY: Check file size
        if header.size()? > MAX_FILE_SIZE {
            return Err(anyhow::anyhow!(
                "File too large: {:?} ({} bytes, max: {} bytes)",
                path,
                header.size()?,
                MAX_FILE_SIZE
            ));
        }

        // SECURITY: Limit file count
        file_count += 1;
        if file_count > MAX_FILE_COUNT {
            return Err(anyhow::anyhow!(
                "Too many files in archive (max: {})",
                MAX_FILE_COUNT
            ));
        }

        // Determine the prefix from the first entry
        if prefix.is_none() {
            if let Some(first_component) = path.components().next() {
                prefix = Some(first_component.as_os_str().to_string_lossy().to_string());
            }
        }

        // Strip the prefix
        if let Some(ref pfx) = prefix {
            if let Ok(stripped) = path.strip_prefix(pfx) {
                // SECURITY: Validate stripped path as well
                if !stripped.as_os_str().is_empty() {
                    validate_tar_path(stripped)?;

                    let mut header = entry.header().clone();
                    header.set_path(stripped)?;

                    let mut data = Vec::new();
                    entry.read_to_end(&mut data)?;

                    build_context.append(&header, &data[..])?;
                }
            }
        }
    }

    // Check if Dockerfile exists in the tarball, if not, generate one
    let tar_bytes = build_context.into_inner()?;
    let has_dockerfile = check_has_dockerfile(&tar_bytes);

    let tar_with_dockerfile = if has_dockerfile {
        tar_bytes
    } else {
        add_dockerfile_to_tar(&tar_bytes, &job.runtime)?
    };

    // Auto-upgrade Node.js version if package-lock.json requires it
    let final_tar = maybe_upgrade_node_in_tar(&tar_with_dockerfile)?;

    // Build the image
    let options = BuildImageOptions {
        t: image_tag,
        dockerfile: "Dockerfile",
        rm: true,
        ..Default::default()
    };

    let mut stream = docker.build_image(options, None, Some(final_tar.into()));

    while let Some(result) = stream.next().await {
        match result {
            Ok(output) => {
                if let Some(stream) = output.stream {
                    tracing::debug!("{}", stream.trim());
                }
                if let Some(error) = output.error {
                    return Err(anyhow::anyhow!("Build error: {}", error));
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Docker error: {}", e));
            }
        }
    }

    Ok(())
}

/// Legacy build function for when no tarball is available (public repos via git clone)
pub async fn build_image(docker: &Docker, job: &BuildJob, image_tag: &str) -> Result<()> {
    // Validate inputs to prevent command injection
    validate_github_repo(&job.github_repo)
        .context("Invalid GitHub repository name")?;
    validate_git_branch(&job.github_branch)
        .context("Invalid Git branch name")?;

    // Clone the repository using git
    let temp_dir = tempfile::tempdir()?;
    let repo_path = temp_dir.path();

    tracing::info!("Cloning {} to {:?}", job.github_repo, repo_path);

    let status = tokio::process::Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            &job.github_branch,
            &format!("https://github.com/{}.git", job.github_repo),
            repo_path.to_str().ok_or_else(|| anyhow::anyhow!("Invalid UTF-8 in repo path"))?,
        ])
        .status()
        .await
        .context("Failed to execute git clone")?;

    if !status.success() {
        return Err(anyhow::anyhow!("Git clone failed"));
    }

    // Create tar archive from cloned repo
    let mut ar = tar::Builder::new(Vec::new());
    ar.append_dir_all(".", repo_path)?;
    let tar_bytes = ar.into_inner()?;

    // Check for Dockerfile, add if missing
    let has_dockerfile = repo_path.join("Dockerfile").exists();
    let tar_with_dockerfile = if has_dockerfile {
        tar_bytes
    } else {
        add_dockerfile_to_tar(&tar_bytes, &job.runtime)?
    };

    // Auto-upgrade Node.js version if package-lock.json requires it
    let final_tar = maybe_upgrade_node_in_tar(&tar_with_dockerfile)?;

    let options = BuildImageOptions {
        t: image_tag,
        dockerfile: "Dockerfile",
        rm: true,
        ..Default::default()
    };

    let mut stream = docker.build_image(options, None, Some(final_tar.into()));

    while let Some(result) = stream.next().await {
        match result {
            Ok(output) => {
                if let Some(stream) = output.stream {
                    tracing::debug!("{}", stream.trim());
                }
                if let Some(error) = output.error {
                    return Err(anyhow::anyhow!("Build error: {}", error));
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Docker error: {}", e));
            }
        }
    }

    Ok(())
}

/// Parse a Node.js engines.node specifier and return the minimum required major version.
/// Handles OR branches by taking the minimum (compatible with any branch).
pub(crate) fn min_node_major_from_spec(spec: &str) -> Option<u32> {
    spec.split("||")
        .filter_map(|branch| {
            let b = branch.trim();
            // Match >=X, >X, ^X, ~X, or a bare number at the start
            let re = Regex::new(r"(?:>=|>|\^|~)\s*(\d+)|^(\d+)").ok()?;
            re.captures(b).and_then(|c| {
                c.get(1).or(c.get(2))
                    .and_then(|m| m.as_str().parse::<u32>().ok())
            })
        })
        .min()
}

/// Scan package-lock.json (lockfileVersion 2/3) for the highest minimum required Node.js version.
/// Returns (major_version, "package@version") or None if no engines.node constraints found.
pub(crate) fn max_node_requirement_from_lockfile(lockfile_json: &str) -> Option<(u32, String)> {
    let lockfile: serde_json::Value = serde_json::from_str(lockfile_json).ok()?;
    let packages = lockfile.get("packages")?.as_object()?;

    let mut max_major: u32 = 0;
    let mut culprit = String::new();

    for (pkg_path, pkg_info) in packages {
        if pkg_path.is_empty() {
            continue; // root package entry — skip
        }
        let Some(node_spec) = pkg_info
            .get("engines")
            .and_then(|e| e.get("node"))
            .and_then(|n| n.as_str())
        else {
            continue;
        };

        if let Some(major) = min_node_major_from_spec(node_spec) {
            if major > max_major {
                max_major = major;
                // pkg_path looks like "node_modules/undici" or nested "node_modules/a/node_modules/b"
                let name = pkg_path
                    .rsplit("node_modules/")
                    .next()
                    .unwrap_or(pkg_path)
                    .trim_matches('/');
                let version = pkg_info.get("version").and_then(|v| v.as_str()).unwrap_or("");
                culprit = if version.is_empty() {
                    name.to_string()
                } else {
                    format!("{}@{}", name, version)
                };
            }
        }
    }

    if max_major > 0 { Some((max_major, culprit)) } else { None }
}

/// Extract the Node.js major version from a Dockerfile's FROM line (first `FROM node:X` match).
pub(crate) fn node_major_in_dockerfile(dockerfile: &str) -> Option<u32> {
    let re = Regex::new(r"(?im)^\s*FROM\s+node:(\d+)").ok()?;
    re.captures(dockerfile)?.get(1)?.as_str().parse().ok()
}

/// Replace all `node:X` / `node:X.Y.Z` occurrences in a Dockerfile with `node:{target_major}`.
/// Preserves the image variant suffix (e.g. `-alpine`, `-slim`).
pub(crate) fn set_node_version_in_dockerfile(dockerfile: &str, target_major: u32) -> String {
    let re = Regex::new(r"\bnode:(\d+)(?:\.\d+)*").unwrap();
    re.replace_all(dockerfile, |_: &regex::Captures| format!("node:{}", target_major))
        .into_owned()
}

/// Read the tarball's Dockerfile and package-lock.json. If the lockfile's transitive
/// engines.node requires a newer Node.js major than the Dockerfile specifies, rewrite
/// the Dockerfile and return an updated tarball. Returns the original bytes unchanged
/// if no upgrade is needed or the tarball has no Node.js project.
fn maybe_upgrade_node_in_tar(tar_bytes: &[u8]) -> Result<Vec<u8>> {
    // Pass 1: extract Dockerfile and package-lock.json from the tarball.
    let mut dockerfile: Option<String> = None;
    let mut lockfile: Option<String> = None;

    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        match path.as_str() {
            "Dockerfile" if dockerfile.is_none() => {
                dockerfile = Some(String::from_utf8_lossy(&buf).into_owned());
            }
            "package-lock.json" if lockfile.is_none() => {
                lockfile = Some(String::from_utf8_lossy(&buf).into_owned());
            }
            _ => {}
        }
    }

    let (dockerfile_str, lockfile_str) = match (dockerfile, lockfile) {
        (Some(d), Some(l)) => (d, l),
        _ => return Ok(tar_bytes.to_vec()),
    };

    let current = match node_major_in_dockerfile(&dockerfile_str) {
        Some(v) => v,
        None => return Ok(tar_bytes.to_vec()), // Not a node image — nothing to do.
    };

    let (required, culprit) = match max_node_requirement_from_lockfile(&lockfile_str) {
        Some(v) => v,
        None => return Ok(tar_bytes.to_vec()),
    };

    if required <= current {
        return Ok(tar_bytes.to_vec());
    }

    tracing::warn!(
        "Auto-upgrading Dockerfile Node.js {} → {} (required by {})",
        current, required, culprit
    );

    let upgraded = set_node_version_in_dockerfile(&dockerfile_str, required);

    // Pass 2: rebuild the tarball, substituting the upgraded Dockerfile.
    let mut new_tar = tar::Builder::new(Vec::new());
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let mut header = entry.header().clone();
        let path = header.path()?.to_string_lossy().to_string();
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;

        if path == "Dockerfile" {
            header.set_size(upgraded.len() as u64);
            header.set_cksum();
            new_tar.append(&header, upgraded.as_bytes())?;
        } else {
            new_tar.append(&header, &data[..])?;
        }
    }

    Ok(new_tar.into_inner()?)
}

fn check_has_dockerfile(tar_bytes: &[u8]) -> bool {
    if let Ok(mut archive) = tar::Archive::new(Cursor::new(tar_bytes)).entries() {
        while let Some(Ok(entry)) = archive.next() {
            if let Ok(path) = entry.path() {
                if path.to_string_lossy() == "Dockerfile" {
                    return true;
                }
            }
        }
    }
    false
}

fn add_dockerfile_to_tar(original_tar: &[u8], runtime: &str) -> Result<Vec<u8>> {
    let dockerfile = match runtime {
        "node" => generate_node_dockerfile(),
        "python" => generate_python_dockerfile(),
        "go" => generate_go_dockerfile(),
        "rust" => generate_rust_dockerfile(),
        _ => generate_docker_dockerfile(),
    };

    let mut new_tar = tar::Builder::new(Vec::new());

    // Copy existing entries
    let mut archive = tar::Archive::new(Cursor::new(original_tar));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let header = entry.header().clone();
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        new_tar.append(&header, &data[..])?;
    }

    // Add Dockerfile
    let mut header = tar::Header::new_gnu();
    header.set_path("Dockerfile")?;
    header.set_size(dockerfile.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    new_tar.append(&header, dockerfile.as_bytes())?;

    Ok(new_tar.into_inner()?)
}

fn generate_node_dockerfile() -> String {
    r#"
FROM node:20-slim

RUN apt-get update && apt-get install -y --no-install-recommends procps curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY package*.json ./
# Install ALL deps (incl. devDependencies) so a TypeScript/bundler build can run.
RUN npm ci 2>/dev/null || npm install

COPY . .

# Run the project's own build script if it declares one (no-op when absent).
RUN npm run build --if-present

EXPOSE 3000

CMD ["node", "index.js"]
"#
    .to_string()
}

fn generate_python_dockerfile() -> String {
    r#"
FROM python:3.11-slim

RUN apt-get update && apt-get install -y --no-install-recommends procps curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

COPY . .

EXPOSE 8000

CMD ["python", "main.py"]
"#
    .to_string()
}

fn generate_go_dockerfile() -> String {
    r#"
FROM golang:1.22 AS builder

WORKDIR /app

COPY go.mod go.sum ./
RUN go mod download

COPY . .

RUN CGO_ENABLED=0 GOOS=linux go build -o /app/server .

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates procps curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/server .

EXPOSE 8080

CMD ["./server"]
"#
    .to_string()
}

fn generate_rust_dockerfile() -> String {
    r#"
FROM rust:1.75-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends build-essential && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

COPY . .
RUN touch src/main.rs
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates procps curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/mcp-server .

EXPOSE 8080

CMD ["./mcp-server"]
"#
    .to_string()
}

fn generate_docker_dockerfile() -> String {
    // Assume the repo has its own Dockerfile
    r#"
# Using repository's Dockerfile
"#
    .to_string()
}

pub async fn push_image(docker: &Docker, image_tag: &str, registry_url: &str) -> Result<String> {
    let full_tag = format!("{}/{}", registry_url, image_tag);

    docker
        .tag_image(
            image_tag,
            Some(TagImageOptions {
                repo: full_tag.clone(),
                tag: "latest".to_string(),
            }),
        )
        .await
        .context("Failed to tag image")?;

    // Push to registry
    let mut stream = docker.push_image::<String>(
        &full_tag,
        None,
        None,
    );

    while let Some(result) = stream.next().await {
        match result {
            Ok(output) => {
                if let Some(error) = output.error {
                    return Err(anyhow::anyhow!("Push error: {}", error));
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Docker error: {}", e));
            }
        }
    }

    Ok(full_tag)
}

use anyhow::{Context, Result};
use mcp_common::AppConfig;
use mcp_queue::{BuildJob, DeployJob, SecretEnv};
use std::path::Path;
use tokio::process::Command;

use crate::flyctl;

/// Result of a successful container build + run.
pub struct DeployResult {
    /// HTTP endpoint the MCP server is reachable on (e.g. `http://127.0.0.1:PORT`).
    pub endpoint_url: String,
    /// Container name (e.g. `mcp-<uuid-nohyphens>`). Used as the machine_id in the DB.
    pub container_name: String,
}

/// nerdctl build + push + run timeout (seconds).
const BUILD_TIMEOUT_SECS: u64 = 20 * 60;
const RUN_TIMEOUT_SECS: u64 = 60;
const DESTROY_TIMEOUT_SECS: u64 = 30;

// ============================================================================
// Port allocation
//
// Containers run with --network host so they share the host's network namespace.
// We derive a deterministic listen port from the container name so that the
// same container always gets the same port and no two containers collide (with
// overwhelming probability across the range 12000-61999).
// ============================================================================

pub(crate) fn derive_host_port(container_name: &str) -> u16 {
    // FNV-1a hash for good distribution over ASCII strings
    let hash = container_name.bytes().fold(2_166_136_261u32, |h, b| {
        (h ^ b as u32).wrapping_mul(16_777_619)
    });
    (12_000u32 + (hash % 50_000)) as u16
}

// ============================================================================
// Image naming
// ============================================================================

fn image_tag(config: &AppConfig, container_name: &str, tag: &str) -> String {
    if config.container.registry_base.is_empty() {
        // Local-only mode (no registry configured): tag as <container_name>:<tag>
        format!("{}:{}", container_name, tag)
    } else {
        format!("{}/{}:{}", config.container.registry_base, container_name, tag)
    }
}

// ============================================================================
// Build + run
// ============================================================================

/// Build the Docker image with nerdctl, optionally push to the registry, then
/// (re)start the container with the new image.
pub async fn build_and_run(
    config: &AppConfig,
    job: &BuildJob,
    source_dir: &Path,
    secrets: &[SecretEnv],
    plan_memory_ceiling_mb: u64,
    on_log: impl Fn(&str),
) -> Result<DeployResult> {
    let container_name = job.app_name.clone();

    // Prepare Dockerfile, stdio-adapter, validate secrets
    let prep = flyctl::prepare_build(job, source_dir, secrets, plan_memory_ceiling_mb, &on_log).await?;

    // Derive a deterministic host port for this container
    let host_port = derive_host_port(&container_name);
    on_log(&format!("Container {} will listen on host port {}", container_name, host_port));

    // Image tag: use commit sha as tag, fall back to timestamp
    let tag = if !job.commit_sha.is_empty() {
        job.commit_sha[..job.commit_sha.len().min(12)].to_string()
    } else {
        chrono::Utc::now().format("%Y%m%d%H%M%S").to_string()
    };
    let image = image_tag(config, &container_name, &tag);

    // ─── nerdctl build ───────────────────────────────────────────────────────
    on_log(&format!("Building image: {}", image));
    let build_output = tokio::time::timeout(
        std::time::Duration::from_secs(BUILD_TIMEOUT_SECS),
        Command::new("nerdctl")
            .args([
                "build",
                "--label", &format!("nodeflare.container={}", container_name),
                "--label", &format!("nodeflare.commit={}", job.commit_sha),
                "-t", &image,
                ".",
            ])
            .current_dir(source_dir)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("nerdctl build timed out after {}s", BUILD_TIMEOUT_SECS))?
    .context("Failed to run nerdctl build")?;

    let stdout = String::from_utf8_lossy(&build_output.stdout);
    let stderr = String::from_utf8_lossy(&build_output.stderr);
    let sanitized = flyctl::sanitize_log_output(&format!("{}\n{}", stdout, stderr), secrets);
    for line in sanitized.lines() {
        if !line.trim().is_empty() {
            on_log(line);
        }
    }

    if !build_output.status.success() {
        let summary = flyctl::extract_error_lines(&sanitized, 15);
        let message = if summary.is_empty() { sanitized } else { summary };
        return Err(anyhow::anyhow!("Build failed:\n{}", message));
    }
    on_log("Image built successfully");

    // ─── nerdctl push (optional, when registry is configured) ────────────────
    if !config.container.registry_base.is_empty() {
        on_log(&format!("Pushing image to registry: {}", image));
        let push_output = tokio::time::timeout(
            std::time::Duration::from_secs(BUILD_TIMEOUT_SECS),
            Command::new("nerdctl")
                .args(["push", &image])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("nerdctl push timed out"))?
        .context("Failed to run nerdctl push")?;

        if !push_output.status.success() {
            let err = String::from_utf8_lossy(&push_output.stderr);
            return Err(anyhow::anyhow!("nerdctl push failed: {}", err.trim()));
        }
        on_log("Image pushed to registry");
    }

    // ─── nerdctl run ─────────────────────────────────────────────────────────
    // Stop and remove existing container (idempotent re-deploy)
    let _ = Command::new("nerdctl")
        .args(["rm", "-f", &container_name])
        .output()
        .await;

    on_log(&format!("Starting container {}", container_name));

    // Build env args: PORT + all user secrets
    let mut env_args: Vec<String> = vec![
        "--env".to_string(), format!("PORT={}", prep.container_port),
        "--env".to_string(), format!("MCP_PATH={}", job.mcp_path),
    ];
    // Pass host port so the adapter listens on the correct port
    env_args.push("--env".to_string());
    env_args.push(format!("HOST_PORT={}", host_port));
    for secret in secrets {
        env_args.push("--env".to_string());
        env_args.push(format!("{}={}", secret.key, secret.value));
    }

    let memory_arg = format!("{}m", prep.memory_mb);

    let mut run_cmd_args = vec![
        "run".to_string(), "-d".to_string(),
        "--name".to_string(), container_name.clone(),
        "--network".to_string(), "host".to_string(),
        "--memory".to_string(), memory_arg,
        "--restart".to_string(), "unless-stopped".to_string(),
        "--label".to_string(), format!("nodeflare.container={}", container_name),
    ];
    run_cmd_args.extend(env_args);
    run_cmd_args.push(image.clone());

    let run_output = tokio::time::timeout(
        std::time::Duration::from_secs(RUN_TIMEOUT_SECS),
        Command::new("nerdctl")
            .args(&run_cmd_args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("nerdctl run timed out after {}s", RUN_TIMEOUT_SECS))?
    .context("Failed to run nerdctl run")?;

    if !run_output.status.success() {
        let err = String::from_utf8_lossy(&run_output.stderr);
        return Err(anyhow::anyhow!("nerdctl run failed: {}", err.trim()));
    }

    let endpoint_url = format!("http://127.0.0.1:{}", host_port);
    on_log(&format!("Container started — endpoint: {}", endpoint_url));

    Ok(DeployResult { endpoint_url, container_name })
}

// ============================================================================
// Re-deploy (pre-built image)
// ============================================================================

/// Run a pre-built image as a container. Used by the DeployJob path (image already
/// built and pushed; just (re)start the container).
pub async fn run_container(config: &AppConfig, job: &DeployJob) -> Result<String> {
    let container_name = job.app_name.clone();
    let host_port = derive_host_port(&container_name);

    // Stop and remove existing container
    let _ = Command::new("nerdctl")
        .args(["rm", "-f", &container_name])
        .output()
        .await;

    let mut env_args: Vec<String> = vec![
        "--env".to_string(), format!("PORT=3000"),
    ];
    for secret in &job.secrets {
        env_args.push("--env".to_string());
        env_args.push(format!("{}={}", secret.key, secret.value));
    }

    // Resolve image: if registry configured, use full image URL from job
    let image = if config.container.registry_base.is_empty() {
        job.image_url.clone()
    } else {
        job.image_url.clone()
    };

    let mut run_args = vec![
        "run".to_string(), "-d".to_string(),
        "--name".to_string(), container_name.clone(),
        "--network".to_string(), "host".to_string(),
        "--restart".to_string(), "unless-stopped".to_string(),
        "--label".to_string(), format!("nodeflare.container={}", container_name),
    ];
    run_args.extend(env_args);
    run_args.push(image);

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(RUN_TIMEOUT_SECS),
        Command::new("nerdctl")
            .args(&run_args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("nerdctl run timed out"))?
    .context("Failed to run nerdctl run")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("nerdctl run failed: {}", err.trim()));
    }

    Ok(format!("http://127.0.0.1:{}", host_port))
}

// ============================================================================
// Destroy
// ============================================================================

/// Stop and remove a container. Idempotent: if the container doesn't exist, returns Ok.
pub(crate) async fn destroy_container(container_name: &str) -> Result<()> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(DESTROY_TIMEOUT_SECS),
        Command::new("nerdctl")
            .args(["rm", "-f", container_name])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("nerdctl rm -f {} timed out", container_name))?
    .context("Failed to run nerdctl rm")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let lower = stderr.to_lowercase();
    // "no such container" means it's already gone — treat as success (idempotent)
    if lower.contains("no such container") || lower.contains("not found") {
        return Ok(());
    }

    Err(anyhow::anyhow!("nerdctl rm -f {} failed: {}", container_name, stderr.trim()))
}

// ============================================================================
// Logs
// ============================================================================

/// Fetch recent container logs. Returns None on any error or if logs are empty.
pub(crate) async fn fetch_container_logs(container_name: &str) -> Option<String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        Command::new("nerdctl")
            .args(["logs", "--tail", "200", container_name])
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = text.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

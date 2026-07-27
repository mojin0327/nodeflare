use anyhow::{Context, Result};
use mcp_common::AppConfig;
use mcp_queue::{BuildJob, DeployJob, SecretEnv};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::flyctl;

/// Spawn a command and stream stdout+stderr through `on_log` line-by-line.
/// Returns true if the process exited successfully.
async fn run_streaming(
    mut cmd: Command,
    secrets: &[SecretEnv],
    on_log: impl Fn(&str),
) -> Result<bool> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("Failed to spawn process")?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let tx2 = tx.clone();

    let h1 = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(line);
        }
    });
    let h2 = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx2.send(line);
        }
    });
    // When both reader tasks finish, rx will close.
    tokio::spawn(async move { let _ = tokio::join!(h1, h2); });

    while let Some(line) = rx.recv().await {
        let sanitized = flyctl::sanitize_log_output(&line, secrets);
        if !sanitized.trim().is_empty() {
            on_log(&sanitized);
        }
    }

    let status = child.wait().await?;
    Ok(status.success())
}

/// Result of a successful container build + run.
pub struct DeployResult {
    /// HTTP endpoint the MCP server is reachable on (e.g. `http://127.0.0.1:PORT`).
    pub endpoint_url: String,
    /// Container name (e.g. `mcp-<uuid-nohyphens>`). Used as the machine_id in the DB.
    pub container_name: String,
}

/// nerdctl build + push timeout (seconds).
const BUILD_TIMEOUT_SECS: u64 = 20 * 60;

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

    // PORT in the VM must match derive_host_port so the iptables DNAT rule is consistent.
    // Compute it once here; it is reused for env_pairs and (inside boot()) for the DNAT rule.
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
    let mut build_cmd = Command::new("nerdctl");
    build_cmd
        .args([
            "--address", "/run/containerd/containerd.sock",
            "build",
            "--label", &format!("nodeflare.container={}", container_name),
            "--label", &format!("nodeflare.commit={}", job.commit_sha),
            "-t", &image,
            ".",
        ])
        .current_dir(source_dir);
    let build_ok = tokio::time::timeout(
        std::time::Duration::from_secs(BUILD_TIMEOUT_SECS),
        run_streaming(build_cmd, secrets, |l| on_log(l)),
    )
    .await
    .map_err(|_| anyhow::anyhow!("nerdctl build timed out after {}s", BUILD_TIMEOUT_SECS))?
    .context("Failed to run nerdctl build")?;

    if !build_ok {
        return Err(anyhow::anyhow!("Build failed — see build logs above"));
    }
    on_log("Image built successfully");

    // ─── nerdctl push (optional, when registry is configured) ────────────────
    if !config.container.registry_base.is_empty() {
        on_log(&format!("Pushing image to registry: {}", image));
        let mut push_cmd = Command::new("nerdctl");
        push_cmd.args(["--address", "/run/containerd/containerd.sock", "push", &image]);
        let push_ok = tokio::time::timeout(
            std::time::Duration::from_secs(BUILD_TIMEOUT_SECS),
            run_streaming(push_cmd, secrets, |l| on_log(l)),
        )
        .await
        .map_err(|_| anyhow::anyhow!("nerdctl push timed out"))?
        .context("Failed to run nerdctl push")?;

        if !push_ok {
            return Err(anyhow::anyhow!("nerdctl push failed — see build logs above"));
        }
        on_log("Image pushed to registry");
    }

    // ─── Firecracker boot ────────────────────────────────────────────────────
    on_log(&format!("Booting Firecracker VM {}", container_name));

    let mut env_pairs: Vec<(String, String)> = vec![
        ("PORT".to_string(), host_port.to_string()),
        ("MCP_PATH".to_string(), job.mcp_path.clone()),
    ];
    for secret in secrets {
        env_pairs.push((secret.key.clone(), secret.value.clone()));
    }

    let host_port = crate::firecracker::boot(&container_name, &image, prep.memory_mb, &env_pairs)
        .await
        .context("Firecracker VM boot failed")?;

    let endpoint_url = format!("http://127.0.0.1:{}", host_port);
    on_log(&format!("VM started — endpoint: {}", endpoint_url));

    Ok(DeployResult { endpoint_url, container_name })
}

// ============================================================================
// Re-deploy (pre-built image)
// ============================================================================

/// Run a pre-built image as a Firecracker VM. Used by the DeployJob path (image already
/// built and pushed; restore from snapshot if available, otherwise boot from image).
pub async fn run_container(_config: &AppConfig, job: &DeployJob) -> Result<String> {
    let container_name = job.app_name.clone();

    // Fast path: restore a previously snapshotted VM (< 500 ms)
    if let Ok(host_port) = crate::firecracker::restore(&container_name).await {
        tracing::info!("run_container: restored VM {} from snapshot", container_name);
        return Ok(format!("http://127.0.0.1:{}", host_port));
    }

    // No snapshot — boot a fresh VM from the pre-built image
    let host_port_num = derive_host_port(&container_name);
    let mut env_pairs: Vec<(String, String)> = vec![
        ("PORT".to_string(), host_port_num.to_string()),
    ];
    env_pairs.extend(job.secrets.iter().map(|s| (s.key.clone(), s.value.clone())));

    let host_port = crate::firecracker::boot(&container_name, &job.image_url, 256, &env_pairs)
        .await
        .context("Firecracker VM boot failed for re-deploy")?;

    Ok(format!("http://127.0.0.1:{}", host_port))
}

// ============================================================================
// Destroy
// ============================================================================

/// Permanently destroy a server's Firecracker VM and all its state (rootfs + snapshots).
/// Idempotent: missing VMs / missing files are treated as already-gone.
pub(crate) async fn destroy_container(container_name: &str) -> Result<()> {
    crate::firecracker::destroy(container_name).await
}

// ============================================================================
// Logs
// ============================================================================

/// Fetch recent VM logs. Firecracker VMs write stdout/stderr via the virtio
/// console (ttyS0); reading from the host requires a log FIFO which is not
/// yet wired up. Returns None until that is implemented.
pub(crate) async fn fetch_container_logs(_container_name: &str) -> Option<String> {
    None
}

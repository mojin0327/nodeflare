/// Firecracker microVM lifecycle management.
///
/// Each MCP server runs as a Firecracker VM:
///   - Kernel-level isolation (KVM) between tenants
///   - Full snapshot/restore: idle VMs are snapshotted and killed; on the next
///     request the snapshot is loaded and the VM resumes in < 500 ms
///
/// Boot flow for a new VM:
///   1. nerdctl create  (filesystem only; env vars are NOT passed here)
///   2. nerdctl inspect → CMD, Entrypoint, WorkingDir, image-level ENV
///   3. nerdctl export -o → merged filesystem tarball
///   4. mkfs.ext4 + tar extraction into ext4 image
///   5. Inject /init script: mounts /proc|/sys|/dev, exports env vars, exec CMD
///   6. Firecracker: TAP device + configure VM (init=/init in boot args) + boot
///
/// Scale-to-zero:
///   idle:  snapshot → kill VM   (rootfs + snapshot persist on disk)
///   wake:  restore snapshot      (< 500 ms)
///
/// Cache invalidation:
///   The rootfs is re-extracted when either the image tag or the runtime env
///   fingerprint changes (secrets updated without new code → new env hash → re-extract).
use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::time::timeout;

use crate::containerd::derive_host_port;
use crate::tap::{ForkNetns, TapDevice};

const KERNEL_PATH: &str = "/opt/firecracker/kernels/vmlinux-5.10";
const ROOTFS_DIR: &str = "/opt/firecracker/rootfs";
const SOCKET_DIR: &str = "/opt/firecracker/sockets";
const SNAPSHOT_DIR: &str = "/opt/firecracker/snapshots";

const BOOT_TIMEOUT_SECS: u64 = 60;
const RESTORE_TIMEOUT_SECS: u64 = 10;
const SNAPSHOT_TIMEOUT_SECS: u64 = 15;
/// Timeout for each individual Firecracker API call (pause, snapshot/create, etc.)
const API_CALL_TIMEOUT_SECS: u64 = 10;

// ============================================================================
// Public API
// ============================================================================

/// Boot a new Firecracker VM from an OCI image.
///
/// `env` must include `PORT` set to `derive_host_port(container_name)` so the
/// in-VM process listens on the same port the iptables DNAT rule forwards to.
pub async fn boot(
    container_name: &str,
    image: &str,
    memory_mb: u64,
    env: &[(String, String)],
) -> Result<u16> {
    let host_port = derive_host_port(container_name);
    let tap = TapDevice::derive(container_name);
    let rootfs = rootfs_path(container_name);
    let socket = socket_path(container_name);

    kill_firecracker_process(container_name).await;

    extract_rootfs(container_name, image, env, &rootfs).await
        .context("failed to extract rootfs from OCI image")?;

    tap.destroy(host_port).await.ok();
    tap.create().await.context("failed to create TAP device")?;

    start_firecracker_daemon(container_name, &socket).await
        .context("failed to start firecracker daemon")?;

    configure_and_start(&socket, &rootfs, &tap, memory_mb).await
        .context("failed to configure/start firecracker VM")?;

    tap.add_port_forward(host_port).await
        .context("failed to set up iptables port forward")?;

    wait_for_port(host_port, BOOT_TIMEOUT_SECS).await
        .with_context(|| format!("VM {} never listened on port {} ({}s timeout)", container_name, host_port, BOOT_TIMEOUT_SECS))?;

    tracing::info!("firecracker: VM {} booted, port={}", container_name, host_port);
    Ok(host_port)
}

/// Create a full memory + disk snapshot, then kill the VM process.
pub async fn snapshot(container_name: &str) -> Result<()> {
    let socket = socket_path(container_name);
    let snap_dir = snapshot_dir(container_name);
    tokio::fs::create_dir_all(&snap_dir).await?;

    let snap_path = snap_dir.join("snapshot.bin");
    let mem_path = snap_dir.join("memory.bin");

    let result: Result<()> = async {
        // Pause must complete within a bounded time to avoid hanging the scaler.
        timeout(
            Duration::from_secs(API_CALL_TIMEOUT_SECS),
            api_patch(&socket, "/vm", &json!({"state": "Paused"})),
        )
        .await
        .map_err(|_| anyhow::anyhow!("pause timed out after {}s", API_CALL_TIMEOUT_SECS))?
        .context("failed to pause VM for snapshot")?;

        timeout(
            Duration::from_secs(SNAPSHOT_TIMEOUT_SECS),
            api_put(&socket, "/snapshot/create", &json!({
                "snapshot_type": "Full",
                "snapshot_path": snap_path.to_str().unwrap(),
                "mem_file_path": mem_path.to_str().unwrap(),
            })),
        )
        .await
        .map_err(|_| anyhow::anyhow!("snapshot/create timed out after {}s", SNAPSHOT_TIMEOUT_SECS))?
        .context("snapshot/create API failed")?;

        Ok(())
    }
    .await;

    if let Err(ref e) = result {
        // If no snapshot files were written, remove the empty dir so it doesn't
        // accumulate as a false positive for "has snapshot" checks.
        if !snap_path.exists() && !mem_path.exists() {
            tracing::warn!(
                "firecracker: snapshot failed for {}, removing empty dir: {}",
                container_name, e
            );
            let _ = tokio::fs::remove_dir_all(&snap_dir).await;
        }
        return result;
    }

    tracing::info!("firecracker: snapshot saved for {}", container_name);

    kill_firecracker_process(container_name).await;
    let tap = TapDevice::derive(container_name);
    tap.destroy(derive_host_port(container_name)).await.ok();

    Ok(())
}

/// Restore a VM from a previously created snapshot.
pub async fn restore(container_name: &str) -> Result<u16> {
    let host_port = derive_host_port(container_name);
    let tap = TapDevice::derive(container_name);
    let socket = socket_path(container_name);
    let snap_dir = snapshot_dir(container_name);
    let snap_path = snap_dir.join("snapshot.bin");
    let mem_path = snap_dir.join("memory.bin");

    if !snap_path.exists() || !mem_path.exists() {
        anyhow::bail!("no snapshot found for {}", container_name);
    }

    kill_firecracker_process(container_name).await;
    tap.destroy(host_port).await.ok();
    tap.create().await.context("failed to recreate TAP device")?;

    start_firecracker_daemon(container_name, &socket).await
        .context("failed to start firecracker daemon for restore")?;

    timeout(
        Duration::from_secs(RESTORE_TIMEOUT_SECS),
        api_put(&socket, "/snapshot/load", &json!({
            "snapshot_path": snap_path.to_str().unwrap(),
            "mem_backend": {
                "backend_type": "File",
                "backend_path": mem_path.to_str().unwrap(),
            },
            "enable_diff_snapshots": false,
            "resume_vm": true,
        })),
    )
    .await
    .map_err(|_| anyhow::anyhow!("snapshot/load timed out after {}s", RESTORE_TIMEOUT_SECS))?
    .context("snapshot/load API failed")?;

    tap.add_port_forward(host_port).await
        .context("failed to set up iptables port forward")?;

    wait_for_port(host_port, RESTORE_TIMEOUT_SECS).await
        .with_context(|| format!("VM {} did not resume on port {} in {}s", container_name, host_port, RESTORE_TIMEOUT_SECS))?;

    tracing::info!("firecracker: VM {} restored from snapshot", container_name);
    Ok(host_port)
}

/// Fork an existing snapshot into a new, isolated VM instance.
///
/// Restores `base_container`'s snapshot under `fork_container` with a
/// sparse-copied rootfs so writes are isolated. The fork has no DB entry;
/// call `kill_fork` to clean up after the request completes.
pub async fn restore_fork(base_container: &str, fork_container: &str) -> Result<u16> {
    let snap_dir = snapshot_dir(base_container);
    let snap_path = snap_dir.join("snapshot.bin");
    let mem_path = snap_dir.join("memory.bin");
    if !snap_path.exists() || !mem_path.exists() {
        anyhow::bail!("no snapshot for base container {}", base_container);
    }

    let base_rootfs = rootfs_path(base_container);
    if !base_rootfs.exists() {
        anyhow::bail!("no rootfs for base container {}", base_container);
    }

    let fork_rootfs = rootfs_path(fork_container);
    let fork_port = derive_host_port(fork_container);
    let base_port = derive_host_port(base_container);
    let base_tap = TapDevice::derive(base_container);
    let netns = ForkNetns::derive(fork_container);
    let socket = socket_path(fork_container);

    // Sparse-copy isolates the fork's writes from the base rootfs.
    let status = tokio::process::Command::new("cp")
        .args(["--sparse=always", base_rootfs.to_str().unwrap(), fork_rootfs.to_str().unwrap()])
        .status()
        .await
        .context("cp --sparse=always failed")?;
    if !status.success() {
        anyhow::bail!("rootfs copy failed for fork {}", fork_container);
    }

    // From this point forward any error must clean up the sparse-copy rootfs
    // (and any partially-created netns/Firecracker process) to avoid orphaned
    // resources.  We use a macro so cleanup runs on all error branches without
    // duplicating the logic.
    macro_rules! cleanup_and_bail {
        ($ctx:expr, $err:expr) => {{
            let err = $err;
            tracing::warn!("restore_fork: cleaning up {} after error: {}", fork_container, err);
            kill_firecracker_process(fork_container).await;
            netns.destroy().await;
            let _ = tokio::fs::remove_file(&fork_rootfs).await;
            return Err(anyhow::anyhow!("{}: {}", $ctx, err));
        }};
    }

    // Tear down any stale fork from a previous attempt.
    kill_firecracker_process(fork_container).await;
    netns.destroy().await;

    // Create an isolated network namespace containing a TAP with the same name
    // that the snapshot expects.  This avoids the EBUSY conflict with the live
    // base VM's TAP.
    if let Err(e) = netns.create(&base_tap).await {
        cleanup_and_bail!("fork netns setup failed", e);
    }

    // Start Firecracker inside the fork's network namespace so it opens the
    // TAP from inside the namespace (no conflict with the base VM's TAP).
    if let Err(e) = start_fork_firecracker_daemon(fork_container, &socket, &netns.ns_name).await {
        cleanup_and_bail!("fork firecracker daemon failed", e);
    }

    let load_result = timeout(
        Duration::from_secs(RESTORE_TIMEOUT_SECS),
        api_put(&socket, "/snapshot/load", &json!({
            "snapshot_path": snap_path.to_str().unwrap(),
            "mem_backend": {
                "backend_type": "File",
                "backend_path": mem_path.to_str().unwrap(),
            },
            "enable_diff_snapshots": false,
            "resume_vm": true,
        })),
    )
    .await;
    match load_result {
        Err(_) => cleanup_and_bail!("fork restore timed out", format!("after {}s", RESTORE_TIMEOUT_SECS)),
        Ok(Err(e)) => cleanup_and_bail!("fork snapshot/load API failed", e),
        Ok(Ok(_)) => {}
    }

    // Set up host-side DNAT + policy routing so 127.0.0.1:fork_port → fork VM.
    if let Err(e) = netns.add_port_forward(fork_port, base_tap.vm_ip, base_port).await {
        cleanup_and_bail!("fork port-forward failed", e);
    }

    if let Err(e) = wait_for_port(fork_port, RESTORE_TIMEOUT_SECS).await {
        cleanup_and_bail!(
            format!("fork VM {} did not respond on port {}", fork_container, fork_port),
            e
        );
    }

    tracing::info!("firecracker: fork {} started on port {}", fork_container, fork_port);
    Ok(fork_port)
}

/// Kill a fork VM and clean up its network namespace + rootfs copy.
pub async fn kill_fork(fork_container: &str, base_container: &str) {
    let fork_port = derive_host_port(fork_container);
    let base_port = derive_host_port(base_container);
    let base_tap = TapDevice::derive(base_container);
    let netns = ForkNetns::derive(fork_container);
    netns.remove_port_forward(fork_port, base_tap.vm_ip, base_port).await;
    kill(fork_container).await.ok();
    netns.destroy().await;
    let _ = tokio::fs::remove_file(rootfs_path(fork_container)).await;
    tracing::info!("firecracker: fork {} killed and cleaned up", fork_container);
}

/// Kill a running VM and clean up TAP / iptables. Rootfs and snapshot are kept.
pub async fn kill(container_name: &str) -> Result<()> {
    let host_port = derive_host_port(container_name);
    kill_firecracker_process(container_name).await;
    TapDevice::derive(container_name).destroy(host_port).await.ok();
    Ok(())
}

/// Permanently delete all state for a container (rootfs + snapshot).
pub async fn destroy(container_name: &str) -> Result<()> {
    kill(container_name).await.ok();
    let _ = tokio::fs::remove_file(rootfs_path(container_name)).await;
    let _ = tokio::fs::remove_file(marker_path(container_name)).await;
    let _ = tokio::fs::remove_dir_all(snapshot_dir(container_name)).await;
    Ok(())
}

// ============================================================================
// rootfs extraction
// ============================================================================

/// Extract an OCI image into an ext4 disk image with an injected /init script.
///
/// The cache is keyed on (image_tag, env_fingerprint): if either changes
/// (new code push OR updated secrets) the rootfs is rebuilt.
///
/// The /init script (PID 1 inside the VM):
///   1. Mounts /proc, /sys, /dev
///   2. Exports image-level ENV vars, then runtime env overrides (PORT, secrets)
///   3. `cd` to the image's WorkingDir
///   4. `exec` the image's CMD/Entrypoint
///
/// PORT must be set in `env` to `derive_host_port(container_name)` so the
/// in-VM server listens on the port that the iptables DNAT rule forwards to.
async fn extract_rootfs(
    container_name: &str,
    image: &str,
    env: &[(String, String)],
    rootfs_path: &Path,
) -> Result<()> {
    // Cache: skip if rootfs was built for the same image + same runtime env.
    // This re-extracts when secrets change even if the code hasn't changed.
    let marker = marker_path(container_name);
    let wanted_marker = format!("{}\n{}", image, env_fingerprint(env));
    if rootfs_path.exists() {
        if let Ok(current) = tokio::fs::read_to_string(&marker).await {
            if current.trim() == wanted_marker.trim() {
                tracing::info!("firecracker: rootfs up to date for {}, skipping extraction", container_name);
                return Ok(());
            }
        }
    }

    let tar_path = format!("/tmp/fc-rootfs-{}.tar", container_name);
    let img_mnt = format!("/tmp/fc-img-mnt-{}", container_name);
    let _ = tokio::fs::remove_file(&tar_path).await;

    // Get image metadata via nerdctl image inspect (no throwaway container needed)
    let inspect_out = Command::new("nerdctl")
        .args(["--address", "/run/containerd/containerd.sock", "image", "inspect", image])
        .output().await
        .context("nerdctl image inspect failed")?;
    if !inspect_out.status.success() {
        anyhow::bail!("nerdctl image inspect failed: {}", String::from_utf8_lossy(&inspect_out.stderr).trim());
    }
    let info: serde_json::Value = serde_json::from_slice(&inspect_out.stdout)
        .context("failed to parse nerdctl image inspect output")?;
    let cfg = &info[0]["Config"];

    let entrypoint: Vec<String> = serde_json::from_value(cfg["Entrypoint"].clone()).unwrap_or_default();
    let cmd: Vec<String> = serde_json::from_value(cfg["Cmd"].clone()).unwrap_or_default();
    let full_cmd: Vec<String> = entrypoint.into_iter().chain(cmd).collect();
    if full_cmd.is_empty() {
        anyhow::bail!("image {} has no CMD or Entrypoint", image);
    }

    let workdir = cfg["WorkingDir"].as_str().unwrap_or("/").to_string();
    let image_env: Vec<String> = serde_json::from_value(cfg["Env"].clone()).unwrap_or_default();

    // Mount the image filesystem via ctr (nerdctl v2 removed `nerdctl export`)
    let ctr_ref = ctr_image_ref(image);
    tokio::fs::create_dir_all(&img_mnt).await?;
    let mount_st = Command::new("ctr")
        .args(["-n", "default", "images", "mount", &ctr_ref, &img_mnt])
        .status().await
        .context("ctr images mount failed")?;
    if !mount_st.success() {
        let _ = tokio::fs::remove_dir(&img_mnt).await;
        anyhow::bail!("ctr images mount failed for {}", ctr_ref);
    }

    // Create tar from the mounted image filesystem, then unmount
    let tar_st = Command::new("tar")
        .args(["cf", &tar_path, "-C", &img_mnt, "."])
        .status().await;
    Command::new("umount").arg(&img_mnt).status().await.ok();
    let _ = tokio::fs::remove_dir(&img_mnt).await;
    let tar_st = tar_st.context("tar cf (image snapshot) failed")?;
    if !tar_st.success() {
        anyhow::bail!("tar of image filesystem failed (exit {:?})", tar_st.code());
    }

    // Size: 1.5× tar + 64 MB overhead, min 512 MB, max 8 GB
    let tar_mb = tokio::fs::metadata(&tar_path).await
        .context("cannot stat image tar")?.len() / (1024 * 1024);
    let ext4_mb = ((tar_mb * 3 / 2) + 64).clamp(512, 8192);
    tracing::info!("firecracker: tar={}MB, ext4={}MB for {}", tar_mb, ext4_mb, container_name);

    // Create fresh ext4 image
    let _ = tokio::fs::remove_file(rootfs_path).await;
    let st = Command::new("dd")
        .args(["if=/dev/zero", &format!("of={}", rootfs_path.display()), "bs=1M", &format!("count={}", ext4_mb)])
        .status().await.context("dd failed")?;
    if !st.success() { anyhow::bail!("dd failed"); }

    let st = Command::new("mkfs.ext4")
        .args(["-F", "-q", rootfs_path.to_str().unwrap()])
        .status().await.context("mkfs.ext4 failed")?;
    if !st.success() { anyhow::bail!("mkfs.ext4 failed"); }

    // Mount, populate, inject /init — always umount before returning
    let mnt = format!("/tmp/fc-mnt-{}", container_name);
    tokio::fs::create_dir_all(&mnt).await?;
    let st = Command::new("mount")
        .args([rootfs_path.to_str().unwrap(), &mnt])
        .status().await.context("mount failed")?;
    if !st.success() {
        let _ = tokio::fs::remove_dir(&mnt).await;
        anyhow::bail!("mount failed");
    }

    let populate_result = populate_rootfs(&mnt, &tar_path, &image_env, env, &workdir, &full_cmd).await;

    // Always unmount regardless of success/failure
    Command::new("umount").arg(&mnt).status().await.ok();
    let _ = tokio::fs::remove_dir(&mnt).await;

    populate_result?;

    // Record cache key so we can skip re-extraction on next boot with same state
    tokio::fs::write(&marker, &wanted_marker).await.ok();

    tracing::info!("firecracker: rootfs ready for {} ({}MB)", container_name, ext4_mb);
    Ok(())
}

/// Extract the tar into `mnt` and write the /init script.
/// Separated from `extract_rootfs` so cleanup (umount) can always run.
async fn populate_rootfs(
    mnt: &str,
    tar_path: &str,
    image_env: &[String],
    runtime_env: &[(String, String)],
    workdir: &str,
    cmd: &[String],
) -> Result<()> {
    let tar_status = Command::new("tar")
        .args(["-xf", tar_path, "-C", mnt, "--ignore-failed-read"])
        .status().await
        .context("tar extraction failed (spawn error)")?;
    let _ = tokio::fs::remove_file(tar_path).await;

    // tar exit code 1 = warnings (often device nodes; tolerated with --ignore-failed-read)
    // tar exit code 2 = fatal error
    if tar_status.code() == Some(2) {
        anyhow::bail!("tar extraction failed fatally (exit code 2)");
    }

    let init_script = build_init_script(image_env, runtime_env, workdir, cmd);
    tokio::fs::write(format!("{}/init", mnt), &init_script).await
        .context("failed to write /init into rootfs")?;
    Command::new("chmod").args(["755", &format!("{}/init", mnt)])
        .status().await.ok();

    Ok(())
}

/// Build a minimal PID-1 shell script.
///
/// Execution order:
///   1. Mount essential filesystems (proc/sys/dev/devpts)
///   2. Export image-level ENV defaults
///   3. Export runtime overrides (PORT must match the DNAT rule, plus user secrets)
///   4. cd WorkingDir
///   5. exec CMD
fn build_init_script(
    image_env: &[String],
    runtime_env: &[(String, String)],
    workdir: &str,
    cmd: &[String],
) -> String {
    let mut lines: Vec<String> = vec![
        "#!/bin/sh".into(),
        "mount -t proc proc /proc 2>/dev/null || true".into(),
        "mount -t sysfs sysfs /sys 2>/dev/null || true".into(),
        "mount -t devtmpfs devtmpfs /dev 2>/dev/null || true".into(),
        "mkdir -p /dev/pts && mount -t devpts devpts /dev/pts 2>/dev/null || true".into(),
    ];

    // Image-level defaults (PATH, NODE_VERSION, PYTHON_VERSION, etc.)
    for pair in image_env {
        if let Some((k, v)) = pair.split_once('=') {
            // Validate key: POSIX variable name ([A-Za-z_][A-Za-z0-9_]*).
            // A crafted OCI image could inject shell metacharacters via the key;
            // the value is already protected by shell_quote.
            let valid_key = !k.is_empty()
                && k.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if valid_key {
                lines.push(format!("export {}={}", k, shell_quote(v)));
            } else {
                tracing::warn!("build_init_script: skipping OCI image env with invalid key: {:?}", k);
            }
        }
    }

    // Runtime overrides: these win over image defaults for any duplicate key.
    // PORT must equal derive_host_port so the DNAT mapping is consistent.
    for (k, v) in runtime_env {
        let valid_key = !k.is_empty()
            && k.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if valid_key {
            lines.push(format!("export {}={}", k, shell_quote(v)));
        } else {
            tracing::warn!("build_init_script: skipping runtime env with invalid key: {:?}", k);
        }
    }

    let wd = if workdir.is_empty() { "/" } else { workdir };
    if wd != "/" {
        lines.push(format!("cd {}", shell_quote(wd)));
    }

    // Warm the page cache in the background before exec so the first tool call
    // doesn't pay the cold-read penalty. Scan $HOME and the workdir — covers
    // virtually all MCP servers (templates, DBs, etc. live under the user home).
    // -xdev avoids crossing into /proc /sys /dev mounts.
    lines.push(
        r#"{ find "${HOME:-/root}" "$(pwd)" -xdev -type f 2>/dev/null | xargs cat > /dev/null 2>&1; } &"#.into()
    );


    let exec_args = cmd.iter().map(|s| shell_quote(s)).collect::<Vec<_>>().join(" ");
    // Redirect exec stderr to stdout so errors appear in the Firecracker console log
    lines.push(format!("exec {} 2>&1", exec_args));

    lines.join("\n") + "\n"
}

/// POSIX single-quote a shell argument, escaping embedded single quotes.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A deterministic fingerprint of the runtime env vars (keys + values).
/// Changes when any secret is added, removed, or updated.
/// Uses FNV-1a over sorted key=value lines for stable, fast hashing.
fn env_fingerprint(env: &[(String, String)]) -> String {
    // Sort for stability (env order shouldn't affect the fingerprint)
    let mut pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    pairs.sort_unstable_by_key(|(k, _)| *k);

    let hash = pairs.iter().fold(2_166_136_261u32, |h, (k, v)| {
        let h = k.bytes().fold(h, |h, b| (h ^ b as u32).wrapping_mul(16_777_619));
        let h = (h ^ b'=' as u32).wrapping_mul(16_777_619);
        let h = v.bytes().fold(h, |h, b| (h ^ b as u32).wrapping_mul(16_777_619));
        (h ^ b'\n' as u32).wrapping_mul(16_777_619)
    });
    format!("{:08x}", hash)
}

// ============================================================================
// Firecracker process + API
// ============================================================================

async fn start_firecracker_daemon(container_name: &str, socket: &Path) -> Result<()> {
    let _ = tokio::fs::remove_file(socket).await;
    let log_path = format!("/tmp/fc-{}.log", container_name);
    let log_file = std::fs::File::create(&log_path)
        .with_context(|| format!("create firecracker log file {}", log_path))?;
    let log_file2 = log_file.try_clone().context("clone log file handle")?;
    tracing::info!("firecracker: daemon log → {}", log_path);
    Command::new("firecracker")
        .args(["--api-sock", socket.to_str().unwrap(), "--id", container_name])
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(log_file2))
        .spawn()
        .context("failed to spawn firecracker")?;

    // Wait up to 5 s for the API socket to appear
    for _ in 0..50 {
        if socket.exists() { return Ok(()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("firecracker API socket did not appear within 5s")
}

/// Start a Firecracker process inside a network namespace so the snapshot's
/// expected TAP device can be opened without conflicting with the base VM.
async fn start_fork_firecracker_daemon(
    container_name: &str,
    socket: &Path,
    ns_name: &str,
) -> Result<()> {
    let _ = tokio::fs::remove_file(socket).await;
    let log_path = format!("/tmp/fc-{}.log", container_name);
    let log_file = std::fs::File::create(&log_path)
        .with_context(|| format!("create fork firecracker log file {}", log_path))?;
    let log_file2 = log_file.try_clone().context("clone log file handle")?;
    tracing::info!("firecracker: fork daemon log → {}", log_path);
    // Run `ip netns exec NS firecracker --api-sock SOCKET --id CONTAINER`
    Command::new("ip")
        .args([
            "netns", "exec", ns_name,
            "firecracker",
            "--api-sock", socket.to_str().unwrap(),
            "--id", container_name,
        ])
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(log_file2))
        .spawn()
        .context("failed to spawn fork firecracker")?;

    // Wait up to 5 s for the API socket (created on the host filesystem)
    for _ in 0..50 {
        if socket.exists() { return Ok(()); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("fork firecracker API socket did not appear within 5s")
}


async fn configure_and_start(
    socket: &Path,
    rootfs: &Path,
    tap: &TapDevice,
    memory_mb: u64,
) -> Result<()> {
    // init=/init overrides PID 1. The script mounts /proc|/sys|/dev, exports all
    // env vars (including PORT and user secrets), and exec's the Docker CMD.
    let boot_args = format!(
        "console=ttyS0 reboot=k panic=1 pci=off init=/init \
         ip={vm_ip}::{gw}:255.255.255.252::eth0:off",
        vm_ip = tap.vm_ip,
        gw = tap.host_ip,
    );

    api_put(socket, "/boot-source", &json!({
        "kernel_image_path": KERNEL_PATH,
        "boot_args": boot_args,
    })).await.context("PUT /boot-source")?;

    api_put(socket, "/drives/rootfs", &json!({
        "drive_id": "rootfs",
        "path_on_host": rootfs.to_str().unwrap(),
        "is_root_device": true,
        "is_read_only": false,
    })).await.context("PUT /drives/rootfs")?;

    let vcpu_count = if memory_mb >= 1024 { 4u64 } else if memory_mb >= 512 { 2 } else { 1 };
    api_put(socket, "/machine-config", &json!({
        "vcpu_count": vcpu_count,
        "mem_size_mib": memory_mb,
    })).await.context("PUT /machine-config")?;

    api_put(socket, "/network-interfaces/eth0", &json!({
        "iface_id": "eth0",
        "guest_mac": tap.vm_mac(),
        "host_dev_name": tap.name,
    })).await.context("PUT /network-interfaces/eth0")?;

    api_put(socket, "/actions", &json!({ "action_type": "InstanceStart" }))
        .await.context("PUT /actions InstanceStart")?;

    Ok(())
}

/// HTTP PUT over the Firecracker Unix domain socket — no child process required.
///
/// Content-Length is the byte length of the JSON body, which is correct for
/// HTTP (Content-Length is measured in bytes, not characters).
async fn api_put(socket: &Path, path: &str, body: &serde_json::Value) -> Result<()> {
    api_request(socket, "PUT", path, body).await
}

/// HTTP PATCH over the Firecracker Unix domain socket.
/// Used for VM state changes (Paused/Resumed) which require PATCH /vm.
async fn api_patch(socket: &Path, path: &str, body: &serde_json::Value) -> Result<()> {
    api_request(socket, "PATCH", path, body).await
}

async fn api_request(socket: &Path, method: &str, path: &str, body: &serde_json::Value) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let body_str = body.to_string();
    let request = format!(
        "{} {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        method,
        path,
        body_str.len(),
        body_str,
    );

    let stream = UnixStream::connect(socket).await
        .with_context(|| format!("connect to Firecracker socket {:?}", socket))?;
    let (reader, mut writer) = tokio::io::split(stream);
    writer.write_all(request.as_bytes()).await
        .context("write to Firecracker API socket")?;

    // Firecracker ignores Connection: close and keeps the socket open, so
    // read_to_end() blocks forever. Instead, read only the HTTP headers:
    // the status line tells us success/failure, and the body (if any) is
    // the error JSON which follows. We read until the blank line (\r\n\r\n)
    // then stop — no EOF needed.
    let mut reader = BufReader::new(reader);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).await
        .context("read status line from Firecracker API socket")?;
    // Drain headers until blank line
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.ok();
        if line == "\r\n" || line.is_empty() { break; }
    }

    let status_line = status_line.trim();
    if !status_line.contains("HTTP/1.1 2") && !status_line.contains("HTTP/1.0 2") {
        // Read the error body (Firecracker sends a JSON error description).
        // Use a short timeout + size cap: the socket stays open (Firecracker ignores
        // Connection: close), so read_to_end() would block forever.
        let mut body_buf = [0u8; 512];
        let n = timeout(Duration::from_millis(200), reader.read(&mut body_buf))
            .await.unwrap_or(Ok(0)).unwrap_or(0);
        let body = std::str::from_utf8(&body_buf[..n]).unwrap_or("").trim();
        if body.is_empty() {
            anyhow::bail!("Firecracker API {} {} → {}", method, path, status_line);
        }
        anyhow::bail!("Firecracker API {} {} → {} | {}", method, path, status_line, body);
    }
    Ok(())
}

async fn kill_firecracker_process(container_name: &str) {
    // Use "firecracker.*--id {name}( |$)" (no wildcard between --id and the name)
    // so we only match the exact container and not any name that has container_name
    // as a prefix (e.g. "mcp-abc" must not kill "mcp-abc-fork1").
    Command::new("pkill")
        .args(["-f", &format!("firecracker.*--id {}( |$)", container_name)])
        .output().await.ok();
    let _ = tokio::fs::remove_file(socket_path(container_name)).await;
}

async fn wait_for_port(port: u16, timeout_secs: u64) -> Result<()> {
    let addr = format!("127.0.0.1:{}", port);
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        // Cap each connect attempt at 2s: without this, a dropped SYN (e.g. if the
        // DNAT/SNAT rules aren't set up) triggers TCP retransmit timeouts that can
        // block a single attempt for up to 63 s, consuming the entire boot window.
        let connected = timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(&addr),
        ).await.map(|r| r.is_ok()).unwrap_or(false);

        if connected {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("port {} not reachable after {}s", port, timeout_secs);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

// ============================================================================
// Container inspect helper
// ============================================================================

/// Convert a short image tag to the canonical containerd reference used by ctr.
/// e.g. "mcp-xxx:HEAD" → "docker.io/library/mcp-xxx:HEAD"
///      "ghcr.io/org/repo:tag" → unchanged
fn ctr_image_ref(image: &str) -> String {
    if image.contains('/') {
        image.to_string()
    } else {
        format!("docker.io/library/{}", image)
    }
}

// ============================================================================
// VM introspection (used by the internal HTTP API in main.rs)
// ============================================================================

/// Return the PID of the running Firecracker process for a container, if any.
pub async fn find_vm_pid(container_name: &str) -> Option<u32> {
    let out = Command::new("pgrep")
        .args(["-f", &format!("firecracker.*--id {}( |$)", container_name)])
        .output().await.ok()?;
    if !out.status.success() { return None; }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Return the RSS memory (in bytes) of the Firecracker process for a container.
pub async fn vm_rss_bytes(container_name: &str) -> Option<u64> {
    let pid = find_vm_pid(container_name).await?;
    let status = tokio::fs::read_to_string(format!("/proc/{}/status", pid)).await.ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            // VmRSS:   123456 kB
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// List container names of all currently running Firecracker VMs
/// by enumerating active socket files under SOCKET_DIR.
pub async fn list_running_vms() -> Vec<String> {
    let mut result = Vec::new();
    let mut dir = match tokio::fs::read_dir(SOCKET_DIR).await {
        Ok(d) => d,
        Err(_) => return result,
    };
    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if let Some(stem) = s.strip_suffix(".socket") {
            // Verify the firecracker process is actually alive
            if find_vm_pid(stem).await.is_some() {
                result.push(stem.to_string());
            }
        }
    }
    result
}

// ============================================================================
// Path helpers
// ============================================================================

fn rootfs_path(container_name: &str) -> PathBuf {
    PathBuf::from(ROOTFS_DIR).join(format!("{}.ext4", container_name))
}

/// Marker file stores "{image_tag}\n{env_fingerprint}" for cache invalidation.
fn marker_path(container_name: &str) -> PathBuf {
    PathBuf::from(ROOTFS_DIR).join(format!("{}.marker", container_name))
}

fn socket_path(container_name: &str) -> PathBuf {
    PathBuf::from(SOCKET_DIR).join(format!("{}.socket", container_name))
}

/// Returns true if a Firecracker VM is actually alive for the given container.
/// Checks by attempting a real TCP connection to the Unix socket — a stale socket
/// file left behind after a crash would pass an existence check but fail here.
pub fn vm_socket_exists(container_name: &str) -> bool {
    let path = socket_path(container_name);
    if !path.exists() {
        return false;
    }
    // Attempt a synchronous connect to distinguish a live VM from a stale socket.
    std::os::unix::net::UnixStream::connect(&path).is_ok()
}

fn snapshot_dir(container_name: &str) -> PathBuf {
    PathBuf::from(SNAPSHOT_DIR).join(container_name)
}

/// Remove Firecracker artefacts that belong to containers not in `known_containers`.
///
/// Cleans up three directories:
/// - `SNAPSHOT_DIR/` — per-container subdirectories
/// - `ROOTFS_DIR/`   — `<name>.ext4` and `<name>.marker` files
/// - `SOCKET_DIR/`   — `<name>.socket` files whose process is no longer alive
pub async fn cleanup_stale_resources(known_containers: &std::collections::HashSet<String>) {
    // Snapshot directories
    if let Ok(mut dir) = tokio::fs::read_dir(SNAPSHOT_DIR).await {
        while let Ok(Some(entry)) = dir.next_entry().await {
            if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                if !known_containers.contains(&name) {
                    tracing::info!(
                        "firecracker cleanup: removing orphan snapshot dir {}",
                        name
                    );
                    let _ = tokio::fs::remove_dir_all(entry.path()).await;
                }
            }
        }
    }

    // Rootfs and marker files.
    //
    // Fork rootfs files have the pattern `<base>-f<nanos>.ext4` — no DB entry,
    // so they would normally be deleted as "orphan" files.  But we must NOT
    // remove a fork rootfs while the fork VM is still running.
    // A fork VM is alive iff its socket file responds to a connect.
    if let Ok(mut dir) = tokio::fs::read_dir(ROOTFS_DIR).await {
        while let Ok(Some(entry)) = dir.next_entry().await {
            let fname = entry.file_name().to_string_lossy().to_string();
            let container = fname
                .strip_suffix(".ext4")
                .or_else(|| fname.strip_suffix(".marker"));
            if let Some(c) = container {
                if known_containers.contains(c) {
                    continue; // base container — keep
                }
                // Detect fork names: ends with "-f" followed by one or more digits.
                // e.g.  "my-server-f1234567890"
                let is_fork = c.rfind("-f")
                    .map(|pos| c[pos + 2..].chars().all(|ch| ch.is_ascii_digit()) && pos + 2 < c.len())
                    .unwrap_or(false);

                if is_fork && vm_socket_exists(c) {
                    tracing::debug!(
                        "firecracker cleanup: skipping live fork rootfs {}",
                        fname
                    );
                    continue;
                }
                if is_fork {
                    tracing::info!("firecracker cleanup: removing dead fork rootfs {}", fname);
                } else {
                    tracing::info!("firecracker cleanup: removing orphan rootfs file {}", fname);
                }
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }

    // Stale socket files (socket exists but no live process)
    if let Ok(mut dir) = tokio::fs::read_dir(SOCKET_DIR).await {
        while let Ok(Some(entry)) = dir.next_entry().await {
            let fname = entry.file_name().to_string_lossy().to_string();
            if let Some(c) = fname.strip_suffix(".socket") {
                if !vm_socket_exists(c) {
                    tracing::info!(
                        "firecracker cleanup: removing stale socket for {}",
                        c
                    );
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
            }
        }
    }
}

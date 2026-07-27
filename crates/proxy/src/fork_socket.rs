/// Unix socket client for fork VM lifecycle management and base VM active leases.
///
/// Two lease types:
///   `ForkLease` — holds a fork VM alive (builder kills it on connection drop)
///   `VmLease`   — holds an active-request count on the base VM (builder skips
///                 scale-to-zero snapshot while count > 0)
///
/// Both are backed by a Unix socket connection.  Dropping sends SHUT_WR; the
/// builder detects EOF atomically — even on proxy process crash the kernel
/// closes all file descriptors and the builder is notified.
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

fn is_valid_container_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

// ──────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ──────────────────────────────────────────────────────────────────────────────

async fn connect_and_send(
    socket_path: &str,
    req: &str,
) -> anyhow::Result<(tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf)> {
    let stream = UnixStream::connect(socket_path).await.map_err(|e| {
        anyhow::anyhow!("fork socket: connect {} failed: {}", socket_path, e)
    })?;
    let (read_half, mut write_half) = stream.into_split();
    write_half.write_all(req.as_bytes()).await?;
    Ok((read_half, write_half))
}

async fn read_response(
    read_half: tokio::net::unix::OwnedReadHalf,
) -> anyhow::Result<serde_json::Value> {
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    if line.is_empty() {
        anyhow::bail!("fork socket: builder closed connection without a response");
    }
    let resp: serde_json::Value = serde_json::from_str(line.trim())
        .map_err(|e| anyhow::anyhow!("fork socket: invalid response JSON: {}", e))?;
    if let Some(err) = resp["error"].as_str() {
        anyhow::bail!("fork socket: {}", err);
    }
    Ok(resp)
}

// ──────────────────────────────────────────────────────────────────────────────
// ForkLease — keeps a fork VM alive
// ──────────────────────────────────────────────────────────────────────────────

/// A live fork VM lease.
///
/// The builder keeps the fork VM alive as long as the write half of this
/// connection is open.  Dropping sends SHUT_WR → builder EOF → kill_fork.
/// Guaranteed to fire even on proxy crash (kernel closes all fds on exit).
pub struct ForkLease {
    _conn: tokio::net::unix::OwnedWriteHalf,
    pub port: u16,
    pub fork_name: String,
    pub base_name: String,
}

/// Connect and request a fork VM for `base`.
///
/// Returns a `ForkLease` that keeps the fork alive until dropped.
/// Returns `Err` on any failure so the caller can fall back to the base VM.
pub async fn request_fork(socket_path: &str, base: &str) -> anyhow::Result<ForkLease> {
    if !is_valid_container_name(base) {
        anyhow::bail!("fork socket: invalid container name: {:?}", base);
    }
    let req = format!("{{\"base\":\"{}\"}}\n", base);
    let (read_half, write_half) = connect_and_send(socket_path, &req).await?;
    let resp = read_response(read_half).await?;

    let port = resp["port"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("fork socket: missing 'port' in response"))? as u16;
    let fork_name = resp["fork"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("fork socket: missing 'fork' in response"))?
        .to_string();

    Ok(ForkLease {
        _conn: write_half,
        port,
        fork_name,
        base_name: base.to_string(),
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// VmLease — prevents scale-to-zero from snapshotting an active base VM
// ──────────────────────────────────────────────────────────────────────────────

/// An active-request lease on a base VM.
///
/// The builder increments its in-flight counter for this container while the
/// connection is open, and skips the scale-to-zero snapshot when count > 0.
/// Dropping sends SHUT_WR → builder decrements the count.
///
/// This prevents two failure modes:
///   1. Scale-to-zero snapshots a VM while it is actively serving a request.
///   2. Scale-to-zero overwrites snapshot files while a fork is being restored
///      from them — the first request's VmLease protects the base VM's snapshot
///      files for the full duration of its own request.
///
/// Even on proxy crash the OS closes all fds, the builder decrements, and the
/// VM becomes eligible for scale-to-zero again.
pub struct VmLease {
    _conn: tokio::net::unix::OwnedWriteHalf,
}

/// Acquire an active-request lease on `container`.
///
/// Returns `Err` on failure; callers should log and continue without a lease
/// (scale-to-zero protection is best-effort when the socket is unavailable).
pub async fn hold_vm(socket_path: &str, container: &str) -> anyhow::Result<VmLease> {
    if !is_valid_container_name(container) {
        anyhow::bail!("fork socket: invalid container name: {:?}", container);
    }
    let req = format!("{{\"hold\":\"{}\"}}\n", container);
    let (read_half, write_half) = connect_and_send(socket_path, &req).await?;
    // Builder responds {"ok":true} once the lease is registered.
    let _ = read_response(read_half).await?;
    Ok(VmLease { _conn: write_half })
}

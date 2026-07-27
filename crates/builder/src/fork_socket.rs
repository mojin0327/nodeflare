/// Unix socket server for fork VM lifecycle management and base VM active leases.
///
/// The server handles two request types over the same socket:
///
///   {"base":"<container>"}  — spawn a fork VM; keep it alive until EOF
///   {"hold":"<container>"}  — increment the active-request counter for a base
///                             VM; decrement when the connection is dropped
///
/// Connection lifetime = resource lifetime.  When the proxy drops a connection
/// (normally or on crash), the kernel sends SHUT_WR, which the server detects
/// as EOF and uses as the cleanup signal.  No HTTP call needed, no orphan risk.
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

static FORK_SEQ: AtomicU64 = AtomicU64::new(0);

/// Container names must be non-empty alphanumeric+hyphen strings ≤ 64 chars.
/// This prevents JSON injection in socket requests and bounds activeMap memory.
fn is_valid_container_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Shared counter: number of active proxy connections per base container.
/// The scale-to-zero scaler checks this before snapshotting.
pub type ActiveMap = Arc<Mutex<HashMap<String, usize>>>;

pub fn new_active_map() -> ActiveMap {
    Arc::new(Mutex::new(HashMap::new()))
}

pub async fn run(socket_path: &str, active: ActiveMap) {
    // Remove the stale socket file left by the previous process run.
    let _ = tokio::fs::remove_file(socket_path).await;

    if let Some(parent) = Path::new(socket_path).parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            tracing::error!("fork socket: cannot create parent dir {}: {}", parent.display(), e);
            return;
        }
    }

    let listener = match UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("fork socket: bind {} failed: {}", socket_path, e);
            return;
        }
    };

    // Restrict to owner-only so other processes on the host cannot connect.
    if let Err(e) = std::fs::set_permissions(
        socket_path,
        std::fs::Permissions::from_mode(0o600),
    ) {
        tracing::warn!("fork socket: failed to set permissions on {}: {}", socket_path, e);
    }

    tracing::info!("fork socket: listening on {}", socket_path);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let active = active.clone();
                tokio::spawn(handle_connection(stream, active));
            }
            Err(e) => {
                tracing::error!("fork socket: accept error: {}", e);
            }
        }
    }
}

async fn handle_connection(stream: tokio::net::UnixStream, active: ActiveMap) {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    // Read the single-line JSON request from the proxy.
    let mut line = String::new();
    match reader.read_line(&mut line).await {
        Ok(0) => return, // proxy disconnected before sending anything
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("fork socket: read error: {}", e);
            return;
        }
    }

    let req: serde_json::Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(e) => {
            let _ = write_half
                .write_all(format!("{{\"error\":\"invalid json: {}\"}}\n", e).as_bytes())
                .await;
            return;
        }
    };

    if let Some(container) = req["hold"].as_str() {
        if !is_valid_container_name(container) {
            let _ = write_half.write_all(b"{\"error\":\"invalid container name\"}\n").await;
            return;
        }
        // ── "hold" request: base VM active-request lease ──────────────────────
        handle_hold(container, reader, write_half, active).await;
    } else if let Some(base) = req["base"].as_str() {
        if !is_valid_container_name(base) {
            let _ = write_half.write_all(b"{\"error\":\"invalid container name\"}\n").await;
            return;
        }
        // ── "base" request: fork VM lifecycle ─────────────────────────────────
        handle_fork(base, reader, write_half, active).await;
    } else {
        let _ = write_half
            .write_all(b"{\"error\":\"request must contain 'base' or 'hold' field\"}\n")
            .await;
    }
}

/// Handle a "hold" request: increment active count, respond OK, wait for EOF,
/// then decrement.  The scale-to-zero scaler skips containers with count > 0.
async fn handle_hold(
    container: &str,
    mut reader: BufReader<tokio::io::ReadHalf<tokio::net::UnixStream>>,
    mut write_half: tokio::io::WriteHalf<tokio::net::UnixStream>,
    active: ActiveMap,
) {
    // Increment before responding so the scaler can't race between our ack
    // and the proxy's first request reaching the VM.
    {
        let mut map = active.lock().unwrap_or_else(|p| p.into_inner());
        *map.entry(container.to_string()).or_insert(0) += 1;
    }
    tracing::debug!("fork socket: hold lease acquired for {}", container);

    if let Err(e) = write_half.write_all(b"{\"ok\":true}\n").await {
        tracing::warn!("fork socket: hold ack failed for {}: {}", container, e);
        decrement_active(&active, container);
        return;
    }

    // Wait for EOF (proxy drops connection on request completion or crash).
    let mut buf = [0u8; 1];
    let _ = reader.read(&mut buf).await;

    decrement_active(&active, container);
    tracing::debug!("fork socket: hold lease released for {}", container);
}

/// Handle a "base" request: spawn a fork VM, respond with port, wait for EOF,
/// then kill the fork.
async fn handle_fork(
    base: &str,
    mut reader: BufReader<tokio::io::ReadHalf<tokio::net::UnixStream>>,
    mut write_half: tokio::io::WriteHalf<tokio::net::UnixStream>,
    active: ActiveMap,
) {
    // Derive a unique fork name: millisecond timestamp + atomic counter prevents
    // collisions under concurrent same-millisecond requests.
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let seq = FORK_SEQ.fetch_add(1, Ordering::Relaxed);
    let prefix = &base[..base.len().min(20)];
    let fork = format!("{}-f{}{:06}", prefix, ms, seq % 1_000_000);

    // Count this fork as an active request on the base container so the
    // scale-to-zero scaler doesn't overwrite the snapshot files mid-restore.
    {
        let mut map = active.lock().unwrap_or_else(|p| p.into_inner());
        *map.entry(base.to_string()).or_insert(0) += 1;
    }

    match crate::firecracker::restore_fork(base, &fork).await {
        Ok(port) => {
            let resp = format!("{{\"port\":{},\"fork\":\"{}\"}}\n", port, fork);
            if let Err(e) = write_half.write_all(resp.as_bytes()).await {
                // Proxy disconnected before receiving the port — clean up.
                tracing::warn!("fork socket: write failed for {}, cleaning up: {}", fork, e);
                decrement_active(&active, base);
                crate::firecracker::kill_fork(&fork, base).await;
                return;
            }
            tracing::info!("fork socket: {} ready on port {}, awaiting disconnect", fork, port);

            // Block until the proxy closes its side of the connection (EOF).
            let mut buf = [0u8; 1];
            let _ = reader.read(&mut buf).await;

            tracing::info!("fork socket: client disconnected, cleaning up {}", fork);
        }
        Err(e) => {
            tracing::warn!("fork socket: restore_fork {} failed: {}", base, e);
            decrement_active(&active, base);
            let msg = e.to_string().replace('"', "'");
            let _ = write_half
                .write_all(format!("{{\"error\":\"{}\"}}\n", msg).as_bytes())
                .await;
            return;
        }
    }

    decrement_active(&active, base);
    crate::firecracker::kill_fork(&fork, base).await;
}

fn decrement_active(active: &ActiveMap, container: &str) {
    let mut map = active.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(c) = map.get_mut(container) {
        *c = c.saturating_sub(1);
        if *c == 0 {
            map.remove(container);
        }
    }
}

/// Returns the number of active proxy connections for `container`.
/// Used by the scale-to-zero scaler to skip busy VMs.
pub fn active_count(active: &ActiveMap, container: &str) -> usize {
    active
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(container)
        .copied()
        .unwrap_or(0)
}

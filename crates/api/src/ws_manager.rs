use mcp_common::types::WsMessage;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, RwLock};

/// Default channel capacity for each subscription (configurable via WS_CHANNEL_CAPACITY env var)
fn default_channel_capacity() -> usize {
    std::env::var("WS_CHANNEL_CAPACITY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256)
}

/// Maximum connections per channel (configurable via WS_MAX_CONNECTIONS_PER_CHANNEL env var)
fn max_connections_per_channel() -> usize {
    std::env::var("WS_MAX_CONNECTIONS_PER_CHANNEL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000)
}

/// Cleanup interval in seconds (configurable via WS_CLEANUP_INTERVAL_SECS env var)
fn cleanup_interval_secs() -> u64 {
    std::env::var("WS_CLEANUP_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
}

/// Maximum total connections across all channels (configurable via WS_MAX_TOTAL_CONNECTIONS env var)
fn max_total_connections() -> usize {
    std::env::var("WS_MAX_TOTAL_CONNECTIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10000)
}

/// Maximum number of channels (configurable via WS_MAX_CHANNELS env var)
fn max_channels() -> usize {
    std::env::var("WS_MAX_CHANNELS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000)
}

/// Channel TTL in seconds - channels with no activity are removed (configurable via WS_CHANNEL_TTL_SECS env var)
fn channel_ttl_secs() -> u64 {
    std::env::var("WS_CHANNEL_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3600) // 1 hour default
}

/// Channel metadata for tracking creation time and last activity
/// Scalability: Using AtomicU64 for last_activity to avoid write locks on broadcast
#[derive(Debug)]
struct ChannelInfo {
    sender: broadcast::Sender<WsMessage>,
    /// Last activity timestamp as milliseconds since UNIX_EPOCH
    /// Using AtomicU64 to allow lock-free updates during broadcast
    last_activity_ms: AtomicU64,
}

impl ChannelInfo {
    fn new(sender: broadcast::Sender<WsMessage>) -> Self {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            sender,
            last_activity_ms: AtomicU64::new(now_ms),
        }
    }

    fn touch(&self) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.last_activity_ms.store(now_ms, Ordering::Relaxed);
    }

    fn last_activity_duration(&self) -> Duration {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let last_ms = self.last_activity_ms.load(Ordering::Relaxed);
        Duration::from_millis(now_ms.saturating_sub(last_ms))
    }
}

/// WebSocket connection manager for handling real-time updates
///
/// Uses Redis pub/sub internally for multi-instance support
#[derive(Debug, Clone)]
pub struct WsManager {
    /// Active channels with their broadcast senders and metadata
    channels: Arc<RwLock<HashMap<String, ChannelInfo>>>,
    /// Total connection count across all channels
    total_connections: Arc<AtomicUsize>,
    /// Channel capacity setting
    channel_capacity: usize,
    /// Max connections per channel
    max_per_channel: usize,
    /// Max total connections across all channels
    max_total: usize,
    /// Max number of channels
    max_channels: usize,
    /// Channel TTL for cleanup
    channel_ttl: Duration,
}

impl WsManager {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            total_connections: Arc::new(AtomicUsize::new(0)),
            channel_capacity: default_channel_capacity(),
            max_per_channel: max_connections_per_channel(),
            max_total: max_total_connections(),
            max_channels: max_channels(),
            channel_ttl: Duration::from_secs(channel_ttl_secs()),
        }
    }

    /// Start background cleanup task
    pub fn start_cleanup_task(self: Arc<Self>) {
        let interval = Duration::from_secs(cleanup_interval_secs());
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            loop {
                interval_timer.tick().await;
                self.cleanup_empty_channels().await;
                tracing::debug!(
                    "WsManager cleanup: {} channels, {} total connections",
                    self.channel_count().await,
                    self.total_connections.load(Ordering::Relaxed)
                );
            }
        });
    }

    /// Subscribe to a channel, creating it if it doesn't exist
    /// Returns error if channel or global connection limit is reached
    pub async fn subscribe(&self, channel: &str) -> Result<broadcast::Receiver<WsMessage>, SubscribeError> {
        // Check global connection limit first
        let current_total = self.total_connections.load(Ordering::Relaxed);
        if current_total >= self.max_total {
            return Err(SubscribeError::GlobalLimitReached);
        }

        let mut channels_guard = self.channels.write().await;

        if let Some(info) = channels_guard.get(channel) {
            // Check connection limit per channel
            if info.sender.receiver_count() >= self.max_per_channel {
                return Err(SubscribeError::ChannelFull);
            }
            // Update last activity time (lock-free)
            info.touch();
            self.total_connections.fetch_add(1, Ordering::Relaxed);
            Ok(info.sender.subscribe())
        } else {
            // Check channel limit before creating new channel
            if channels_guard.len() >= self.max_channels {
                return Err(SubscribeError::TooManyChannels);
            }
            let (tx, rx) = broadcast::channel(self.channel_capacity);
            channels_guard.insert(channel.to_string(), ChannelInfo::new(tx));
            self.total_connections.fetch_add(1, Ordering::Relaxed);
            Ok(rx)
        }
    }

    /// Decrement connection count when a subscriber disconnects
    pub fn on_disconnect(&self) {
        self.total_connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// Broadcast a message to all subscribers of a channel
    /// Scalability: Using read lock + atomic update for better concurrency
    pub async fn broadcast(&self, channel: &str, message: WsMessage) -> Result<usize, BroadcastError> {
        // Use read lock for better concurrency - last_activity is updated atomically
        let channels_guard = self.channels.read().await;

        if let Some(info) = channels_guard.get(channel) {
            // Lock-free update of last activity
            info.touch();
            match info.sender.send(message) {
                Ok(count) => Ok(count),
                Err(_) => Err(BroadcastError::NoSubscribers),
            }
        } else {
            Err(BroadcastError::ChannelNotFound)
        }
    }

    /// Clean up empty channels and channels that have exceeded TTL
    pub async fn cleanup_empty_channels(&self) {
        let mut channels_guard = self.channels.write().await;
        let ttl = self.channel_ttl;

        let before_count = channels_guard.len();
        channels_guard.retain(|_, info| {
            // Keep channel if it has subscribers
            if info.sender.receiver_count() > 0 {
                return true;
            }
            // Remove channel if it has no subscribers AND exceeded TTL since last activity
            info.last_activity_duration() < ttl
        });

        let removed = before_count - channels_guard.len();
        if removed > 0 {
            tracing::info!("WsManager: cleaned up {} stale channels", removed);
        }
    }

    /// Get the number of active channels
    pub async fn channel_count(&self) -> usize {
        self.channels.read().await.len()
    }

}

impl Default for WsManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BroadcastError {
    #[error("Channel not found")]
    ChannelNotFound,
    #[error("No subscribers")]
    NoSubscribers,
}

#[derive(Debug, thiserror::Error)]
pub enum SubscribeError {
    #[error("Channel has reached maximum connections")]
    ChannelFull,
    #[error("Global connection limit reached")]
    GlobalLimitReached,
    #[error("Maximum number of channels reached")]
    TooManyChannels,
}

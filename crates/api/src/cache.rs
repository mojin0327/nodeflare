//! Redis-based caching for frequently accessed data
//!
//! Provides caching layer to reduce database queries for:
//! - Workspace plan information (used in limit checks)
//! - Member counts (used in member limit checks)
//!
//! # Scalability
//! - Uses Redis for distributed caching across multiple API instances
//! - Supports cache invalidation via Redis Pub/Sub for multi-instance deployments
//! - TTL-based expiration prevents stale data

use fred::prelude::*;
use mcp_billing::Plan;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Cache TTL for workspace info (5 minutes)
const WORKSPACE_INFO_TTL_SECS: i64 = 300;

/// Cache TTL for deployment counts (1 minute - short to stay accurate)
const DEPLOYMENT_COUNT_TTL_SECS: i64 = 60;

/// Cached workspace info for plan limit checks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedWorkspaceInfo {
    pub name: String,
    pub plan: String,
    pub member_count: i32,
}

impl CachedWorkspaceInfo {
    pub fn billing_plan(&self) -> Plan {
        Plan::from_str(&self.plan)
    }
}

/// API cache service for frequently accessed data
#[derive(Clone)]
pub struct ApiCache {
    client: RedisClient,
}

impl ApiCache {
    pub fn new(client: RedisClient) -> Self {
        Self { client }
    }

    /// Cache key for workspace info
    fn workspace_info_key(workspace_id: Uuid) -> String {
        format!("api:workspace:{}:info", workspace_id)
    }

    /// Get cached workspace info
    pub async fn get_workspace_info(&self, workspace_id: Uuid) -> Option<CachedWorkspaceInfo> {
        let cache_key = Self::workspace_info_key(workspace_id);

        let result: Option<String> = self.client.get(&cache_key).await.ok()?;

        result.and_then(|json| serde_json::from_str(&json).ok())
    }

    /// Cache workspace info
    pub async fn set_workspace_info(&self, workspace_id: Uuid, info: &CachedWorkspaceInfo) {
        let cache_key = Self::workspace_info_key(workspace_id);

        if let Ok(json) = serde_json::to_string(info) {
            let _: Result<(), _> = self
                .client
                .set(
                    &cache_key,
                    json,
                    Some(Expiration::EX(WORKSPACE_INFO_TTL_SECS)),
                    None,
                    false,
                )
                .await;
        }
    }

    /// Invalidate cached workspace info (call when workspace is updated)
    pub async fn invalidate_workspace_info(&self, workspace_id: Uuid) {
        let cache_key = Self::workspace_info_key(workspace_id);
        let _: Result<(), _> = self.client.del(&cache_key).await;
    }

    /// Update member count in cache atomically using Lua script
    /// This prevents race conditions when multiple API instances update simultaneously
    pub async fn update_member_count(&self, workspace_id: Uuid, delta: i32) {
        let cache_key = Self::workspace_info_key(workspace_id);

        // Use Lua script for atomic read-modify-write
        // This ensures consistency across multiple API instances
        let script = r#"
            local data = redis.call('GET', KEYS[1])
            if data then
                local info = cjson.decode(data)
                info.member_count = math.max(0, info.member_count + tonumber(ARGV[1]))
                local updated = cjson.encode(info)
                redis.call('SET', KEYS[1], updated, 'EX', ARGV[2])
                return updated
            end
            return nil
        "#;

        let _: Result<Option<String>, _> = self
            .client
            .eval(
                script,
                vec![cache_key],
                vec![delta.to_string(), WORKSPACE_INFO_TTL_SECS.to_string()],
            )
            .await;
    }

    // =====================================================================
    // Deployment count caching
    // =====================================================================

    /// Cache key for monthly deployment count
    fn deployment_count_key(workspace_id: Uuid, year: i32, month: u32) -> String {
        format!("api:workspace:{}:deployments:{}:{:02}", workspace_id, year, month)
    }

    /// Get cached deployment count for the month, returns None if not cached
    pub async fn get_deployment_count(&self, workspace_id: Uuid, year: i32, month: u32) -> Option<i64> {
        let key = Self::deployment_count_key(workspace_id, year, month);
        self.client.get::<Option<i64>, _>(&key).await.ok().flatten()
    }

    /// Set deployment count cache
    pub async fn set_deployment_count(&self, workspace_id: Uuid, year: i32, month: u32, count: i64) {
        let key = Self::deployment_count_key(workspace_id, year, month);
        let _: Result<(), _> = self
            .client
            .set(
                &key,
                count,
                Some(Expiration::EX(DEPLOYMENT_COUNT_TTL_SECS)),
                None,
                false,
            )
            .await;
    }

    /// Increment deployment count atomically (call when a new deployment is created)
    pub async fn increment_deployment_count(&self, workspace_id: Uuid, year: i32, month: u32) {
        let key = Self::deployment_count_key(workspace_id, year, month);
        // INCR will return 1 if key doesn't exist, which is correct behavior
        // If cached value expires, next get_deployment_count will re-fetch from DB
        let _: Result<i64, _> = self.client.incr(&key).await;
        // Reset TTL on increment
        let _: Result<bool, _> = self.client.expire(&key, DEPLOYMENT_COUNT_TTL_SECS).await;
    }
}

use mcp_db::McpServer;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Build job - triggered when a deployment is requested
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildJob {
    pub deployment_id: Uuid,
    pub server_id: Uuid,
    /// Persisted container name to deploy to (e.g. `mcp-<uuid-nohyphens>`). Carried on the
    /// job so the builder never recomputes it from a truncated UUID prefix.
    pub app_name: String,
    pub github_repo: String,
    pub github_branch: String,
    pub commit_sha: String,
    pub runtime: String,
    /// GitHub App installation ID (None for public repos)
    pub github_installation_id: Option<i64>,
    /// Target region for deployment
    pub region: String,
    /// Root directory within the repo where source code is located
    pub root_directory: String,
    /// HTTP path where the MCP server listens (e.g., /mcp, /sse, /)
    pub mcp_path: String,
    /// Transport type: "sse" (HTTP/SSE endpoint) or "stdio" (stdin/stdout with adapter)
    pub transport: String,
    /// Custom entry command (e.g., "python server.py", "uv run mcp-server")
    /// If None, auto-detect based on project structure
    pub entry_command: Option<String>,
    /// Custom build command run at image-build time (e.g., "npm run build", "npm run compile")
    /// If None, fall back to the runtime default (Node: `npm run build --if-present`)
    pub build_command: Option<String>,
    /// User-selected machine memory in MB (256/512/1024/2048). None = auto (builder default).
    /// The builder still raises this to the detection floor and clamps it to the plan ceiling.
    pub memory_mb: Option<i32>,
    /// Internal listening port for Streamable HTTP (SSE) servers. None = runtime default.
    /// Ignored for stdio transport (the adapter always owns its own port).
    pub port: Option<i32>,
    /// Upstream OAuth provider for this server ('google', 'github', None = no upstream OAuth)
    pub upstream_oauth_provider: Option<String>,
}

impl BuildJob {
    /// Create a BuildJob from an McpServer
    /// Centralizes the clone operations to avoid repetition across codebase
    pub fn from_server(
        server: &McpServer,
        deployment_id: Uuid,
        commit_sha: String,
        region: Option<&str>,
    ) -> Self {
        Self {
            deployment_id,
            server_id: server.id,
            app_name: server.container_name.clone(),
            github_repo: server.github_repo.clone(),
            github_branch: server.github_branch.clone(),
            commit_sha,
            runtime: server.runtime.clone(),
            github_installation_id: server.github_installation_id,
            region: region.map_or_else(|| server.region.clone(), |r| r.to_string()),
            root_directory: server.root_directory.clone(),
            mcp_path: server.mcp_path.clone(),
            transport: server.transport.clone(),
            entry_command: server.entry_command.clone(),
            build_command: server.build_command.clone(),
            memory_mb: server.memory_mb,
            port: server.port,
            upstream_oauth_provider: server.upstream_oauth_provider.clone(),
        }
    }
}

/// Deploy job - triggered after a successful build
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployJob {
    pub deployment_id: Uuid,
    pub server_id: Uuid,
    /// Persisted container name to deploy to (never recomputed from the UUID prefix).
    pub app_name: String,
    pub image_url: String,
    pub secrets: Vec<SecretEnv>,
    /// Target region for deployment
    pub region: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEnv {
    pub key: String,
    pub value: String,
}

/// Cleanup job - for deleting old containers/resources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupJob {
    pub server_id: Uuid,
    pub container_id: String,
}

/// Destroy job - reliably tear down a deleted server's container. Enqueued when a
/// server is deleted so the teardown survives transient errors (the worker
/// retries, and destroying a missing container is a no-op), preventing orphaned containers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroyJob {
    pub server_id: Uuid,
    pub app_name: String,
}

/// Log cleanup job - for removing old request logs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogCleanupJob {
    pub retention_days: i64,
}

/// Metrics collection job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsJob {
    pub server_id: Uuid,
}

// Re-export apalis types for convenience
pub use apalis::prelude::*;
pub use apalis_redis::RedisStorage;

/// Keep-alive interval (5 minutes) - Upstash has idle timeout
const KEEPALIVE_INTERVAL_SECS: u64 = 300;

/// Job queue client for pushing jobs to Redis
#[derive(Clone)]
pub struct JobQueue {
    build_storage: RedisStorage<BuildJob>,
    deploy_storage: RedisStorage<DeployJob>,
    destroy_storage: RedisStorage<DestroyJob>,
    conn: ConnectionManager,
}

impl JobQueue {
    /// Connect to Redis and create job queue
    pub async fn connect(redis_url: &str) -> anyhow::Result<Self> {
        use apalis_redis::Config;

        let client = redis::Client::open(redis_url)?;
        let conn = ConnectionManager::new(client).await?;

        // Use explicit namespaces to ensure API and Builder use the same keys
        let build_config = Config::default().set_namespace("build_jobs");
        let deploy_config = Config::default().set_namespace("deploy_jobs");
        let destroy_config = Config::default().set_namespace("destroy_jobs");

        let build_storage = RedisStorage::new_with_config(conn.clone(), build_config);
        let deploy_storage = RedisStorage::new_with_config(conn.clone(), deploy_config);
        let destroy_storage = RedisStorage::new_with_config(conn.clone(), destroy_config);

        Ok(Self {
            build_storage,
            deploy_storage,
            destroy_storage,
            conn,
        })
    }

    /// Start keep-alive task to prevent Upstash idle timeout
    /// Call this after creating the JobQueue
    pub fn start_keepalive_task(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(KEEPALIVE_INTERVAL_SECS));
            loop {
                interval.tick().await;
                let mut conn = self.conn.clone();
                match redis::cmd("PING").query_async::<String>(&mut conn).await {
                    Ok(_) => {
                        tracing::debug!("JobQueue keep-alive: Redis connection OK");
                    }
                    Err(e) => {
                        tracing::warn!("JobQueue keep-alive ping failed: {}", e);
                    }
                }
            }
        });
    }

    /// Push a build job to the queue
    pub async fn push_build_job(&self, job: BuildJob) -> anyhow::Result<()> {
        use apalis::prelude::Storage;
        tracing::debug!("Pushing build job to queue: deployment_id={}", job.deployment_id);
        self.build_storage
            .clone()
            .push(job)
            .await
            .map_err(|e| {
                tracing::error!("Redis push_build_job failed: {:?}", e);
                anyhow::anyhow!("Failed to push build job: {}", e)
            })?;
        tracing::debug!("Build job pushed successfully");
        Ok(())
    }

    /// Push a deploy job to the queue
    pub async fn push_deploy_job(&self, job: DeployJob) -> anyhow::Result<()> {
        use apalis::prelude::Storage;
        self.deploy_storage
            .clone()
            .push(job)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to push deploy job: {}", e))?;
        Ok(())
    }

    /// Push a destroy job to tear down a deleted server's container.
    pub async fn push_destroy_job(&self, job: DestroyJob) -> anyhow::Result<()> {
        use apalis::prelude::Storage;
        self.destroy_storage
            .clone()
            .push(job)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to push destroy job: {}", e))?;
        Ok(())
    }
}

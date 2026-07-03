//! `POST /servers/inspect` — auto-detect deploy configuration from a GitHub repo so the
//! server-create form can be pre-filled (Vercel-style). Runs the shared `mcp_detect`
//! algorithm over the GitHub Contents API — the same algorithm the builder runs over the
//! cloned repo — so what the form shows is what will actually build.
//!
//! Speed (why this feels instant):
//!   1. The whole file list is fetched ONCE via the recursive git-tree API, so `exists()`
//!      is an in-memory lookup and absent files cost zero network.
//!   2. Every file the detector will read is pre-fetched IN PARALLEL and cached, so the
//!      detection pass itself does no sequential network I/O.
//!   3. Results are cached in Redis per (repo, branch, subdir) for a short TTL, so the
//!      debounced/repeat calls the form makes return with no GitHub round-trips at all.
//! The shared, connection-pooled HTTP client (AppState.http) avoids a cold TLS handshake
//! per request and multiplexes the parallel fetches over one HTTP/2 connection.
//!
//! Auth uses the caller's linked-account OAuth token (as `list_repositories` does), so it
//! reads public repos and the user's own private repos without a GitHub App installation.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use axum::{extract::State, Json};
use fred::prelude::*;
use mcp_db::LinkedGitHubAccountRepository;
use mcp_detect::{Detection, RepoFiles};
use serde::Deserialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::state::AppState;

/// How long a detection result is cached. Short, since a repo can change; long enough to
/// collapse the burst of calls the form makes while the user edits the URL/subdir.
const INSPECT_CACHE_TTL_SECS: i64 = 120;

#[derive(Deserialize)]
pub struct InspectRequest {
    /// `owner/repo`.
    pub github_repo: String,
    pub github_branch: Option<String>,
    /// Linked GitHub account to authenticate as; defaults to the caller's primary.
    #[serde(default)]
    pub account_id: Option<Uuid>,
    /// Optional subdirectory the server lives in (monorepo member).
    #[serde(default)]
    pub root_directory: Option<String>,
}

#[derive(Deserialize)]
struct TreeResponse {
    #[serde(default)]
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
}

/// [`RepoFiles`] backed by the GitHub API over a shared pooled client. The recursive git
/// tree (all paths) is pre-loaded so `exists` never touches the network; content is
/// fetched (and pre-fetched in parallel) then memoized per path. If the tree came back
/// truncated (very large repos), `exists`/`read` fall back to a live fetch.
struct GitHubFiles {
    client: reqwest::Client,
    token: String,
    owner: String,
    repo: String,
    branch: String,
    paths: HashSet<String>,
    truncated: bool,
    cache: Mutex<HashMap<String, Option<String>>>,
}

impl GitHubFiles {
    async fn fetch(&self, path: &str) -> Option<String> {
        if let Some(hit) = self.cache.lock().await.get(path) {
            return hit.clone();
        }
        let url = format!(
            "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
            self.owner, self.repo, path, self.branch
        );
        let result = match self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            // `raw` returns file bytes directly (no base64 wrapper).
            .header("Accept", "application/vnd.github.raw+json")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp.text().await.ok(),
            _ => None,
        };
        self.cache.lock().await.insert(path.to_string(), result.clone());
        result
    }

    /// Fetch every path concurrently, priming the cache so the detection pass hits memory.
    async fn prefetch(&self, paths: &[String]) {
        futures::future::join_all(paths.iter().map(|p| self.fetch(p))).await;
    }
}

#[async_trait]
impl RepoFiles for GitHubFiles {
    async fn read(&self, path: &str) -> Option<String> {
        if !self.truncated && !self.paths.contains(path) {
            return None;
        }
        self.fetch(path).await
    }

    async fn exists(&self, path: &str) -> bool {
        if self.paths.contains(path) {
            return true;
        }
        self.truncated && self.fetch(path).await.is_some()
    }

    async fn list_dir(&self, _path: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Fetch the repo's recursive git tree once. Returns (all paths, truncated). On any
/// failure returns an empty set marked truncated so the detector falls back to live
/// probes (slower, but still correct).
async fn load_tree(
    client: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
    branch: &str,
) -> (HashSet<String>, bool) {
    let url = format!(
        "https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1",
        owner, repo, branch
    );
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => match r.json::<TreeResponse>().await {
            Ok(tree) => (tree.tree.into_iter().map(|e| e.path).collect(), tree.truncated),
            Err(_) => (HashSet::new(), true),
        },
        _ => (HashSet::new(), true),
    }
}

pub async fn inspect(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Json(body): Json<InspectRequest>,
) -> Result<Json<Detection>, AppError> {
    let started = Instant::now();

    let (owner, repo) = {
        let mut parts = body.github_repo.split('/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(o), Some(r), None) if !o.is_empty() && !r.is_empty() => {
                (o.to_string(), r.to_string())
            }
            _ => {
                return Err(AppError::bad_request(
                    "INVALID_GITHUB_REPO",
                    "GitHub repository must be in format 'owner/repo'",
                ))
            }
        }
    };

    let branch = body.github_branch.clone().unwrap_or_else(|| "main".to_string());
    let subdir = body
        .root_directory
        .as_deref()
        .map(|s| s.trim().trim_matches('/'))
        .filter(|s| !s.is_empty());

    let cache_key = format!("inspect:v1:{}/{}@{}#{}", owner, repo, branch, subdir.unwrap_or(""));

    // (3) Redis result cache — a hit skips GitHub, the DB and token decryption entirely.
    if let Some(cached) = state.redis.get::<Option<String>, _>(&cache_key).await.ok().flatten() {
        if let Ok(det) = serde_json::from_str::<Detection>(&cached) {
            tracing::info!("inspect cache HIT {} in {:?}", cache_key, started.elapsed());
            return Ok(Json(det));
        }
    }

    // Resolve the caller's GitHub OAuth token (same path as listing repositories).
    let account = match body.account_id {
        Some(id) => {
            LinkedGitHubAccountRepository::get_with_token(&state.db, id, auth_user.user_id).await
        }
        None => LinkedGitHubAccountRepository::get_any_with_token(&state.db, auth_user.user_id).await,
    }
    .map_err(|e| {
        tracing::error!("inspect: failed to load linked GitHub account: {}", e);
        AppError::internal("Failed to load GitHub account")
    })?
    .ok_or_else(|| {
        AppError::bad_request("NO_GITHUB_ACCOUNT", "No linked GitHub account to inspect the repository")
    })?;

    let token = state
        .crypto
        .decrypt_string(&account.access_token_encrypted, &account.access_token_nonce)
        .map_err(|e| {
            tracing::error!("inspect: token decryption failed: {}", e);
            AppError::internal("GitHub token decryption failed")
        })?;

    // (2) Shared pooled client — no per-request TLS handshake; HTTP/2 multiplexing.
    let client = state.http.clone();

    let t_tree = Instant::now();
    let (paths, truncated) = load_tree(&client, &token, &owner, &repo, &branch).await;
    let tree_ms = t_tree.elapsed();

    // (1) Pre-fetch, in parallel, every file the detector will read that actually exists.
    let wanted: Vec<String> = if truncated {
        Vec::new()
    } else {
        mcp_detect::candidate_paths(subdir)
            .into_iter()
            .filter(|p| paths.contains(p))
            .collect()
    };

    let files = GitHubFiles {
        client,
        token,
        owner: owner.clone(),
        repo: repo.clone(),
        branch: branch.clone(),
        paths,
        truncated,
        cache: Mutex::new(HashMap::new()),
    };

    let t_pf = Instant::now();
    files.prefetch(&wanted).await;
    let prefetch_ms = t_pf.elapsed();

    let t_det = Instant::now();
    let detection = mcp_detect::inspect(&files, subdir).await;
    let detect_ms = t_det.elapsed();

    // Cache the result (best-effort).
    if let Ok(json) = serde_json::to_string(&detection) {
        let _: Result<(), _> = state
            .redis
            .set(&cache_key, json, Some(Expiration::EX(INSPECT_CACHE_TTL_SECS)), None, false)
            .await;
    }

    tracing::info!(
        "inspect MISS {} tree={:?} prefetch={:?}({} files) detect={:?} total={:?} truncated={}",
        cache_key, tree_ms, prefetch_ms, wanted.len(), detect_ms, started.elapsed(), truncated
    );

    Ok(Json(detection))
}

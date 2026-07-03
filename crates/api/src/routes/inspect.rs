//! `POST /servers/inspect` — auto-detect deploy configuration from a GitHub repo so the
//! server-create form can be pre-filled (Vercel-style). Runs the shared `mcp_detect`
//! algorithm over the GitHub Contents API — the same algorithm the builder runs over the
//! cloned repo — so what the form shows is what will actually build.
//!
//! Speed: the whole file list is fetched ONCE via the recursive git-tree API, so
//! `exists()` is an in-memory lookup (no network) and `read()` skips files that aren't in
//! the tree. That collapses the detector's dozens of existence probes into a single tree
//! request plus a handful of content fetches — the reason this feels instant.
//!
//! Auth uses the caller's linked-account OAuth token (as `list_repositories` does), so it
//! reads public repos and the user's own private repos without a GitHub App installation.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use axum::{extract::State, Json};
use mcp_db::LinkedGitHubAccountRepository;
use mcp_detect::{Detection, RepoFiles};
use serde::Deserialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::state::AppState;

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

/// [`RepoFiles`] backed by the GitHub API. The recursive git tree (all paths) is
/// pre-loaded so `exists` never touches the network; content is fetched lazily and
/// memoized per path. If the tree came back truncated (very large repos), `exists`/`read`
/// fall back to a live fetch so correctness is preserved.
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
            .header("User-Agent", "NodeFlare/1.0")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp.text().await.ok(),
            _ => None,
        };
        self.cache.lock().await.insert(path.to_string(), result.clone());
        result
    }
}

#[async_trait]
impl RepoFiles for GitHubFiles {
    async fn read(&self, path: &str) -> Option<String> {
        // Skip the network entirely for files the tree says don't exist.
        if !self.truncated && !self.paths.contains(path) {
            return None;
        }
        self.fetch(path).await
    }

    async fn exists(&self, path: &str) -> bool {
        if self.paths.contains(path) {
            return true;
        }
        // Truncated tree: the path might exist but be missing from the partial list.
        self.truncated && self.fetch(path).await.is_some()
    }

    async fn list_dir(&self, _path: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Fetch the repo's recursive git tree once. Returns (all paths, truncated). On any
/// failure, returns an empty set marked truncated so the detector falls back to live
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
        .header("User-Agent", "NodeFlare/1.0")
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => match r.json::<TreeResponse>().await {
            Ok(tree) => (
                tree.tree.into_iter().map(|e| e.path).collect(),
                tree.truncated,
            ),
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

    let branch = body.github_branch.clone().unwrap_or_else(|| "main".to_string());
    let subdir = body
        .root_directory
        .as_deref()
        .map(|s| s.trim().trim_matches('/'))
        .filter(|s| !s.is_empty());

    let client = reqwest::Client::new();
    let (paths, truncated) = load_tree(&client, &token, &owner, &repo, &branch).await;

    let files = GitHubFiles {
        client,
        token,
        owner,
        repo,
        branch,
        paths,
        truncated,
        cache: Mutex::new(HashMap::new()),
    };

    let detection = mcp_detect::inspect(&files, subdir).await;
    Ok(Json(detection))
}

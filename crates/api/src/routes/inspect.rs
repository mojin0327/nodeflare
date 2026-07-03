//! `POST /servers/inspect` — auto-detect deploy configuration from a GitHub repo so the
//! server-create form can be pre-filled (Vercel-style). Runs the shared `mcp_detect`
//! algorithm over the GitHub Contents API — the same algorithm the builder runs over the
//! cloned repo — so what the form shows is what will actually build.
//!
//! Auth uses the caller's linked-account OAuth token (as `list_repositories` does), so it
//! reads public repos and the user's own private repos without a GitHub App installation.

use std::collections::HashMap;
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

/// [`RepoFiles`] backed by the GitHub Contents API using a user OAuth token. Fetches are
/// memoized per path so an `exists` + `read` of the same file costs one request, and the
/// bounded set of probes `mcp_detect` makes stays well under the API rate limit.
struct GitHubFiles {
    client: reqwest::Client,
    token: String,
    owner: String,
    repo: String,
    branch: String,
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
            // `raw` returns file bytes directly (no base64 wrapper). Directories return
            // JSON, which we treat as "not a readable file" — the detector only reads files.
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
        self.fetch(path).await
    }
    async fn exists(&self, path: &str) -> bool {
        self.fetch(path).await.is_some()
    }
    async fn list_dir(&self, _path: &str) -> Vec<String> {
        Vec::new()
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

    let files = GitHubFiles {
        client: reqwest::Client::new(),
        token,
        owner,
        repo,
        branch,
        cache: Mutex::new(HashMap::new()),
    };

    let detection = mcp_detect::inspect(&files, subdir).await;
    Ok(Json(detection))
}

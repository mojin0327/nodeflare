//! `POST /servers/inspect` — auto-detect deploy configuration from a GitHub repo so the
//! server-create form can be pre-filled (Vercel-style). Runs the shared `mcp_detect`
//! algorithm over the GitHub API — the same algorithm the builder runs over the cloned
//! repo — so what the form shows is what will actually build.
//!
//! Speed (how this reaches ~1 round-trip, Vercel-style):
//!   - PRIMARY: one GraphQL request returns the default branch name, the contents of every
//!     candidate file, and the existence of every probe path — the entire detector input in
//!     a single round-trip, with no over-fetching (missing paths come back null).
//!   - FALLBACK (if GraphQL errors): the recursive git tree (one call, makes `exists()`
//!     free) + a parallel batch of content fetches.
//!   - Results are cached in Redis per (repo, branch, subdir) for a short TTL, and a shared
//!     pooled HTTP/2 client avoids per-request TLS handshakes.
//!
//! Auth uses the caller's linked-account OAuth token (as `list_repositories` does).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use axum::{extract::State, Json};
use fred::prelude::*;
use mcp_db::LinkedGitHubAccountRepository;
use mcp_detect::{Detection, RepoFiles};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::state::AppState;

const INSPECT_CACHE_TTL_SECS: i64 = 120;

#[derive(Deserialize)]
pub struct InspectRequest {
    /// `owner/repo`.
    pub github_repo: String,
    pub github_branch: Option<String>,
    #[serde(default)]
    pub account_id: Option<Uuid>,
    #[serde(default)]
    pub root_directory: Option<String>,
}

// ---------------------------------------------------------------------------------------
// PRIMARY path: a single GraphQL request for everything.
// ---------------------------------------------------------------------------------------

/// Files already resolved (contents + existence) from the batched GraphQL response, so the
/// detector runs entirely in memory with zero further network I/O.
struct BatchFiles {
    contents: HashMap<String, String>,
    present: HashSet<String>,
}

#[async_trait]
impl RepoFiles for BatchFiles {
    async fn read(&self, path: &str) -> Option<String> {
        self.contents.get(path).cloned()
    }
    async fn exists(&self, path: &str) -> bool {
        self.present.contains(path)
    }
    async fn list_dir(&self, _path: &str) -> Vec<String> {
        Vec::new()
    }
}

struct GqlResult {
    files: BatchFiles,
    default_branch: Option<String>,
}

/// Fetch the default branch name, the content of every `read_paths` file, and the existence
/// of every `probe_paths` file in ONE GraphQL request. `ref_prefix` is `"HEAD:"` (default
/// branch) or `"<branch>:"`. Returns None on any transport/GraphQL error so the caller can
/// fall back to REST.
async fn graphql_batch(
    client: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
    ref_prefix: &str,
    read_paths: &[String],
    probe_paths: &[String],
) -> Option<GqlResult> {
    let mut fields = String::from("defaultBranchRef { name }\n");
    for (i, p) in read_paths.iter().enumerate() {
        fields.push_str(&format!(
            "r{}: object(expression: {}) {{ ... on Blob {{ text }} }}\n",
            i,
            json!(format!("{}{}", ref_prefix, p))
        ));
    }
    for (i, p) in probe_paths.iter().enumerate() {
        fields.push_str(&format!(
            "e{}: object(expression: {}) {{ __typename }}\n",
            i,
            json!(format!("{}{}", ref_prefix, p))
        ));
    }
    let query = format!(
        "query {{ repository(owner: {}, name: {}) {{ {} }} }}",
        json!(owner),
        json!(repo),
        fields
    );

    let resp = client
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "NodeFlare/1.0")
        .json(&json!({ "query": query }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    let repo_obj = body.get("data")?.get("repository")?;
    if repo_obj.is_null() {
        return None;
    }

    let mut contents = HashMap::new();
    let mut present = HashSet::new();
    for (i, p) in read_paths.iter().enumerate() {
        if let Some(obj) = repo_obj.get(format!("r{}", i).as_str()) {
            if !obj.is_null() {
                present.insert(p.clone());
                if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                    contents.insert(p.clone(), text.to_string());
                }
            }
        }
    }
    for (i, p) in probe_paths.iter().enumerate() {
        if let Some(obj) = repo_obj.get(format!("e{}", i).as_str()) {
            if !obj.is_null() {
                present.insert(p.clone());
            }
        }
    }
    let default_branch = repo_obj
        .get("defaultBranchRef")
        .and_then(|r| r.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_string);

    Some(GqlResult { files: BatchFiles { contents, present }, default_branch })
}

// ---------------------------------------------------------------------------------------
// FALLBACK path: recursive git tree + parallel content prefetch over the Contents API.
// ---------------------------------------------------------------------------------------

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

#[derive(Deserialize)]
struct RepoMeta {
    default_branch: Option<String>,
}

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

async fn fetch_default_branch(
    client: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
) -> Option<String> {
    let url = format!("https://api.github.com/repos/{}/{}", owner, repo);
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<RepoMeta>().await.ok()?.default_branch
}

/// REST fallback: resolve the branch (default via HEAD tree + metadata in parallel), then
/// run the detector over a tree-backed, parallel-prefetched file view.
async fn rest_detect(
    client: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
    requested_branch: Option<String>,
    subdir: Option<&str>,
) -> Detection {
    let (form_branch, content_ref, paths, truncated) = match requested_branch {
        Some(b) => {
            let (paths, truncated) = load_tree(client, token, owner, repo, &b).await;
            (b.clone(), b, paths, truncated)
        }
        None => {
            let (name_opt, (paths, truncated)) = tokio::join!(
                fetch_default_branch(client, token, owner, repo),
                load_tree(client, token, owner, repo, "HEAD"),
            );
            let content_ref = name_opt.clone().unwrap_or_else(|| "HEAD".to_string());
            let form_branch = name_opt.unwrap_or_else(|| "main".to_string());
            (form_branch, content_ref, paths, truncated)
        }
    };

    let wanted: Vec<String> = if truncated {
        Vec::new()
    } else {
        mcp_detect::candidate_paths(subdir)
            .into_iter()
            .filter(|p| paths.contains(p))
            .collect()
    };

    let files = GitHubFiles {
        client: client.clone(),
        token: token.to_string(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        branch: content_ref,
        paths,
        truncated,
        cache: Mutex::new(HashMap::new()),
    };
    files.prefetch(&wanted).await;

    let mut detection = mcp_detect::inspect(&files, subdir).await;
    detection.branch = Some(form_branch);
    detection
}

// ---------------------------------------------------------------------------------------

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

    let requested_branch = body
        .github_branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let subdir = body
        .root_directory
        .as_deref()
        .map(|s| s.trim().trim_matches('/'))
        .filter(|s| !s.is_empty());

    let cache_key = format!(
        "inspect:v2:{}/{}@{}#{}",
        owner,
        repo,
        requested_branch.as_deref().unwrap_or(""),
        subdir.unwrap_or("")
    );

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

    let client = state.http.clone();

    // PRIMARY: one GraphQL request for branch + all file contents + existence.
    let read_paths = mcp_detect::candidate_paths(subdir);
    let probe_paths: Vec<String> = mcp_detect::probe_paths(subdir)
        .into_iter()
        .filter(|p| !read_paths.contains(p))
        .collect();
    let ref_prefix = match &requested_branch {
        Some(b) => format!("{}:", b),
        None => "HEAD:".to_string(),
    };

    let t_g = Instant::now();
    let gql = graphql_batch(&client, &token, &owner, &repo, &ref_prefix, &read_paths, &probe_paths).await;

    let (detection, path_kind) = match gql {
        Some(g) => {
            let form_branch = requested_branch
                .clone()
                .or(g.default_branch)
                .unwrap_or_else(|| "main".to_string());
            let mut det = mcp_detect::inspect(&g.files, subdir).await;
            det.branch = Some(form_branch);
            (det, "graphql")
        }
        None => (
            rest_detect(&client, &token, &owner, &repo, requested_branch, subdir).await,
            "rest-fallback",
        ),
    };

    if let Ok(json) = serde_json::to_string(&detection) {
        let _: Result<(), _> = state
            .redis
            .set(&cache_key, json, Some(Expiration::EX(INSPECT_CACHE_TTL_SECS)), None, false)
            .await;
    }

    tracing::info!(
        "inspect MISS {} via={} in {:?} (fetch={:?})",
        cache_key,
        path_kind,
        started.elapsed(),
        t_g.elapsed()
    );

    Ok(Json(detection))
}

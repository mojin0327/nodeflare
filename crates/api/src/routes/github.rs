use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use mcp_db::LinkedGitHubAccountRepository;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{db_error, internal_error};
use crate::extractors::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubRepo {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub private: bool,
    pub html_url: String,
    pub default_branch: String,
    pub updated_at: String,
    pub language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubRepoResponse {
    id: i64,
    name: String,
    full_name: String,
    description: Option<String>,
    private: bool,
    html_url: String,
    default_branch: String,
    updated_at: String,
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReposQuery {
    /// Optional account ID to fetch repositories from a specific linked GitHub account
    pub account_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct GitHubBranchResponse {
    name: String,
}

#[derive(Debug, Deserialize)]
pub struct BranchesQuery {
    /// `owner/repo` to list branches for.
    pub repo: String,
    /// Optional account ID whose OAuth token authenticates the GitHub call.
    pub account_id: Option<Uuid>,
}

pub async fn list_repositories(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Query(params): Query<ReposQuery>,
) -> Result<Json<Vec<GitHubRepo>>, (StatusCode, String)> {
    // Get the GitHub access token from linked accounts
    let linked_account = if let Some(account_id) = params.account_id {
        // Use specific account
        LinkedGitHubAccountRepository::get_with_token(&state.db, account_id, auth_user.user_id)
            .await
            .map_err(db_error)?
    } else {
        // Use primary or any linked account
        LinkedGitHubAccountRepository::get_any_with_token(&state.db, auth_user.user_id)
            .await
            .map_err(db_error)?
    };

    // If no linked account found, return empty array (not an error)
    let linked_account = match linked_account {
        Some(account) => account,
        None => {
            tracing::info!(
                "No linked GitHub account for user {}",
                auth_user.user_id
            );
            return Ok(Json(vec![]));
        }
    };

    // Decrypt GitHub access token
    let access_token = state
        .crypto
        .decrypt_string(&linked_account.access_token_encrypted, &linked_account.access_token_nonce)
        .map_err(|e| internal_error("Token decryption failed", e))?;

    // Fetch repositories from GitHub
    let client = reqwest::Client::new();
    let response = client
        .get("https://api.github.com/user/repos")
        .query(&[("sort", "updated"), ("per_page", "100")])
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "MCP-Cloud/1.0")
        .send()
        .await
        .map_err(db_error)?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!("GitHub API error: {} - {}", status, body);
        return Err((
            StatusCode::BAD_REQUEST,
            "Failed to fetch repositories. Please re-authenticate.".to_string(),
        ));
    }

    let repos: Vec<GitHubRepoResponse> = response
        .json()
        .await
        .map_err(db_error)?;

    let result: Vec<GitHubRepo> = repos
        .into_iter()
        .map(|r| GitHubRepo {
            id: r.id,
            name: r.name,
            full_name: r.full_name,
            description: r.description,
            private: r.private,
            html_url: r.html_url,
            default_branch: r.default_branch,
            updated_at: r.updated_at,
            language: r.language,
        })
        .collect();

    Ok(Json(result))
}

/// List a repository's branches (names only). Fetched lazily by the create form's branch
/// dropdown when the user opens it, so it never blocks the initial render or auto-detection.
/// Uses the caller's linked-account OAuth token, mirroring `list_repositories`.
pub async fn list_branches(
    State(state): State<Arc<AppState>>,
    auth_user: AuthUser,
    Query(params): Query<BranchesQuery>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    // Expect `owner/repo`; anything else is a client bug, so fail loudly.
    let (owner, repo) = match params.repo.split_once('/') {
        Some((o, r)) if !o.is_empty() && !r.is_empty() && !r.contains('/') => (o, r),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "repo must be in 'owner/repo' format".to_string(),
            ))
        }
    };

    // Resolve a GitHub token (specific account, else any linked one).
    let linked_account = if let Some(account_id) = params.account_id {
        LinkedGitHubAccountRepository::get_with_token(&state.db, account_id, auth_user.user_id)
            .await
            .map_err(db_error)?
    } else {
        LinkedGitHubAccountRepository::get_any_with_token(&state.db, auth_user.user_id)
            .await
            .map_err(db_error)?
    };

    // No linked account → nothing to authenticate with; return empty (not an error) so the
    // dropdown just keeps its seeded default branch.
    let linked_account = match linked_account {
        Some(account) => account,
        None => return Ok(Json(vec![])),
    };

    let access_token = state
        .crypto
        .decrypt_string(&linked_account.access_token_encrypted, &linked_account.access_token_nonce)
        .map_err(|e| internal_error("Token decryption failed", e))?;

    let client = reqwest::Client::new();
    let response = client
        .get(format!("https://api.github.com/repos/{}/{}/branches", owner, repo))
        .query(&[("per_page", "100")])
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "MCP-Cloud/1.0")
        .send()
        .await
        .map_err(db_error)?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::warn!("GitHub branches API error: {} - {}", status, body);
        // Non-fatal: the form falls back to the seeded default branch.
        return Ok(Json(vec![]));
    }

    let branches: Vec<GitHubBranchResponse> = response.json().await.map_err(db_error)?;
    Ok(Json(branches.into_iter().map(|b| b.name).collect()))
}

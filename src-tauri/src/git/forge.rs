use std::path::Path;

use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};

use super::{
    error::GitError,
    remote::{current_branch_name, current_head_summary, read_origin_info},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedPullRequest {
    pub url: String,
    pub number: u64,
}

#[derive(Debug, Deserialize)]
struct GithubRepoResponse {
    default_branch: String,
}

#[derive(Debug, Serialize)]
struct GithubCreatePrRequest<'a> {
    title: &'a str,
    head: &'a str,
    base: &'a str,
    draft: bool,
}

#[derive(Debug, Deserialize)]
struct GithubCreatePrResponse {
    number: u64,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubErrorResponse {
    message: Option<String>,
}

pub async fn create_pull_request(
    cwd: &Path,
    title: Option<String>,
    draft: bool,
) -> Result<CreatedPullRequest, GitError> {
    let remote = read_origin_info(cwd)?.ok_or_else(|| {
        GitError::CommandFailed("git remote origin is not configured".to_string())
    })?;
    let branch = current_branch_name(cwd)?.ok_or_else(|| {
        GitError::CommandFailed("cannot create a pull request from a detached HEAD".to_string())
    })?;

    if !remote.host.eq_ignore_ascii_case("github.com")
        && !remote.host.to_ascii_lowercase().contains("github")
    {
        return Err(GitError::CommandFailed(format!(
            "pull request creation is currently only implemented for GitHub remotes (found {})",
            remote.host
        )));
    }

    let token = std::env::var("GH_TOKEN")
        .ok()
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .ok_or_else(|| {
            GitError::CommandFailed(
                "set GH_TOKEN or GITHUB_TOKEN to create GitHub pull requests".to_string(),
            )
        })?;

    let api_base = github_api_base(&remote.host);
    let repo_api = format!("{api_base}/repos/{}/{}/", remote.owner, remote.repo);
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| GitError::CommandFailed(format!("failed to build HTTP client: {e}")))?;

    let default_branch: GithubRepoResponse = send_github(
        client
            .get(format!("{repo_api}"))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(USER_AGENT, "jockey")
            .header(ACCEPT, "application/vnd.github+json"),
    )
    .await?;

    let fallback_title = title
        .filter(|value| !value.trim().is_empty())
        .or_else(|| current_head_summary(cwd).ok().flatten())
        .unwrap_or_else(|| format!("Open PR for {branch}"));

    let payload = GithubCreatePrRequest {
        title: fallback_title.trim(),
        head: branch.as_str(),
        base: default_branch.default_branch.as_str(),
        draft,
    };

    let created: GithubCreatePrResponse = send_github(
        client
            .post(format!("{repo_api}pulls"))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(USER_AGENT, "jockey")
            .header(ACCEPT, "application/vnd.github+json")
            .json(&payload),
    )
    .await?;

    Ok(CreatedPullRequest {
        url: created.html_url,
        number: created.number,
    })
}

fn github_api_base(host: &str) -> String {
    if host.eq_ignore_ascii_case("github.com") {
        "https://api.github.com".to_string()
    } else {
        format!("https://{host}/api/v3")
    }
}

async fn send_github<T>(request: reqwest::RequestBuilder) -> Result<T, GitError>
where
    T: for<'de> Deserialize<'de>,
{
    let response = request
        .send()
        .await
        .map_err(|e| GitError::CommandFailed(format!("GitHub request failed: {e}")))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| GitError::CommandFailed(format!("GitHub response read failed: {e}")))?;

    if !status.is_success() {
        let message = serde_json::from_slice::<GithubErrorResponse>(&bytes)
            .ok()
            .and_then(|body| body.message)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes).trim().to_string());
        return Err(GitError::CommandFailed(format!(
            "GitHub API error ({}): {}",
            status.as_u16(),
            message
        )));
    }

    serde_json::from_slice(&bytes)
        .map_err(|e| GitError::CommandFailed(format!("invalid GitHub response: {e}")))
}

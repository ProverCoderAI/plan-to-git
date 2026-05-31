use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitContext {
    pub repo_root: PathBuf,
    pub repo_slug: Option<String>,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
}

pub fn discover(start: &Path) -> AppResult<GitContext> {
    let repo_root = git_output(start, ["rev-parse", "--show-toplevel"])?;
    let repo_root = PathBuf::from(repo_root.trim());
    let branch = git_output(&repo_root, ["rev-parse", "--abbrev-ref", "HEAD"]).ok();
    let head_sha = git_output(&repo_root, ["rev-parse", "HEAD"]).ok();
    let remote = git_output(&repo_root, ["remote", "get-url", "origin"]).ok();

    Ok(GitContext {
        repo_root,
        repo_slug: remote.as_deref().and_then(parse_github_slug),
        branch: branch.map(|value| value.trim().to_owned()),
        head_sha: head_sha.map(|value| value.trim().to_owned()),
    })
}

#[must_use]
pub fn parse_github_slug(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches(".git");

    if let Some(path) = remote.strip_prefix("git@github.com:") {
        return normalize_slug(path);
    }

    if let Some(path) = remote.strip_prefix("https://github.com/") {
        return normalize_slug(path);
    }

    if let Some(path) = remote.strip_prefix("ssh://git@github.com/") {
        return normalize_slug(path);
    }

    None
}

fn normalize_slug(path: &str) -> Option<String> {
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

fn git_output<const N: usize>(cwd: &Path, args: [&str; N]) -> AppResult<String> {
    let output = Command::new("git").arg("-C").arg(cwd).args(args).output()?;
    if output.status.success() {
        return Ok(String::from_utf8(output.stdout)?.trim().to_owned());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(AppError::new(format!("git command failed: {stderr}")).into())
}

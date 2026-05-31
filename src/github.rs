use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::git::GitContext;
use crate::pr_body::upsert_marker_block;
use crate::render::{has_current_branch_items, render_plan_block};
use crate::store::AgentPlanState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    NoItems,
    NoPullRequest,
    Unchanged { number: u64 },
    Updated { number: u64 },
}

#[derive(Debug, Deserialize)]
struct PullRequest {
    number: u64,
    #[serde(default)]
    body: Option<String>,
}

pub fn sync_state(context: &GitContext, state: &AgentPlanState) -> AppResult<SyncStatus> {
    if !has_current_branch_items(state) {
        return Ok(SyncStatus::NoItems);
    }

    let Some(pull_request) = view_current_pr(&context.repo_root)? else {
        return Ok(SyncStatus::NoPullRequest);
    };

    let body = pull_request.body.unwrap_or_default();
    let plan_block = render_plan_block(state);
    let updated_body = upsert_marker_block(&body, &plan_block)?;

    if body.trim_end() == updated_body.trim_end() {
        return Ok(SyncStatus::Unchanged {
            number: pull_request.number,
        });
    }

    edit_pr_body(&context.repo_root, &updated_body)?;
    Ok(SyncStatus::Updated {
        number: pull_request.number,
    })
}

fn view_current_pr(repo_root: &Path) -> AppResult<Option<PullRequest>> {
    let output = Command::new("gh")
        .current_dir(repo_root)
        .args(["pr", "view", "--json", "number,body"])
        .output()?;

    if output.status.success() {
        return Ok(Some(serde_json::from_slice(&output.stdout)?));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("no pull requests found") {
        return Ok(None);
    }

    Err(AppError::new(format!("gh pr view failed: {stderr}")).into())
}

fn edit_pr_body(repo_root: &Path, body: &str) -> AppResult<()> {
    let body_file = temp_body_path();
    fs::write(&body_file, body)?;

    let output = Command::new("gh")
        .current_dir(repo_root)
        .args(["pr", "edit", "--body-file"])
        .arg(&body_file)
        .output();

    let remove_result = fs::remove_file(&body_file);
    let output = output?;
    if let Err(error) = remove_result {
        return Err(error.into());
    }

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(AppError::new(format!("gh pr edit failed: {stderr}")).into())
}

fn temp_body_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    path.push(format!(
        "plan-to-git-pr-body-{}-{timestamp}.md",
        std::process::id()
    ));
    path
}

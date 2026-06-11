use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::git::GitContext;
use crate::render::render_plan_comment;
use crate::store::AgentPlanState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    NoItems,
    NoPullRequest,
    ClosedPullRequest {
        number: u64,
        state: String,
    },
    DraftPullRequest {
        number: u64,
    },
    Unchanged {
        number: u64,
    },
    Commented {
        number: u64,
        comment_id: u64,
        items: usize,
    },
}

#[derive(Debug, Deserialize)]
struct PullRequest {
    number: u64,
    state: String,
    #[serde(default, rename = "isDraft")]
    is_draft: bool,
}

#[derive(Debug, Deserialize)]
struct IssueComment {
    id: u64,
}

pub fn sync_state(context: &GitContext, state: &mut AgentPlanState) -> AppResult<SyncStatus> {
    if !state.has_current_branch_items() {
        return Ok(SyncStatus::NoItems);
    }

    let Some(pull_request) = view_current_pr(&context.repo_root)? else {
        return Ok(SyncStatus::NoPullRequest);
    };
    if !pull_request.state.eq_ignore_ascii_case("OPEN") {
        return Ok(SyncStatus::ClosedPullRequest {
            number: pull_request.number,
            state: pull_request.state,
        });
    }
    if pull_request.is_draft {
        return Ok(SyncStatus::DraftPullRequest {
            number: pull_request.number,
        });
    }

    let (comment_body, item_ids, item_count) = {
        let items = state.unposted_items_for_pr(pull_request.number);
        if items.is_empty() {
            return Ok(SyncStatus::Unchanged {
                number: pull_request.number,
            });
        }
        let item_ids = items.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
        (render_plan_comment(state, &items), item_ids, items.len())
    };

    let comment_id = create_issue_comment(context, pull_request.number, &comment_body)?;
    state.mark_items_commented(pull_request.number, &item_ids, Some(comment_id));

    Ok(SyncStatus::Commented {
        number: pull_request.number,
        comment_id,
        items: item_count,
    })
}

fn view_current_pr(repo_root: &Path) -> AppResult<Option<PullRequest>> {
    let output = Command::new("gh")
        .current_dir(repo_root)
        .args(["pr", "view", "--json", "number,state,url,isDraft"])
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

fn create_issue_comment(context: &GitContext, number: u64, body: &str) -> AppResult<u64> {
    let repo_slug = context
        .repo_slug
        .as_deref()
        .ok_or_else(|| AppError::new("cannot sync PR comments without a GitHub origin remote"))?;
    let request_file = temp_request_path();
    let request = serde_json::json!({ "body": body });
    fs::write(&request_file, serde_json::to_vec(&request)?)?;

    let output = Command::new("gh")
        .current_dir(&context.repo_root)
        .args(["api", "--method", "POST"])
        .arg(format!("repos/{repo_slug}/issues/{number}/comments"))
        .args(["--input"])
        .arg(&request_file)
        .output();

    let remove_result = fs::remove_file(&request_file);
    let output = output?;
    if let Err(error) = remove_result {
        return Err(error.into());
    }

    if output.status.success() {
        let comment: IssueComment = serde_json::from_slice(&output.stdout)?;
        return Ok(comment.id);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(AppError::new(format!("gh api PR comment failed: {stderr}")).into())
}

fn temp_request_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    path.push(format!(
        "plan-to-git-pr-comment-{}-{timestamp}.json",
        std::process::id()
    ));
    path
}

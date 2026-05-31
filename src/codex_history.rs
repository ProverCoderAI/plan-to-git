use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::git::{parse_github_slug, GitContext};
use crate::normalize::extract_marked_plans;
use crate::store::{AgentPlanState, AgentSource, NewPlanItem};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexHistoryImportOutcome {
    pub files_scanned: usize,
    pub files_matched: usize,
    pub lines_scanned: usize,
    pub parse_errors: usize,
    pub plans_found: usize,
    pub plans_added: usize,
    pub duplicates: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionMetadata {
    id: Option<String>,
    repo_slug: Option<String>,
    branch: Option<String>,
    cwd: Option<PathBuf>,
}

pub fn import_codex_history(
    codex_home: &Path,
    context: &GitContext,
    state: &mut AgentPlanState,
) -> AppResult<CodexHistoryImportOutcome> {
    let mut outcome = CodexHistoryImportOutcome {
        files_scanned: 0,
        files_matched: 0,
        lines_scanned: 0,
        parse_errors: 0,
        plans_found: 0,
        plans_added: 0,
        duplicates: 0,
    };

    let mut files = codex_session_files(codex_home)?;
    files.sort();

    for path in files {
        outcome.files_scanned += 1;
        import_session_file(&path, context, state, &mut outcome)?;
    }

    Ok(outcome)
}

fn import_session_file(
    path: &Path,
    context: &GitContext,
    state: &mut AgentPlanState,
    outcome: &mut CodexHistoryImportOutcome,
) -> AppResult<()> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut metadata: Option<SessionMetadata> = None;
    let mut file_matches = false;

    for (line_index, line) in reader.lines().enumerate() {
        outcome.lines_scanned += 1;
        let line = line?;
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            outcome.parse_errors += 1;
            continue;
        };

        if event.get("type").and_then(Value::as_str) == Some("session_meta") {
            metadata = Some(parse_session_metadata(&event));
            file_matches = metadata
                .as_ref()
                .is_some_and(|session| session_matches_context(session, context));
            if file_matches {
                outcome.files_matched += 1;
            }
            continue;
        }

        if !file_matches {
            continue;
        }

        let Some(message) = plan_message_text(&event) else {
            continue;
        };

        let session_id = metadata
            .as_ref()
            .and_then(|session| session.id.clone())
            .or_else(|| session_id_from_path(path));
        let turn_id = event
            .get("payload")
            .and_then(|payload| payload.get("turn_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| line_turn_id(path, line_index + 1));
        let created_at = event
            .get("timestamp")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        for plan in extract_marked_plans(&message) {
            outcome.plans_found += 1;
            let added = state.add_plan(NewPlanItem {
                source: AgentSource::Codex,
                title: plan.title,
                content: plan.content,
                branch: context.branch.clone(),
                head_sha: context.head_sha.clone(),
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                created_at: created_at.clone(),
            });

            if added {
                outcome.plans_added += 1;
            } else {
                outcome.duplicates += 1;
            }
        }
    }

    Ok(())
}

fn codex_session_files(codex_home: &Path) -> AppResult<Vec<PathBuf>> {
    let sessions_dir = codex_home.join("sessions");
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_jsonl_files(&sessions_dir, &mut files)?;
    Ok(files)
}

fn collect_jsonl_files(dir: &Path, files: &mut Vec<PathBuf>) -> AppResult<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn parse_session_metadata(event: &Value) -> SessionMetadata {
    let payload = event.get("payload");
    let id = payload
        .and_then(|payload| payload.get("id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let branch = payload
        .and_then(|payload| payload.get("git"))
        .and_then(|git| git.get("branch"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let repo_slug = payload
        .and_then(|payload| payload.get("git"))
        .and_then(|git| git.get("repository_url"))
        .and_then(Value::as_str)
        .and_then(parse_github_slug);
    let cwd = payload
        .and_then(|payload| payload.get("cwd"))
        .and_then(Value::as_str)
        .map(PathBuf::from);

    SessionMetadata {
        id,
        repo_slug,
        branch,
        cwd,
    }
}

fn session_matches_context(session: &SessionMetadata, context: &GitContext) -> bool {
    let repo_matches = match (&context.repo_slug, &session.repo_slug) {
        (Some(current), Some(history)) => current == history,
        _ => session
            .cwd
            .as_ref()
            .is_some_and(|cwd| cwd.starts_with(&context.repo_root)),
    };
    let branch_matches = match (&context.branch, &session.branch) {
        (Some(current), Some(history)) => current == history,
        _ => true,
    };

    repo_matches && branch_matches
}

fn plan_message_text(event: &Value) -> Option<String> {
    assistant_message_text(event).or_else(|| task_complete_message_text(event))
}

fn assistant_message_text(event: &Value) -> Option<String> {
    if event.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }

    let payload = event.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    if payload.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }

    let text = payload
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");

    (!text.trim().is_empty()).then_some(text)
}

fn task_complete_message_text(event: &Value) -> Option<String> {
    if event.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }

    let payload = event.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("task_complete") {
        return None;
    }

    let text = payload
        .get("last_agent_message")
        .or_else(|| payload.get("last_assistant_message"))
        .and_then(Value::as_str)?;

    (!text.trim().is_empty()).then_some(text.to_owned())
}

fn session_id_from_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
}

fn line_turn_id(path: &Path, line_number: usize) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    Some(format!("{stem}:{line_number}"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::git::GitContext;
    use crate::store::AgentPlanState;

    use super::import_codex_history;

    #[test]
    fn imports_marked_assistant_plans_and_skips_duplicates() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let codex_home = temp_dir.path().join("codex");
        let session_dir = codex_home.join("sessions/2026/05/31");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        let session = session_dir.join("rollout-2026-05-31T00-00-00-session.jsonl");
        fs::write(
            &session,
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"session","cwd":"{}","git":{{"branch":"feature/test","repository_url":"https://github.com/example/repo.git"}}}}}}
{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"<proposed_plan>ignore user text</proposed_plan>"}}]}}}}
{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"not a marked plan"}}]}}}}
{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"<proposed_plan>\n# Backfill\n\n- Import old plans\n- Keep api_key=secret-value private\n</proposed_plan>"}}]}}}}
{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"<proposed_plan>\n# Backfill\n\n- Import old plans\n- Keep api_key=secret-value private\n</proposed_plan>"}}]}}}}
"#,
                repo_root.display()
            ),
        )
        .expect("write session");

        let context = GitContext {
            repo_root,
            repo_slug: Some("example/repo".to_owned()),
            branch: Some("feature/test".to_owned()),
            head_sha: Some("abcdef".to_owned()),
        };
        let mut state = AgentPlanState::default();

        let outcome = import_codex_history(&codex_home, &context, &mut state).expect("import");

        assert_eq!(outcome.files_scanned, 1);
        assert_eq!(outcome.files_matched, 1);
        assert_eq!(outcome.plans_found, 2);
        assert_eq!(outcome.plans_added, 1);
        assert_eq!(outcome.duplicates, 1);
        assert_eq!(state.items.len(), 1);
        assert!(state.items[0].content.contains("Import old plans"));
        assert!(state.items[0].content.contains("api_key=[REDACTED]"));
        assert!(!state.items[0].content.contains("secret-value"));
        assert_eq!(
            state.items[0].turn_id.as_deref(),
            Some("rollout-2026-05-31T00-00-00-session:4")
        );
    }

    #[test]
    fn skips_sessions_from_other_branches() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let codex_home = temp_dir.path().join("codex");
        let session_dir = codex_home.join("sessions/2026/05/31");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        fs::write(
            session_dir.join("rollout-2026-05-31T00-00-00-other.jsonl"),
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"session","cwd":"{}","git":{{"branch":"feature/other","repository_url":"https://github.com/example/repo.git"}}}}}}
{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"<proposed_plan>\n# Other\n\n- Wrong branch\n</proposed_plan>"}}]}}}}
"#,
                repo_root.display()
            ),
        )
        .expect("write session");

        let context = GitContext {
            repo_root,
            repo_slug: Some("example/repo".to_owned()),
            branch: Some("feature/test".to_owned()),
            head_sha: Some("abcdef".to_owned()),
        };
        let mut state = AgentPlanState::default();

        let outcome = import_codex_history(&codex_home, &context, &mut state).expect("import");

        assert_eq!(outcome.files_scanned, 1);
        assert_eq!(outcome.files_matched, 0);
        assert_eq!(outcome.plans_found, 0);
        assert!(state.items.is_empty());
    }

    #[test]
    fn imports_task_complete_last_agent_message() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let codex_home = temp_dir.path().join("codex");
        let session_dir = codex_home.join("sessions/2026/05/31");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        fs::write(
            session_dir.join("rollout-2026-05-31T00-00-00-complete.jsonl"),
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"session","cwd":"{}","git":{{"branch":"feature/test","repository_url":"https://github.com/example/repo.git"}}}}}}
{{"timestamp":"2026-05-31T12:34:56Z","type":"event_msg","payload":{{"type":"task_complete","last_agent_message":"<proposed_plan>\n# Complete\n\n- Import completion text\n</proposed_plan>"}}}}
"#,
                repo_root.display()
            ),
        )
        .expect("write session");

        let context = GitContext {
            repo_root,
            repo_slug: Some("example/repo".to_owned()),
            branch: Some("feature/test".to_owned()),
            head_sha: Some("abcdef".to_owned()),
        };
        let mut state = AgentPlanState::default();

        let outcome = import_codex_history(&codex_home, &context, &mut state).expect("import");

        assert_eq!(outcome.plans_added, 1);
        assert!(state.items[0].content.contains("Import completion text"));
        assert_eq!(state.items[0].created_at, "2026-05-31T12:34:56Z");
    }

    #[test]
    fn skips_sessions_without_positive_repo_or_cwd_match() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let codex_home = temp_dir.path().join("codex");
        let session_dir = codex_home.join("sessions/2026/05/31");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        fs::write(
            session_dir.join("rollout-2026-05-31T00-00-00-no-context.jsonl"),
            r#"{"type":"session_meta","payload":{"id":"session","git":{"branch":"feature/test"}}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"<proposed_plan>\n# No Context\n\n- Do not import\n</proposed_plan>"}]}}
"#,
        )
        .expect("write session");

        let context = GitContext {
            repo_root,
            repo_slug: Some("example/repo".to_owned()),
            branch: Some("feature/test".to_owned()),
            head_sha: Some("abcdef".to_owned()),
        };
        let mut state = AgentPlanState::default();

        let outcome = import_codex_history(&codex_home, &context, &mut state).expect("import");

        assert_eq!(outcome.files_matched, 0);
        assert!(state.items.is_empty());
    }
}

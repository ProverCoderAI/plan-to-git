use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::git::{parse_github_slug, GitContext};
use crate::history::{
    collect_jsonl_files, line_turn_id, looks_like_rendered_plan_stack, session_id_from_path,
    HistoryImportOutcome,
};
use crate::normalize::{extract_marked_plans, CapturedPlan};
use crate::store::{AgentPlanState, AgentSource, NewPlanItem};

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
) -> AppResult<HistoryImportOutcome> {
    let mut outcome = HistoryImportOutcome::default();
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
    outcome: &mut HistoryImportOutcome,
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

        let plans = event_plans(&event);
        if plans.is_empty() {
            continue;
        }

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

        for plan in plans {
            outcome.plans_found += 1;
            if looks_like_rendered_plan_stack(&plan.content) {
                outcome.rendered_stacks_skipped += 1;
                continue;
            }
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

fn event_plans(event: &Value) -> Vec<CapturedPlan> {
    if let Some(message) = plan_message_text(event) {
        return extract_marked_plans(&message);
    }

    codex_update_plan(event).into_iter().collect()
}

fn codex_update_plan(event: &Value) -> Option<CapturedPlan> {
    if event.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }

    let payload = event.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    if payload.get("name").and_then(Value::as_str) != Some("update_plan") {
        return None;
    }

    let arguments = payload.get("arguments").and_then(Value::as_str)?;
    let arguments = serde_json::from_str::<Value>(arguments).ok()?;
    captured_plan_from_update_plan_arguments(&arguments)
}

fn captured_plan_from_update_plan_arguments(arguments: &Value) -> Option<CapturedPlan> {
    let steps = arguments
        .get("plan")?
        .as_array()?
        .iter()
        .filter_map(update_plan_step)
        .collect::<Vec<_>>();
    if steps.is_empty() {
        return None;
    }

    let mut content = String::from("# Codex Plan\n");
    if let Some(explanation) = arguments
        .get("explanation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|explanation| !explanation.is_empty())
    {
        content.push('\n');
        content.push_str(explanation);
        content.push('\n');
    }

    content.push_str("\n## Steps\n\n");
    for (status, step) in steps {
        content.push_str("- ");
        content.push_str(status);
        content.push_str(": ");
        content.push_str(step);
        content.push('\n');
    }

    Some(CapturedPlan {
        title: Some(String::from("Codex Plan")),
        content: content.trim_end().to_owned(),
    })
}

fn update_plan_step(item: &Value) -> Option<(&str, &str)> {
    let step = item.get("step").and_then(Value::as_str)?.trim();
    if step.is_empty() {
        return None;
    }
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|status| !status.is_empty())
        .unwrap_or("pending");
    Some((status, step))
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use crate::git::GitContext;
    use crate::store::AgentPlanState;

    use super::import_codex_history;

    fn json_line(value: &serde_json::Value) -> String {
        serde_json::to_string(value).expect("serialize jsonl event")
    }

    fn session_meta_line(cwd: &Path, branch: &str) -> String {
        json_line(&json!({
            "type": "session_meta",
            "payload": {
                "id": "session",
                "cwd": cwd.to_string_lossy().into_owned(),
                "git": {
                    "branch": branch,
                    "repository_url": "https://github.com/example/repo.git"
                }
            }
        }))
    }

    fn message_line(role: &str, content_type: &str, text: &str) -> String {
        json_line(&json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": role,
                "content": [{
                    "type": content_type,
                    "text": text
                }]
            }
        }))
    }

    fn task_complete_line(timestamp: &str, text: &str) -> String {
        json_line(&json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "last_agent_message": text
            }
        }))
    }

    fn update_plan_line(timestamp: &str) -> String {
        json_line(&json!({
            "timestamp": timestamp,
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "update_plan",
                "arguments": serde_json::to_string(&json!({
                    "explanation": "Structured Codex planning event.",
                    "plan": [
                        {
                            "step": "Inspect failing import output",
                            "status": "completed"
                        },
                        {
                            "step": "Import structured plan calls",
                            "status": "in_progress"
                        },
                        {
                            "step": "Run regression tests",
                            "status": "pending"
                        }
                    ]
                })).expect("serialize update_plan arguments"),
                "call_id": "call-plan"
            }
        }))
    }

    fn write_jsonl(path: &Path, lines: &[String]) {
        fs::write(path, format!("{}\n", lines.join("\n"))).expect("write session");
    }

    #[test]
    fn test_jsonl_helpers_escape_windows_paths() {
        let line = session_meta_line(Path::new(r"C:\Users\dev\repo"), "feature/test");
        let event = serde_json::from_str::<serde_json::Value>(&line).expect("parse session meta");

        assert_eq!(
            event
                .get("payload")
                .and_then(|payload| payload.get("cwd"))
                .and_then(serde_json::Value::as_str),
            Some(r"C:\Users\dev\repo")
        );
    }

    #[test]
    fn imports_marked_assistant_plans_and_skips_duplicates() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let codex_home = temp_dir.path().join("codex");
        let session_dir = codex_home.join("sessions/2026/05/31");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        let session = session_dir.join("rollout-2026-05-31T00-00-00-session.jsonl");
        write_jsonl(
            &session,
            &[
                session_meta_line(&repo_root, "feature/test"),
                message_line(
                    "user",
                    "input_text",
                    "<proposed_plan>ignore user text</proposed_plan>",
                ),
                message_line("assistant", "output_text", "not a marked plan"),
                message_line(
                    "assistant",
                    "output_text",
                    "<proposed_plan>\n# Backfill\n\n- Import old plans\n- Keep api_key=secret-value private\n</proposed_plan>",
                ),
                message_line(
                    "assistant",
                    "output_text",
                    "<proposed_plan>\n# Backfill\n\n- Import old plans\n- Keep api_key=secret-value private\n</proposed_plan>",
                ),
            ],
        );

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

        write_jsonl(
            &session_dir.join("rollout-2026-05-31T00-00-00-other.jsonl"),
            &[
                session_meta_line(&repo_root, "feature/other"),
                message_line(
                    "assistant",
                    "output_text",
                    "<proposed_plan>\n# Other\n\n- Wrong branch\n</proposed_plan>",
                ),
            ],
        );

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

        write_jsonl(
            &session_dir.join("rollout-2026-05-31T00-00-00-complete.jsonl"),
            &[
                session_meta_line(&repo_root, "feature/test"),
                task_complete_line(
                    "2026-05-31T12:34:56Z",
                    "<proposed_plan>\n# Complete\n\n- Import completion text\n</proposed_plan>",
                ),
            ],
        );

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
    fn imports_structured_update_plan_function_calls() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let codex_home = temp_dir.path().join("codex");
        let session_dir = codex_home.join("sessions/2026/06/17");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        write_jsonl(
            &session_dir.join("rollout-2026-06-17T12-00-00-plan.jsonl"),
            &[
                session_meta_line(&repo_root, "feature/test"),
                update_plan_line("2026-06-17T12:34:56Z"),
            ],
        );

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
        assert_eq!(outcome.plans_found, 1);
        assert_eq!(outcome.plans_added, 1);
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].title.as_deref(), Some("Codex Plan"));
        assert!(state.items[0]
            .content
            .contains("Structured Codex planning event."));
        assert!(state.items[0]
            .content
            .contains("- completed: Inspect failing import output"));
        assert!(state.items[0]
            .content
            .contains("- in_progress: Import structured plan calls"));
        assert!(state.items[0]
            .content
            .contains("- pending: Run regression tests"));
        assert_eq!(state.items[0].created_at, "2026-06-17T12:34:56Z");
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

    #[test]
    fn skips_rendered_plan_stack_blocks() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let codex_home = temp_dir.path().join("codex");
        let session_dir = codex_home.join("sessions/2026/05/31");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        write_jsonl(
            &session_dir.join("rollout-2026-05-31T00-00-00-rendered.jsonl"),
            &[
                session_meta_line(&repo_root, "feature/test"),
                message_line(
                    "assistant",
                    "output_text",
                    "<proposed_plan>\n<!-- plan-to-git:start -->\n## Agent Plan Stack\n<!-- plan-to-git:end -->\n</proposed_plan>",
                ),
            ],
        );

        let context = GitContext {
            repo_root,
            repo_slug: Some("example/repo".to_owned()),
            branch: Some("feature/test".to_owned()),
            head_sha: Some("abcdef".to_owned()),
        };
        let mut state = AgentPlanState::default();

        let outcome = import_codex_history(&codex_home, &context, &mut state).expect("import");

        assert_eq!(outcome.plans_found, 1);
        assert_eq!(outcome.rendered_stacks_skipped, 1);
        assert!(state.items.is_empty());
    }
}

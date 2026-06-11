use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::git::GitContext;
use crate::history::{
    collect_jsonl_files, line_turn_id, looks_like_rendered_plan_stack, session_id_from_path,
    HistoryImportOutcome,
};
use crate::normalize::{extract_marked_plans, CapturedPlan};
use crate::store::{AgentPlanState, AgentSource, NewPlanItem};

pub fn import_claude_history(
    claude_home: &Path,
    context: &GitContext,
    state: &mut AgentPlanState,
) -> AppResult<HistoryImportOutcome> {
    let mut outcome = HistoryImportOutcome::default();
    let mut files = claude_project_files(claude_home)?;
    files.sort();

    for path in files {
        outcome.files_scanned += 1;
        import_session_file(&path, claude_home, context, state, &mut outcome)?;
    }

    Ok(outcome)
}

fn import_session_file(
    path: &Path,
    claude_home: &Path,
    context: &GitContext,
    state: &mut AgentPlanState,
    outcome: &mut HistoryImportOutcome,
) -> AppResult<()> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut file_matches = false;
    let mut imported_plan_paths = HashSet::new();

    for (line_index, line) in reader.lines().enumerate() {
        outcome.lines_scanned += 1;
        let line = line?;
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            outcome.parse_errors += 1;
            continue;
        };

        if !event_matches_context(&event, context) {
            continue;
        }
        if !file_matches {
            file_matches = true;
            outcome.files_matched += 1;
        }

        let session_id = event
            .get("sessionId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| session_id_from_path(path));
        let turn_id = event
            .get("uuid")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| line_turn_id(path, line_index + 1));
        let created_at = event
            .get("timestamp")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        if let Some(message) = assistant_message_text(&event) {
            for plan in extract_marked_plans(&message) {
                import_plan(
                    plan,
                    context,
                    state,
                    outcome,
                    session_id.clone(),
                    turn_id.clone(),
                    created_at.clone(),
                );
            }
        }

        if let Some(plan) = claude_plan_mode_plan(&event) {
            if let Some(plan_path) = claude_plan_mode_path(&event) {
                imported_plan_paths.insert(plan_path);
            }
            import_plan(
                plan,
                context,
                state,
                outcome,
                session_id.clone(),
                turn_id.clone(),
                created_at.clone(),
            );
        } else if let Some(plan_path) = claude_plan_mode_path(&event) {
            if imported_plan_paths.insert(plan_path.clone()) {
                if let Some(plan) = claude_plan_mode_file(&plan_path, claude_home) {
                    import_plan(
                        plan,
                        context,
                        state,
                        outcome,
                        session_id.clone(),
                        turn_id.clone(),
                        created_at.clone(),
                    );
                }
            }
        }
    }

    Ok(())
}

fn import_plan(
    plan: CapturedPlan,
    context: &GitContext,
    state: &mut AgentPlanState,
    outcome: &mut HistoryImportOutcome,
    session_id: Option<String>,
    turn_id: Option<String>,
    created_at: Option<String>,
) {
    outcome.plans_found += 1;
    if looks_like_rendered_plan_stack(&plan.content) {
        outcome.rendered_stacks_skipped += 1;
        return;
    }
    let added = state.add_plan(NewPlanItem {
        source: AgentSource::Claude,
        title: plan.title,
        content: plan.content,
        branch: context.branch.clone(),
        head_sha: context.head_sha.clone(),
        session_id,
        turn_id,
        created_at,
    });

    if added {
        outcome.plans_added += 1;
    } else {
        outcome.duplicates += 1;
    }
}

fn claude_project_files(claude_home: &Path) -> AppResult<Vec<PathBuf>> {
    let projects_dir = claude_home.join("projects");
    if !projects_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_jsonl_files(&projects_dir, &mut files)?;
    Ok(files)
}

fn event_matches_context(event: &Value, context: &GitContext) -> bool {
    let Some(cwd) = event.get("cwd").and_then(Value::as_str).map(PathBuf::from) else {
        return false;
    };
    if !cwd.starts_with(&context.repo_root) {
        return false;
    }

    match (
        context.branch.as_deref(),
        event.get("gitBranch").and_then(Value::as_str),
    ) {
        (Some(current), Some(history)) => current == history,
        _ => true,
    }
}

fn assistant_message_text(event: &Value) -> Option<String> {
    if event.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let message = event.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    content_text(message.get("content")?)
}

fn content_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return (!text.trim().is_empty()).then_some(text.to_owned());
    }

    let text = content
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");

    (!text.trim().is_empty()).then_some(text)
}

fn claude_plan_mode_plan(event: &Value) -> Option<CapturedPlan> {
    event
        .get("toolUseResult")
        .and_then(|result| result.get("plan"))
        .and_then(Value::as_str)
        .and_then(captured_plan_from_markdown)
}

fn claude_plan_mode_path(event: &Value) -> Option<PathBuf> {
    event
        .get("toolUseResult")
        .and_then(|result| result.get("filePath"))
        .and_then(Value::as_str)
        .or_else(|| {
            (event
                .get("attachment")
                .and_then(|attachment| attachment.get("type"))
                .and_then(Value::as_str)
                == Some("plan_mode_exit"))
            .then(|| {
                event
                    .get("attachment")
                    .and_then(|attachment| attachment.get("planFilePath"))
                    .and_then(Value::as_str)
            })
            .flatten()
        })
        .map(PathBuf::from)
}

fn claude_plan_mode_file(path: &Path, claude_home: &Path) -> Option<CapturedPlan> {
    if !path.starts_with(claude_home) {
        return None;
    }

    fs::read_to_string(path)
        .ok()
        .and_then(|content| captured_plan_from_markdown(&content))
}

fn captured_plan_from_markdown(content: &str) -> Option<CapturedPlan> {
    let content = content.trim();
    if content.is_empty() {
        return None;
    }

    Some(CapturedPlan {
        title: markdown_title(content),
        content: content.to_owned(),
    })
}

fn markdown_title(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| line.trim().strip_prefix("# "))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use crate::git::GitContext;
    use crate::store::{AgentPlanState, AgentSource};

    use super::import_claude_history;

    fn json_line(value: &serde_json::Value) -> String {
        serde_json::to_string(value).expect("serialize jsonl event")
    }

    fn assistant_line(cwd: &Path, branch: Option<&str>, uuid: &str, text: &str) -> String {
        let mut event = json!({
            "type": "assistant",
            "uuid": uuid,
            "sessionId": "session",
            "cwd": cwd.to_string_lossy().into_owned(),
            "timestamp": "2026-06-11T12:34:56Z",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": text }]
            }
        });
        if let Some(branch) = branch {
            event["gitBranch"] = json!(branch);
        }
        json_line(&event)
    }

    fn user_line(cwd: &Path, text: &str) -> String {
        json_line(&json!({
            "type": "user",
            "uuid": "user-turn",
            "sessionId": "session",
            "cwd": cwd.to_string_lossy().into_owned(),
            "message": {
                "role": "user",
                "content": text
            }
        }))
    }

    fn plan_mode_line(
        cwd: &Path,
        branch: Option<&str>,
        uuid: &str,
        plan: &str,
        plan_path: &Path,
    ) -> String {
        let mut event = json!({
            "type": "user",
            "uuid": uuid,
            "sessionId": "session",
            "cwd": cwd.to_string_lossy().into_owned(),
            "timestamp": "2026-06-11T12:34:56Z",
            "toolUseResult": {
                "plan": plan,
                "filePath": plan_path.to_string_lossy().into_owned()
            }
        });
        if let Some(branch) = branch {
            event["gitBranch"] = json!(branch);
        }
        json_line(&event)
    }

    fn plan_mode_exit_line(
        cwd: &Path,
        branch: Option<&str>,
        uuid: &str,
        plan_path: &Path,
    ) -> String {
        let mut event = json!({
            "type": "attachment",
            "uuid": uuid,
            "sessionId": "session",
            "cwd": cwd.to_string_lossy().into_owned(),
            "timestamp": "2026-06-11T12:34:57Z",
            "attachment": {
                "type": "plan_mode_exit",
                "planFilePath": plan_path.to_string_lossy().into_owned(),
                "planExists": true
            }
        });
        if let Some(branch) = branch {
            event["gitBranch"] = json!(branch);
        }
        json_line(&event)
    }

    fn write_jsonl(path: &Path, lines: &[String]) {
        fs::write(path, format!("{}\n", lines.join("\n"))).expect("write session");
    }

    fn context(repo_root: std::path::PathBuf) -> GitContext {
        GitContext {
            repo_root,
            repo_slug: Some("example/repo".to_owned()),
            branch: Some("feature/test".to_owned()),
            head_sha: Some("abcdef".to_owned()),
        }
    }

    #[test]
    fn imports_marked_assistant_plans_and_skips_duplicates() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let claude_home = temp_dir.path().join("claude");
        let session_dir = claude_home.join("projects/-tmp-repo");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        write_jsonl(
            &session_dir.join("session.jsonl"),
            &[
                user_line(&repo_root, "<proposed_plan>ignore user text</proposed_plan>"),
                assistant_line(&repo_root, Some("feature/test"), "turn-1", "not a marked plan"),
                assistant_line(
                    &repo_root,
                    Some("feature/test"),
                    "turn-2",
                    "<proposed_plan>\n# Claude Backfill\n\n- Import old plans\n- Keep api_key=secret-value private\n</proposed_plan>",
                ),
                assistant_line(
                    &repo_root,
                    Some("feature/test"),
                    "turn-3",
                    "<proposed_plan>\n# Claude Backfill\n\n- Import old plans\n- Keep api_key=secret-value private\n</proposed_plan>",
                ),
            ],
        );

        let mut state = AgentPlanState::default();
        let outcome =
            import_claude_history(&claude_home, &context(repo_root), &mut state).expect("import");

        assert_eq!(outcome.files_scanned, 1);
        assert_eq!(outcome.files_matched, 1);
        assert_eq!(outcome.plans_found, 2);
        assert_eq!(outcome.plans_added, 1);
        assert_eq!(outcome.duplicates, 1);
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].source, AgentSource::Claude);
        assert!(state.items[0].content.contains("Import old plans"));
        assert!(state.items[0].content.contains("api_key=[REDACTED]"));
        assert!(!state.items[0].content.contains("secret-value"));
        assert_eq!(state.items[0].turn_id.as_deref(), Some("turn-2"));
    }

    #[test]
    fn imports_claude_plan_mode_artifacts_once() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let claude_home = temp_dir.path().join("claude");
        let session_dir = claude_home.join("projects/-tmp-repo");
        let plan_path = claude_home.join("plans/native-plan.md");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");
        fs::create_dir_all(plan_path.parent().expect("plan parent")).expect("plan dir");
        fs::write(
            &plan_path,
            "# Plan: Claude Plan Mode\n\n- Capture native plan files\n",
        )
        .expect("write plan file");

        write_jsonl(
            &session_dir.join("session.jsonl"),
            &[
                plan_mode_line(
                    &repo_root,
                    Some("feature/test"),
                    "plan-turn",
                    "# Plan: Claude Plan Mode\n\n- Capture native plan files\n",
                    &plan_path,
                ),
                plan_mode_exit_line(&repo_root, Some("feature/test"), "exit-turn", &plan_path),
            ],
        );

        let mut state = AgentPlanState::default();
        let outcome =
            import_claude_history(&claude_home, &context(repo_root), &mut state).expect("import");

        assert_eq!(outcome.files_scanned, 1);
        assert_eq!(outcome.files_matched, 1);
        assert_eq!(outcome.plans_found, 1);
        assert_eq!(outcome.plans_added, 1);
        assert_eq!(outcome.duplicates, 0);
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].source, AgentSource::Claude);
        assert_eq!(
            state.items[0].title.as_deref(),
            Some("Plan: Claude Plan Mode")
        );
        assert!(state.items[0].content.contains("Capture native plan files"));
        assert_eq!(state.items[0].turn_id.as_deref(), Some("plan-turn"));
    }

    #[test]
    fn skips_unrelated_cwd_and_wrong_branch() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let other_root = temp_dir.path().join("other");
        let claude_home = temp_dir.path().join("claude");
        let session_dir = claude_home.join("projects/-tmp-repo");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&other_root).expect("other root");
        fs::create_dir_all(&session_dir).expect("session dir");

        write_jsonl(
            &session_dir.join("session.jsonl"),
            &[
                assistant_line(
                    &other_root,
                    Some("feature/test"),
                    "turn-1",
                    "<proposed_plan>\n# Other Cwd\n\n- Do not import\n</proposed_plan>",
                ),
                assistant_line(
                    &repo_root,
                    Some("feature/other"),
                    "turn-2",
                    "<proposed_plan>\n# Other Branch\n\n- Do not import\n</proposed_plan>",
                ),
            ],
        );

        let mut state = AgentPlanState::default();
        let outcome =
            import_claude_history(&claude_home, &context(repo_root), &mut state).expect("import");

        assert_eq!(outcome.files_matched, 0);
        assert_eq!(outcome.plans_found, 0);
        assert!(state.items.is_empty());
    }

    #[test]
    fn imports_when_branch_is_missing() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let claude_home = temp_dir.path().join("claude");
        let session_dir = claude_home.join("projects/-tmp-repo");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        write_jsonl(
            &session_dir.join("session.jsonl"),
            &[assistant_line(
                &repo_root,
                None,
                "turn-1",
                "<proposed_plan>\n# No Branch\n\n- Import anyway\n</proposed_plan>",
            )],
        );

        let mut state = AgentPlanState::default();
        let outcome =
            import_claude_history(&claude_home, &context(repo_root), &mut state).expect("import");

        assert_eq!(outcome.plans_added, 1);
        assert!(state.items[0].content.contains("Import anyway"));
    }

    #[test]
    fn skips_rendered_plan_stack_blocks() {
        let temp_dir = tempdir().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let claude_home = temp_dir.path().join("claude");
        let session_dir = claude_home.join("projects/-tmp-repo");
        fs::create_dir_all(&repo_root).expect("repo root");
        fs::create_dir_all(&session_dir).expect("session dir");

        write_jsonl(
            &session_dir.join("session.jsonl"),
            &[assistant_line(
                &repo_root,
                Some("feature/test"),
                "turn-1",
                "<proposed_plan>\n<!-- plan-to-git:start -->\n## Agent Plan Stack\n<!-- plan-to-git:end -->\n</proposed_plan>",
            )],
        );

        let mut state = AgentPlanState::default();
        let outcome =
            import_claude_history(&claude_home, &context(repo_root), &mut state).expect("import");

        assert_eq!(outcome.plans_found, 1);
        assert_eq!(outcome.rendered_stacks_skipped, 1);
        assert!(state.items.is_empty());
    }
}

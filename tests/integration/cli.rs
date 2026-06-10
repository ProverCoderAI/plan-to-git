#[cfg(unix)]
mod unix {
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use plan_to_git::store::STATE_FILE_NAME;

    #[test]
    fn hook_captures_plan_and_handles_missing_pr() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        let payload = format!(
            r#"{{
                "session_id":"session",
                "cwd":"{}",
                "hook_event_name":"Stop",
                "turn_id":"turn",
                "last_assistant_message":"<proposed_plan>\n# MVP\n\n- Capture plan\n</proposed_plan>"
            }}"#,
            repo_dir.display()
        );

        let mut child = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
            .arg("hook")
            .arg("--source")
            .arg("codex")
            .current_dir(&repo_dir)
            .env("PATH", path_with_fake_bin(&bin_dir))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn plan-to-git");

        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(payload.as_bytes())
            .expect("write payload");

        let output = child.wait_with_output().expect("wait");
        assert!(output.status.success());
        assert!(output.stdout.is_empty());

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("Capture plan"));
    }

    #[test]
    fn hook_accepts_current_codex_last_agent_message() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn",
                    "last_agent_message":"<proposed_plan>\n# Current Codex\n\n- Capture current payload\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("Capture current payload"));
    }

    #[test]
    fn hook_accepts_proposed_plan_title_attribute() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn",
                    "last_agent_message":"<proposed_plan title=\"Attribute Hook Plan\">\n- Capture title attribute\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("Attribute Hook Plan"));
        assert!(state.contains("# Attribute Hook Plan"));
        assert!(state.contains("Capture title attribute"));
    }

    #[test]
    fn hook_normalizes_xml_plan_sections() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn",
                    "last_agent_message":"<proposed_plan title=\"XML Plan\">\n  <summary>\n    Verify production capture.\n  </summary>\n\n  <test_plan>\n    1. Check GitHub comment.\n  </test_plan>\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("# XML Plan"));
        assert!(state.contains("## Summary"));
        assert!(state.contains("Verify production capture."));
        assert!(state.contains("## Test Plan"));
        assert!(state.contains("1. Check GitHub comment."));
        assert!(!state.contains("<summary>"));
        assert!(!state.contains("<test_plan>"));
    }

    #[test]
    fn hook_records_question_answer_decision() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn-1",
                    "last_assistant_message":"Should sync be automatic?"
                }}"#,
                repo_dir.display()
            ),
        );

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"UserPromptSubmit",
                    "turn_id":"turn-2",
                    "prompt":"Yes, sync automatically when a PR exists."
                }}"#,
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("Should sync be automatic?"));
        assert!(state.contains("Yes, sync automatically"));
        assert!(state.contains("\"pending_questions\": []"));
    }

    #[test]
    fn hook_posts_open_pr_comment_through_gh_api() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let captured_request = temp_dir.path().join("request.json");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_open_pr(&bin_dir, &captured_request);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn",
                    "last_assistant_message":"<proposed_plan>\n# MVP\n\n- Post PR comment\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        let request = fs::read_to_string(captured_request).expect("captured request");
        assert!(request.contains("Agent Plan Update"));
        assert!(request.contains("Post PR comment"));
        assert!(!request.contains("Original PR body"));

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("\"posted_comments\""));
        assert!(state.contains("\"comment_id\": 12345"));
    }

    #[test]
    fn hook_leaves_plans_queued_when_pr_is_merged() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let captured_request = temp_dir.path().join("request.json");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_closed_pr(&bin_dir, "MERGED", &captured_request);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn",
                    "last_assistant_message":"<proposed_plan>\n# Queued\n\n- Wait for an open PR\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("Wait for an open PR"));
        assert!(state.contains("\"posted_comments\": []"));
        assert!(!captured_request.exists());
    }

    #[test]
    fn hook_leaves_plans_queued_when_pr_is_closed() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let captured_request = temp_dir.path().join("request.json");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_closed_pr(&bin_dir, "CLOSED", &captured_request);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn",
                    "last_assistant_message":"<proposed_plan>\n# Queued\n\n- Wait for an open PR\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("Wait for an open PR"));
        assert!(state.contains("\"posted_comments\": []"));
        assert!(!captured_request.exists());
    }

    #[test]
    fn hook_leaves_plans_queued_when_pr_is_draft() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let captured_request = temp_dir.path().join("request.json");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_draft_pr(&bin_dir, &captured_request);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn",
                    "last_assistant_message":"<proposed_plan>\n# Queued\n\n- Wait for a valid PR\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("Wait for a valid PR"));
        assert!(state.contains("\"posted_comments\": []"));
        assert!(!captured_request.exists());
    }

    #[test]
    fn sync_reports_draft_pr_and_does_not_comment() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let captured_request = temp_dir.path().join("request.json");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_draft_pr(&bin_dir, &captured_request);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn",
                    "last_assistant_message":"<proposed_plan>\n# Queued\n\n- Wait for a valid PR\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        let output = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
            .arg("sync")
            .current_dir(&repo_dir)
            .env("PATH", path_with_fake_bin(&bin_dir))
            .output()
            .expect("run sync");

        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).expect("stdout");
        assert!(stdout.contains("pull request #17 is a draft; leaving plan items queued"));
        assert!(!captured_request.exists());
    }

    #[test]
    fn sync_reports_merged_pr_and_does_not_comment() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let captured_request = temp_dir.path().join("request.json");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_closed_pr(&bin_dir, "MERGED", &captured_request);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"turn",
                    "last_assistant_message":"<proposed_plan>\n# Queued\n\n- Wait for an open PR\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        let output = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
            .arg("sync")
            .current_dir(&repo_dir)
            .env("PATH", path_with_fake_bin(&bin_dir))
            .output()
            .expect("run sync");

        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).expect("stdout");
        assert!(stdout.contains("pull request #17 is MERGED; leaving plan items queued"));
        assert!(!captured_request.exists());
    }

    #[test]
    fn import_codex_backfills_history_once_without_syncing() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let codex_home = temp_dir.path().join("codex");
        let session_dir = codex_home.join("sessions/2026/05/31");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        fs::create_dir_all(&session_dir).expect("session dir");
        write_fake_git(&bin_dir, &repo_dir);

        fs::write(
            session_dir.join("rollout-2026-05-31T00-00-00-session.jsonl"),
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"session","cwd":"{}","git":{{"branch":"feature/test","repository_url":"https://github.com/example/repo.git"}}}}}}
{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"<proposed_plan>\n# Archived Plan\n\n- Import archived plan\n</proposed_plan>"}}]}}}}
"#,
                repo_dir.display()
            ),
        )
        .expect("write session");

        let first = run_import_codex(&repo_dir, &bin_dir, &codex_home);
        assert!(first.contains("found 1 plan(s), added 1"));
        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("Import archived plan"));

        let second = run_import_codex(&repo_dir, &bin_dir, &codex_home);
        assert!(second.contains("found 1 plan(s), added 0"));
        assert!(second.contains("skipped 1 duplicate(s)"));
    }

    fn run_hook(repo_dir: &Path, bin_dir: &Path, payload: &str) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
            .arg("hook")
            .arg("--source")
            .arg("codex")
            .current_dir(repo_dir)
            .env("PATH", path_with_fake_bin(bin_dir))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn plan-to-git");

        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(payload.as_bytes())
            .expect("write payload");

        let output = child.wait_with_output().expect("wait");
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
    }

    fn run_import_codex(repo_dir: &Path, bin_dir: &Path, codex_home: &Path) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
            .arg("import-codex")
            .arg("--codex-home")
            .arg(codex_home)
            .arg("--no-sync")
            .current_dir(repo_dir)
            .env("PATH", path_with_fake_bin(bin_dir))
            .output()
            .expect("run import-codex");

        assert!(output.status.success());
        String::from_utf8(output.stdout).expect("stdout")
    }

    fn write_fake_git(bin_dir: &Path, repo_dir: &Path) {
        let script = format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == "-C" ]]; then
  shift 2
fi
case "$*" in
  "rev-parse --show-toplevel")
    printf '%s\n' "{}"
    ;;
  "rev-parse --abbrev-ref HEAD")
    printf '%s\n' "feature/test"
    ;;
  "rev-parse HEAD")
    printf '%s\n' "abcdef1234567890"
    ;;
  "remote get-url origin")
    printf '%s\n' "https://github.com/example/repo.git"
    ;;
  *)
    echo "unexpected git args: $*" >&2
    exit 1
    ;;
esac
"#,
            repo_dir.display()
        );
        write_executable(&bin_dir.join("git"), &script);
    }

    fn write_fake_gh_no_pr(bin_dir: &Path) {
        let script = r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == "pr view --json number,state,url,isDraft" ]]; then
  echo 'no pull requests found for branch "feature/test"' >&2
  exit 1
fi
echo "unexpected gh args: $*" >&2
exit 1
"#;
        write_executable(&bin_dir.join("gh"), script);
    }

    fn write_fake_gh_open_pr(bin_dir: &Path, captured_request: &Path) {
        let script = format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == "pr view --json number,state,url,isDraft" ]]; then
  printf '%s\n' '{{"number":17,"state":"OPEN","url":"https://github.com/example/repo/pull/17"}}'
  exit 0
fi
if [[ "$1 $2 $3" == "api --method POST" && "$4" == "repos/example/repo/issues/17/comments" && "$5" == "--input" ]]; then
  cp "$6" "{}"
  printf '%s\n' '{{"id":12345}}'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            captured_request.display()
        );
        write_executable(&bin_dir.join("gh"), &script);
    }

    fn write_fake_gh_closed_pr(bin_dir: &Path, state: &str, captured_request: &Path) {
        let script = format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == "pr view --json number,state,url,isDraft" ]]; then
  printf '%s\n' '{{"number":17,"state":"{state}","url":"https://github.com/example/repo/pull/17"}}'
  exit 0
fi
if [[ "$1" == "api" ]]; then
  printf '%s\n' "$*" > "{}"
  echo "comment API should not be called for closed PR" >&2
  exit 1
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            captured_request.display()
        );
        write_executable(&bin_dir.join("gh"), &script);
    }

    fn write_fake_gh_draft_pr(bin_dir: &Path, captured_request: &Path) {
        let script = format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == "pr view --json number,state,url,isDraft" ]]; then
  printf '%s\n' '{{"number":17,"state":"OPEN","url":"https://github.com/example/repo/pull/17","isDraft":true}}'
  exit 0
fi
if [[ "$1" == "api" ]]; then
  printf '%s\n' "$*" > "{}"
  echo "comment API should not be called for draft PR" >&2
  exit 1
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
            captured_request.display()
        );
        write_executable(&bin_dir.join("gh"), &script);
    }

    fn write_executable(path: &Path, content: &str) {
        fs::write(path, content).expect("write script");
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("permissions");
    }

    fn path_with_fake_bin(bin_dir: &Path) -> String {
        format!(
            "{}:{}",
            bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }
}

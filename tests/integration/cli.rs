#[cfg(unix)]
mod unix {
    use std::fs;
    use std::io::Write;
    use std::process::{Command, Stdio};

    use crate::support::{
        find_state_files, path_with_fake_bin, run_hook, run_hook_source, run_hook_with_env,
        run_import_claude_from_default, run_import_codex, state_path, write_fake_gh_closed_pr,
        write_fake_gh_draft_pr, write_fake_gh_explicit_open_pr,
        write_fake_gh_explicit_open_pr_for_repo, write_fake_gh_no_pr, write_fake_gh_open_pr,
        write_fake_git,
    };
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
            .env("PLAN_TO_GIT_STATE_PATH", state_path(&repo_dir))
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
    fn hook_writes_state_under_configured_tmp_state_dir() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let state_dir = temp_dir.path().join("plan-to-git-state");
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
                "last_assistant_message":"<proposed_plan>\n# Tmp State\n\n- Store outside repo\n</proposed_plan>"
            }}"#,
            repo_dir.display()
        );

        let mut child = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
            .arg("hook")
            .arg("--source")
            .arg("codex")
            .current_dir(&repo_dir)
            .env("PATH", path_with_fake_bin(&bin_dir))
            .env("PLAN_TO_GIT_STATE_DIR", &state_dir)
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
        assert!(!repo_dir.join(STATE_FILE_NAME).exists());

        let state_files = find_state_files(&state_dir);
        assert_eq!(state_files.len(), 1);
        let state = fs::read_to_string(&state_files[0]).expect("state file");
        assert!(state.contains("Store outside repo"));
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
    fn hook_captures_claude_plan_source() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        run_hook_source(
            &repo_dir,
            &bin_dir,
            "claude",
            &format!(
                r#"{{
                    "session_id":"session",
                    "transcript_path":"{}",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "last_assistant_message":"<proposed_plan>\n# Claude Hook\n\n- Capture Claude plan\n</proposed_plan>"
                }}"#,
                repo_dir.join("transcript.jsonl").display(),
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("\"source\": \"claude\""));
        assert!(state.contains("Capture Claude plan"));
    }

    #[test]
    fn hook_captures_claude_plan_from_transcript_path_fallback() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let transcript_path = temp_dir.path().join("transcript.jsonl");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        fs::write(
            &transcript_path,
            format!(
                r#"{{"type":"user","sessionId":"session","cwd":"{}","message":{{"role":"user","content":"<proposed_plan>\n# User Plan\n\n- Do not capture\n</proposed_plan>"}}}}
{{"type":"assistant","sessionId":"session","cwd":"{}","message":{{"role":"assistant","content":[{{"type":"text","text":"<proposed_plan>\n# Transcript Claude Hook\n\n- Capture transcript fallback\n</proposed_plan>"}}]}}}}
"#,
                repo_dir.display(),
                repo_dir.display()
            ),
        )
        .expect("write transcript");

        run_hook_source(
            &repo_dir,
            &bin_dir,
            "claude",
            &format!(
                r#"{{
                    "session_id":"session",
                    "transcript_path":"{}",
                    "cwd":"{}",
                    "hook_event_name":"Stop"
                }}"#,
                transcript_path.display(),
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("\"source\": \"claude\""));
        assert!(state.contains("Capture transcript fallback"));
        assert!(!state.contains("Do not capture"));
    }

    #[test]
    fn hook_captures_claude_plan_mode_plan_from_transcript_path() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let transcript_path = temp_dir.path().join("transcript.jsonl");
        let plan_path = temp_dir.path().join("plans/native-plan.md");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        fs::create_dir_all(plan_path.parent().expect("plan parent")).expect("plan dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        fs::write(
            &transcript_path,
            format!(
                r##"{{"type":"assistant","sessionId":"session","cwd":"{}","message":{{"role":"assistant","content":[{{"type":"text","text":"No marked plan here."}}]}}}}
{{"type":"user","uuid":"plan-turn","sessionId":"session","cwd":"{}","gitBranch":"feature/test","timestamp":"2026-06-11T07:04:40Z","toolUseResult":{{"plan":"# Plan: Claude Plan Mode\n\n- Capture native plan mode","filePath":"{}"}}}}
"##,
                repo_dir.display(),
                repo_dir.display(),
                plan_path.display()
            ),
        )
        .expect("write transcript");

        run_hook_source(
            &repo_dir,
            &bin_dir,
            "claude",
            &format!(
                r#"{{
                    "session_id":"session",
                    "transcript_path":"{}",
                    "cwd":"{}",
                    "hook_event_name":"Stop"
                }}"#,
                transcript_path.display(),
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("\"source\": \"claude\""));
        assert!(state.contains("Claude Plan Mode"));
        assert!(state.contains("Capture native plan mode"));
    }

    #[test]
    fn hook_records_claude_question_answer_decision() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        run_hook_source(
            &repo_dir,
            &bin_dir,
            "claude",
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "last_assistant_message":"Should Claude sync plans?"
                }}"#,
                repo_dir.display()
            ),
        );

        run_hook_source(
            &repo_dir,
            &bin_dir,
            "claude",
            &format!(
                r#"{{
                    "session_id":"session",
                    "cwd":"{}",
                    "hook_event_name":"UserPromptSubmit",
                    "prompt":"Yes, capture Claude planning decisions."
                }}"#,
                repo_dir.display()
            ),
        );

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("\"source\": \"claude\""));
        assert!(state.contains("Should Claude sync plans?"));
        assert!(state.contains("Yes, capture Claude planning decisions."));
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
    fn sync_explicit_pr_posts_unposted_items_from_all_sources() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let captured_request = temp_dir.path().join("request.json");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        run_hook(
            &repo_dir,
            &bin_dir,
            &format!(
                r#"{{
                    "session_id":"codex-session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"codex-turn",
                    "last_assistant_message":"<proposed_plan>\n# Codex Plan\n\n- Sync Codex item\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        run_hook_source(
            &repo_dir,
            &bin_dir,
            "claude",
            &format!(
                r#"{{
                    "session_id":"claude-session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "last_assistant_message":"<proposed_plan>\n# Claude Plan\n\n- Sync Claude item\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
        );

        write_fake_gh_explicit_open_pr(&bin_dir, &captured_request);

        let output = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
            .arg("sync")
            .arg("--pr")
            .arg("17")
            .current_dir(&repo_dir)
            .env("PATH", path_with_fake_bin(&bin_dir))
            .env("PLAN_TO_GIT_STATE_PATH", state_path(&repo_dir))
            .output()
            .expect("run sync");

        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).expect("stdout");
        assert!(stdout.contains("posted 2 plan item(s) to pull request #17 comment #12345"));

        let request = fs::read_to_string(captured_request).expect("captured request");
        assert!(request.contains("Source: codex"));
        assert!(request.contains("Sync Codex item"));
        assert!(request.contains("Source: claude"));
        assert!(request.contains("Sync Claude item"));

        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("\"comment_id\": 12345"));
    }

    #[test]
    fn sync_explicit_pr_uses_explicit_repo_context() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let state_dir = temp_dir.path().join("state");
        let captured_request = temp_dir.path().join("request.json");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        write_fake_git(&bin_dir, &repo_dir);
        write_fake_gh_no_pr(&bin_dir);

        run_hook_with_env(
            &repo_dir,
            &bin_dir,
            "codex",
            &format!(
                r#"{{
                    "session_id":"codex-session",
                    "cwd":"{}",
                    "hook_event_name":"Stop",
                    "turn_id":"codex-turn",
                    "last_assistant_message":"<proposed_plan>\n# Upstream Plan\n\n- Sync to upstream\n</proposed_plan>"
                }}"#,
                repo_dir.display()
            ),
            &[
                ("PLAN_TO_GIT_REPO", "upstream/repo"),
                (
                    "PLAN_TO_GIT_STATE_DIR",
                    state_dir.to_str().expect("state dir"),
                ),
            ],
        );

        write_fake_gh_explicit_open_pr_for_repo(&bin_dir, "upstream/repo", &captured_request);

        let output = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
            .arg("--repo")
            .arg("upstream/repo")
            .arg("sync")
            .arg("--pr")
            .arg("17")
            .current_dir(&repo_dir)
            .env("PATH", path_with_fake_bin(&bin_dir))
            .env("PLAN_TO_GIT_STATE_DIR", &state_dir)
            .output()
            .expect("run sync");

        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).expect("stdout");
        assert!(stdout.contains("posted 1 plan item(s) to pull request #17 comment #12345"));

        let request = fs::read_to_string(captured_request).expect("captured request");
        assert!(request.contains("Sync to upstream"));

        let state_files = find_state_files(&state_dir);
        assert_eq!(state_files.len(), 1);
        let state_path = state_files[0].to_string_lossy();
        assert!(state_path.contains("example-repo"));
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
            .env("PLAN_TO_GIT_STATE_PATH", state_path(&repo_dir))
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
            .env("PLAN_TO_GIT_STATE_PATH", state_path(&repo_dir))
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

    #[test]
    fn import_claude_backfills_history_once_from_config_dir_without_syncing() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp_dir.path().join("bin");
        let repo_dir = temp_dir.path().join("repo");
        let claude_home = temp_dir.path().join("claude");
        let session_dir = claude_home.join("projects/-tmp-repo");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        fs::create_dir_all(&session_dir).expect("session dir");
        write_fake_git(&bin_dir, &repo_dir);

        fs::write(
            session_dir.join("session.jsonl"),
            format!(
                r#"{{"type":"user","uuid":"user","sessionId":"session","cwd":"{}","gitBranch":"feature/test","message":{{"role":"user","content":"<proposed_plan>ignore user text</proposed_plan>"}}}}
{{"type":"assistant","uuid":"turn-1","sessionId":"session","cwd":"{}","gitBranch":"feature/test","timestamp":"2026-06-11T12:34:56Z","message":{{"role":"assistant","content":[{{"type":"text","text":"<proposed_plan>\n# Claude Archived Plan\n\n- Import Claude archived plan\n</proposed_plan>"}}]}}}}
{{"type":"assistant","uuid":"turn-2","sessionId":"session","cwd":"{}","gitBranch":"feature/other","message":{{"role":"assistant","content":[{{"type":"text","text":"<proposed_plan>\n# Wrong Branch\n\n- Do not import\n</proposed_plan>"}}]}}}}
"#,
                repo_dir.display(),
                repo_dir.display(),
                repo_dir.display()
            ),
        )
        .expect("write session");

        let first = run_import_claude_from_default(&repo_dir, &bin_dir, &claude_home);
        assert!(first.contains("found 1 plan(s), added 1"));
        let state = fs::read_to_string(repo_dir.join(STATE_FILE_NAME)).expect("state file");
        assert!(state.contains("\"source\": \"claude\""));
        assert!(state.contains("Import Claude archived plan"));
        assert!(!state.contains("Do not import"));

        let second = run_import_claude_from_default(&repo_dir, &bin_dir, &claude_home);
        assert!(second.contains("found 1 plan(s), added 0"));
        assert!(second.contains("skipped 1 duplicate(s)"));
    }
}

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use plan_to_git::store::STATE_FILE_NAME;
use walkdir::WalkDir;

pub fn run_hook(repo_dir: &Path, bin_dir: &Path, payload: &str) {
    run_hook_source(repo_dir, bin_dir, "codex", payload);
}

pub fn run_hook_source(repo_dir: &Path, bin_dir: &Path, source: &str, payload: &str) {
    run_hook_with_env(repo_dir, bin_dir, source, payload, &[]);
}

pub fn run_hook_with_env(
    repo_dir: &Path,
    bin_dir: &Path,
    source: &str,
    payload: &str,
    envs: &[(&str, &str)],
) {
    let has_state_override = envs
        .iter()
        .any(|(key, _)| matches!(*key, "PLAN_TO_GIT_STATE_PATH" | "PLAN_TO_GIT_STATE_DIR"));

    let mut command = Command::new(env!("CARGO_BIN_EXE_plan-to-git"));
    command
        .arg("hook")
        .arg("--source")
        .arg(source)
        .current_dir(repo_dir)
        .env("PATH", path_with_fake_bin(bin_dir))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());

    if !has_state_override {
        command.env("PLAN_TO_GIT_STATE_PATH", state_path(repo_dir));
    }

    for (key, value) in envs {
        command.env(key, value);
    }

    let mut child = command.spawn().expect("spawn plan-to-git");

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

pub fn run_import_codex(repo_dir: &Path, bin_dir: &Path, codex_home: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
        .arg("import-codex")
        .arg("--codex-home")
        .arg(codex_home)
        .arg("--no-sync")
        .current_dir(repo_dir)
        .env("PATH", path_with_fake_bin(bin_dir))
        .env("PLAN_TO_GIT_STATE_PATH", state_path(repo_dir))
        .output()
        .expect("run import-codex");

    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("stdout")
}

pub fn run_import_claude_from_default(
    repo_dir: &Path,
    bin_dir: &Path,
    claude_home: &Path,
) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_plan-to-git"))
        .arg("import-claude")
        .arg("--no-sync")
        .current_dir(repo_dir)
        .env("PATH", path_with_fake_bin(bin_dir))
        .env("CLAUDE_CONFIG_DIR", claude_home)
        .env("PLAN_TO_GIT_STATE_PATH", state_path(repo_dir))
        .output()
        .expect("run import-claude");

    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("stdout")
}

pub fn state_path(repo_dir: &Path) -> PathBuf {
    repo_dir.join(STATE_FILE_NAME)
}

pub fn find_state_files(dir: &Path) -> Vec<PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some(STATE_FILE_NAME))
        .collect()
}

pub fn write_fake_git(bin_dir: &Path, repo_dir: &Path) {
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

pub fn write_fake_gh_no_pr(bin_dir: &Path) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == "pr view --json number,state,url,isDraft" ]]; then
  echo 'no pull requests found for branch "feature/test"' >&2
  exit 1
fi
if [[ "$*" == pr\ view\ --json\ number,state,url,isDraft\ --repo\ * ]]; then
  echo 'no pull requests found for branch "feature/test"' >&2
  exit 1
fi
echo "unexpected gh args: $*" >&2
exit 1
"#;
    write_executable(&bin_dir.join("gh"), script);
}

pub fn write_fake_gh_open_pr(bin_dir: &Path, captured_request: &Path) {
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

pub fn write_fake_gh_explicit_open_pr(bin_dir: &Path, captured_request: &Path) {
    let script = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == "pr view 17 --json number,state,url,isDraft" ]]; then
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

pub fn write_fake_gh_explicit_open_pr_for_repo(
    bin_dir: &Path,
    repo_slug: &str,
    captured_request: &Path,
) {
    let script = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == "pr view 17 --json number,state,url,isDraft --repo {repo_slug}" ]]; then
  printf '%s\n' '{{"number":17,"state":"OPEN","url":"https://github.com/{repo_slug}/pull/17"}}'
  exit 0
fi
if [[ "$1 $2 $3" == "api --method POST" && "$4" == "repos/{repo_slug}/issues/17/comments" && "$5" == "--input" ]]; then
  cp "$6" "{captured_request}"
  printf '%s\n' '{{"id":12345}}'
  exit 0
fi
echo "unexpected gh args: $*" >&2
exit 1
"#,
        repo_slug = repo_slug,
        captured_request = captured_request.display()
    );
    write_executable(&bin_dir.join("gh"), &script);
}

pub fn write_fake_gh_closed_pr(bin_dir: &Path, state: &str, captured_request: &Path) {
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

pub fn write_fake_gh_draft_pr(bin_dir: &Path, captured_request: &Path) {
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

pub fn path_with_fake_bin(bin_dir: &Path) -> String {
    format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

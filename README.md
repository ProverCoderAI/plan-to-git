# plan-to-git

`plan-to-git` captures explicit plans produced by coding agents and posts new pull request comments for plan updates.

The MVP supports Codex and Claude Code:

- reads Codex or Claude Code hook JSON from stdin;
- captures only explicit plan blocks such as `<proposed_plan>...</proposed_plan>`, `<proposed_plan title="...">...</proposed_plan>`, or `## Accepted Plan`;
- stores captured plans and planning Q/A decisions in a per-repository state file;
- posts a new PR comment with newly captured current-branch items when a valid (open, non-draft) PR exists;
- leaves the local stack queued when no valid PR exists yet.

## CLI

```bash
plan-to-git hook --source codex < hook-payload.json
plan-to-git hook --source claude < hook-payload.json
plan-to-git show
plan-to-git render
plan-to-git sync
plan-to-git sync --pr 7
plan-to-git --repo owner/repo sync --pr 7
plan-to-git import-codex --dry-run
plan-to-git import-codex
plan-to-git import-claude --dry-run
plan-to-git import-claude
plan-to-git clear --yes
```

`hook` is intentionally quiet on stdout, because agent hook stdout can be interpreted by the agent. Operational messages go to stderr.

## State Storage

By default, `plan-to-git` stores its state outside the repository under the system temp directory:

```text
/tmp/plan-to-git/<repo-key>/.agent-plan.json
```

Set `PLAN_TO_GIT_STATE_DIR` to choose another state root, or `PLAN_TO_GIT_STATE_PATH` to choose an exact state file path. This keeps hook-generated state from dirtying the working tree while preserving a stable queue for the current repository.

## Codex Hook Example

Add the command to Codex hook configuration for `Stop` and `UserPromptSubmit` events:

```toml
[[hooks.UserPromptSubmit]]
[[hooks.UserPromptSubmit.hooks]]
type = "command"
command = "plan-to-git hook --source codex"

[[hooks.Stop]]
[[hooks.Stop.hooks]]
type = "command"
command = "plan-to-git hook --source codex"
```

Exact hook configuration shape can vary by Codex release. The hook command itself expects the release behavior documented by Codex hooks: `Stop` includes the final agent message (`last_agent_message`, with `last_assistant_message` still accepted for older payloads), and `UserPromptSubmit` includes `prompt`.

## Claude Code Hook Example

Add the command to Claude Code hook configuration for `Stop` and `UserPromptSubmit` events:

```json
{
  "hooks": {
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "plan-to-git hook --source claude"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "plan-to-git hook --source claude"
          }
        ]
      }
    ]
  }
}
```

Claude Code `Stop` hooks provide `last_assistant_message`, `session_id`, `transcript_path`, `cwd`, and `hook_event_name`; `UserPromptSubmit` hooks provide `prompt`. `import-claude` backfills explicitly marked plans and Claude Plan Mode artifacts from `CLAUDE_CONFIG_DIR/projects/**/*.jsonl`, `CLAUDE_HOME/projects/**/*.jsonl`, or `~/.claude/projects/**/*.jsonl`.

If an agent emits known XML-style plan sections (`summary`, `flow`, `test_plan`, or `assumptions`) inside a proposed plan, `plan-to-git` normalizes them to Markdown headings before storage and PR sync.

## Pull Request Comments

When `gh pr view` finds an open, non-draft PR for the current branch, `plan-to-git` creates a new issue comment on that PR containing items that have not been posted before:

```markdown
## Agent Plan Update

...
```

Use `plan-to-git sync --pr 7` to post queued current-branch items to a specific pull request instead of relying on branch-based PR discovery. `sync` is source-agnostic: one run posts all unposted current-branch items in the state file, whether they came from Codex, Claude Code, or another supported agent.

Use `--repo owner/repo` or `PLAN_TO_GIT_REPO=owner/repo` when the local `origin` remote is not the pull request target repository, for example in fork-origin workflows. The explicit repository only selects the GitHub PR/comment target; local state and history matching remain tied to the current checkout.

The PR description is not edited. Closed, merged, or still-draft pull requests are not commented on; new items stay queued until the PR is valid (open and marked ready for review). After a comment is created, the local state file records the posted item hashes and GitHub comment id so repeated `sync`, `hook`, `import-codex`, or `import-claude` runs do not post the same plan again, including on a later PR.

## Safety

The hook path only uses stable hook payload fields, explicitly marked plan text, and Claude Plan Mode transcript artifacts. `import-codex` can backfill previous plans from `~/.codex/sessions`; `import-claude` can backfill from Claude Code transcript files under the active Claude config directory. Both importers only read sessions that match the current repository and branch when branch metadata is available, and they still import only explicit markers such as `<proposed_plan>...</proposed_plan>`, `<proposed_plan title="...">...</proposed_plan>`, `## Accepted Plan`, or Claude Code's native Plan Mode output.

Captured content is redacted before local storage and PR sync. The local state file also acts as the sent-plan registry: content hashes prevent the same plan from being added and commented again.

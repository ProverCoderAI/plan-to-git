# plan-to-git

`plan-to-git` captures explicit plans produced by coding agents and posts new pull request comments for plan updates.

The MVP is Codex-first:

- reads Codex hook JSON from stdin;
- captures only explicit plan blocks such as `<proposed_plan>...</proposed_plan>`, `<proposed_plan title="...">...</proposed_plan>`, or `## Accepted Plan`;
- stores captured plans and planning Q/A decisions in `.agent-plan.json`;
- posts a new PR comment with newly captured current-branch items when an open PR exists;
- leaves the local stack queued when no open PR exists yet.

## CLI

```bash
plan-to-git hook --source codex < hook-payload.json
plan-to-git show
plan-to-git render
plan-to-git sync
plan-to-git import-codex --dry-run
plan-to-git import-codex
plan-to-git clear --yes
```

`hook` is intentionally quiet on stdout, because Codex hook stdout is interpreted by Codex. Operational messages go to stderr.

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

If an agent emits known XML-style plan sections (`summary`, `flow`, `test_plan`, or `assumptions`) inside a proposed plan, `plan-to-git` normalizes them to Markdown headings before storage and PR sync.

## Pull Request Comments

When `gh pr view` finds an open PR for the current branch, `plan-to-git` creates a new issue comment on that PR containing items that have not been posted before:

```markdown
## Agent Plan Update

...
```

The PR description is not edited. Closed or merged pull requests are not commented on; new items stay queued until an open PR exists. After a comment is created, `.agent-plan.json` records the posted item hashes and GitHub comment id so repeated `sync`, `hook`, or `import-codex` runs do not post the same plan again, including on a later PR.

## Safety

The hook path only uses stable hook payload fields and explicitly marked plan text. `import-codex` can backfill previous plans from `~/.codex/sessions`, but it only reads assistant message events from sessions that match the current repository and branch, and it still imports only explicit markers such as `<proposed_plan>...</proposed_plan>`, `<proposed_plan title="...">...</proposed_plan>`, or `## Accepted Plan`.

Captured content is redacted before local storage and PR sync. `.agent-plan.json` also acts as the sent-plan registry: content hashes prevent the same plan from being added and commented again.

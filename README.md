# plan-to-git

`plan-to-git` captures explicit plans produced by coding agents and keeps the current pull request description in sync.

The MVP is Codex-first:

- reads Codex hook JSON from stdin;
- captures only explicit plan blocks such as `<proposed_plan>...</proposed_plan>` or `## Accepted Plan`;
- stores captured plans and planning Q/A decisions in `.agent-plan.json`;
- updates the current branch PR body between stable markers when a PR exists;
- leaves the local stack queued when no PR exists yet.

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

Exact hook configuration shape can vary by Codex release. The hook command itself expects the release behavior documented by Codex hooks: `Stop` includes `last_assistant_message`, and `UserPromptSubmit` includes `prompt`.

## Pull Request Block

When `gh pr view` finds a PR for the current branch, `plan-to-git` inserts or replaces only this section:

```markdown
<!-- plan-to-git:start -->
## Agent Plan Stack

...
<!-- plan-to-git:end -->
```

If only one marker exists, sync fails rather than risking corruption of the human-written PR body.

## Safety

The hook path only uses stable hook payload fields and explicitly marked plan text. `import-codex` can backfill previous plans from `~/.codex/sessions`, but it only reads assistant message events from sessions that match the current repository and branch, and it still imports only explicit markers such as `<proposed_plan>...</proposed_plan>` or `## Accepted Plan`.

Captured content is redacted before local storage and PR sync. `.agent-plan.json` also acts as the sent-plan registry: content hashes prevent the same plan from being added and uploaded again for the same branch.

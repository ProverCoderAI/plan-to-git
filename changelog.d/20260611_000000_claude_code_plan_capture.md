---
bump: minor
---

### Added
- Added Claude Code hook capture and `import-claude` backfill support for explicitly marked plans and native Claude Plan Mode artifacts from Claude transcript files.
- Added temp-directory state storage with `PLAN_TO_GIT_STATE_DIR` and `PLAN_TO_GIT_STATE_PATH` overrides so hook state no longer has to dirty the repository.
- Added `plan-to-git sync --pr <number>` for source-agnostic syncing of queued Codex, Claude Code, and other supported agent plans to an explicit pull request.

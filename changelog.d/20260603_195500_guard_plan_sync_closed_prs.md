---
bump: patch
---

### Fixed
- Kept captured plan updates queued instead of posting comments to closed or merged pull requests.

### Changed
- Accepted current Codex `last_agent_message` hook payloads and normalized known XML-style plan sections before PR sync.

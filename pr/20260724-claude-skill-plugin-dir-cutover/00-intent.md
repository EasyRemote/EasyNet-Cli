# Intent

Goal: remove the Claude Code mission driver's legacy `<cwd>/skills/` plugin discovery path and make `.claude/skills/` the only runtime-owned plugin discovery root for Claude Code workspaces.

Non-goals:

- Do not change skill install, publish, or list APIs.
- Do not migrate old workspace data or preserve a compatibility scan for pre-fix layouts.
- Do not change Codex workspace skill discovery; Codex remains bound to `.agents/skills/`.

Acceptance criteria:

- Claude Code launch args are derived only from the canonical `.claude/skills/` workspace path.
- No production driver comment documents `<cwd>/skills/` as an active fallback.
- A regression test proves `<cwd>/skills/` plugin-shaped directories are ignored while `.claude/skills/` directories are emitted.
- Architecture gates prevent reintroducing the legacy driver discovery path.

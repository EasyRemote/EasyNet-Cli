# Invariants

- Workspace skill discovery must be runtime-owned, not historical-product-owned.
- Claude Code canonical skill discovery root is `<workspace>/.claude/skills/`.
- Codex canonical skill discovery root is `<workspace>/.agents/skills/`.
- `<workspace>/skills/` may remain an agent-private content directory where required by existing public APIs, but it must not be a driver runtime plugin-discovery fallback.
- Launch argument construction must be deterministic from the current workspace layout and must not repair or adopt legacy layouts.

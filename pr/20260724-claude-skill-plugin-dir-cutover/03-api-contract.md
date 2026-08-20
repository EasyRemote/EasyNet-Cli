# API contract

Request/response shape:

- No public request, response, or receipt fields change.
- Claude Code process launch arguments remain internal runtime implementation detail.

Error and tenant rules:

- Missing `.claude/skills/` is a no-op, not an error.
- Malformed or empty plugin candidate subdirectories are ignored as before.
- Legacy `<cwd>/skills/` is not inspected, so it cannot influence process launch or cross a runtime directory boundary.

Compatibility posture:

- Public skill APIs remain source-compatible.
- Internal legacy runtime discovery fallback is removed intentionally.

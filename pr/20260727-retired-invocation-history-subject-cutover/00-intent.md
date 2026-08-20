Goal: close the old invocation-history subject carrier that still appears as `session/invocation_history:*` in runtime authority paths.

Non-goals:
- Do not add compatibility rewriting from old subjects to runtime-state subjects.
- Do not make descriptor resolution probe remote catalogues or signers.
- Do not change the public invocation tuple shape.

Acceptance criteria:
- Core identity owns the retired invocation-history subject predicate.
- Session authority classification cannot treat `session/invocation_history` or `session/invocation_history:*` as live session subjects.
- All-zero principal placeholders still fail as all-zero facts at tuple/public authority boundaries.
- Focused Rust tests and architecture gates pass.

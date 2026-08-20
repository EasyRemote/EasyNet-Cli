# Decisions log

- 2026-07-24: Selected Claude Code legacy skill path discovery as the next seam because codegraph and source comments identified an active runtime fallback from canonical `.claude/skills/` to historical `<cwd>/skills/`.
- 2026-07-24: Removed `<cwd>/skills/` from Claude Code plugin discovery instead of preserving a migration fallback; old workspace data must not influence runtime launch semantics.
- 2026-07-24: Kept public skill APIs unchanged; this cutover only changes the internal driver process-argument assembly boundary.
- 2026-07-24: Added architecture gate coverage so the driver cannot reintroduce `cwd.join("skills")` or pre-cutover compatibility language in production code.

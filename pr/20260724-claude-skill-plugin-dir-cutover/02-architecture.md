# Architecture

Layering:

- Mission workspace preparation owns runtime-specific seed locations.
- Runtime drivers consume prepared workspace runtime directories.
- Skill install/list/publish APIs own agent skill records and managed skill state, not Claude Code runtime plugin discovery.

Boundary decision:

- Claude Code driver scans only `.claude/skills/` for plugin-shaped subdirectories.
- The legacy `<cwd>/skills/` path is no longer part of driver launch semantics.

This removes a product-history compatibility path from the runtime driver without changing public CLI skill command shapes.

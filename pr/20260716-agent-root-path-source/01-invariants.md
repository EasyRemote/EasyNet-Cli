# Invariants

- Fresh agent creation and registry migration may choose the default
  `agents_root()/name` location before the registry row exists.
- After load/migration, missing `root_path` is corrupt registry state.
- Steady-state agent list, ability publish, skill publish, install, upgrade,
  remove, and managed skill inventory code must use `AgentEntry::required_root_path`.
- No production reader may recover a registered agent's root by falling back
  from a missing row field to `agents_root()/name`.
- Tests may construct explicit roots, but they should not depend on implicit
  fallback behavior.

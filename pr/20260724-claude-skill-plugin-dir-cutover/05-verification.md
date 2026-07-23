# Verification

Passed:

- `cargo test plugin_dirs_use_claude_project_skill_root_only --features axon-pb`
- `cargo fmt --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check`

Codegraph evidence:

- `codegraph query append_claude_workspace_plugin_dirs`
- `codegraph callers append_claude_workspace_plugin_dirs`

Result: the helper is called by production `invoke` and by the regression test only.

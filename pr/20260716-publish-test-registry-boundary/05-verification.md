# Verification Plan

```bash
cargo test publish_ --lib
cargo test unpublish_ --lib
cargo test --features axon-pb --lib --no-run 2>&1 | tee target/publish-boundary-no-run.log
rg 'unused import: `crate::daemon::persistence::agent_registry as agents`' target/publish-boundary-no-run.log
git diff --check -- src/daemon/ability/builtins/device_control/ability_management/publish.rs pr/20260716-publish-test-registry-boundary
```

The warning grep is expected to return no matches.

# Results

- `cargo test publish_ --lib`: 32 passed.
- `cargo test unpublish_ --lib`: 7 passed.
- `cargo test --features axon-pb --lib --no-run`: passed.
- `rg 'unused import: `crate::daemon::persistence::agent_registry as agents`'
  target/publish-boundary-no-run.log`: no matches; warning absent.
- `git diff --check -- src/daemon/ability/builtins/device_control/ability_management/publish.rs
  pr/20260716-publish-test-registry-boundary`: clean.

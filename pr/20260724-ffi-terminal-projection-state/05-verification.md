# Verification

Executed:

- `cargo test -q ffi::invocation::tests::`
  - Result: passed; 91 FFI invocation tests passed.
- `cargo fmt --check`
  - Result: passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - Result: passed.
- `tools/scripts/check-architecture-convergence.sh`
  - Result: passed.
- `git diff --check`
  - Result: passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
  - Result: synced changed Rust graph.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
  - Result: index is up to date.

Focused negative evidence:

- Production `src/ffi/invocation/mod.rs` no longer contains `projection["terminal"].as_bool().unwrap_or(false)`.
- SPEC v2 self-test now includes a legacy FFI fixture that reintroduces JSON terminal lookup and confirms the gate fails.

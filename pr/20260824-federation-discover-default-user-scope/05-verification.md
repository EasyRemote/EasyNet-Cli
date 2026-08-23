# Verification

## Passed

- `cargo test --lib --features axon-pb federation_discover_ -- --nocapture`
  - 9 passed.
- `cargo test --lib --features axon-pb cli::commands::federation_discover::tests -- --nocapture`
  - 2 passed.
- `cargo build --bin easynet --features axon-pb`
- `git diff --check`
- Live paired-Device default read:
  - `target/debug/easynet federation discover --json`
  - returned the joined Device with `status: active`.
- Live privilege boundary:
  - `target/debug/easynet federation discover --operator-audit --json`
  - exited 1 before I/O with `requires a local Authority runtime, got device`.
- Live product-online audit:
  - `packaging/release/dev-check-local-runtime.sh --easynet-bin target/debug/easynet --json --wait-online 10`
  - exited 0 with `session_admitted: true`, `directory_status: online`, and
    `true_online: true`.

## Known unrelated gate blocker

`bash tools/scripts/check-architecture-convergence.sh` reports the existing
`R16D_CANONICAL_ENVELOPE_OWNER_FORK` violation in
`src/daemon/invocation/admission/admission_facade.rs`. That file is not changed
by this slice; the added R153 federation scope rules pass.

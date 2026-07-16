# Verification Plan

```bash
cargo test cancel_invocations_for_binding_removes_only_owned_entries --lib
cargo test invocation_stream_cancel_is_idempotent_for_unknown_stream --lib
cargo test invocation_bidi_cancel_is_idempotent_for_unknown_session --lib
cargo check --features axon-pb --lib --all-targets 2>&1 | tee target/ffi-cancel-binding-owner-check.log
rg 'cancel_invocations_for_handle|warning:' target/ffi-cancel-binding-owner-check.log
bash tools/scripts/check-architecture-convergence.sh
git diff --check -- src/ffi/invocation/mod.rs pr/20260716-ffi-cancel-binding-owner
```

The compile log is expected to contain no `cancel_invocations_for_handle`
warning.

## Results

- `cargo test cancel_invocations_for_binding_removes_only_owned_entries --lib`: pass.
- `cargo test invocation_stream_cancel_is_idempotent_for_unknown_stream --lib`: pass.
- `cargo test invocation_bidi_cancel_is_idempotent_for_unknown_session --lib`: pass.
- `cargo check --features axon-pb --lib --all-targets 2>&1 | tee target/ffi-cancel-binding-owner-check.log`: pass.
- Warning grep over `target/ffi-cancel-binding-owner-check.log`: no
  `cancel_invocations_for_handle` or `warning:` matches.
- `bash tools/scripts/check-architecture-convergence.sh`: pass.
- `git diff --check -- src/ffi/invocation/mod.rs pr/20260716-ffi-cancel-binding-owner`: pass.

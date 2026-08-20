Results:

- Added `DESCRIPTOR_OWNER_OFFLINE` typed projection at the shared C-ABI daemon
  error boundary.
- `ffi_daemon_error` now detects daemon status errors that prove descriptor
  owner liveness failure and records canonical last-error JSON:
  `code=DESCRIPTOR_OWNER_OFFLINE`, `stage=routing`, `retry=safe`.
- Generic `Unavailable` status without owner-offline facts remains
  `RUNTIME_OFFLINE`, preserving daemon transport semantics.
- The C ABI integer code remains stable (`ERR_DAEMON_DOWN`) while typed SDK
  bindings can read the canonical runtime state from last-error JSON.
- SPEC v2 gate now checks the FFI owner-offline projection and keeps the
  descriptor resolver remote-probe vocabulary ban scoped to the resolver
  section.

Verification:

- `cargo test native_runtime_owner_offline_status_records_descriptor_owner_offline_projection --lib` passed.
- `cargo test native_runtime_unavailable_without_owner_offline_remains_runtime_offline --lib` passed.
- `cargo test native_runtime_signer_error_records_caller_signer_projection --lib` passed.
- `cargo test daemon_transport_error_records_typed_last_error --lib` passed.
- `cargo test daemon_status_error_records_typed_last_error --lib` passed.
- `cargo fmt --check` passed.
- `git diff --check` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` passed.

Codegraph evidence:

- `codegraph callers -p . ffi_status_code_to_error` showed the only caller is
  `ffi_code_for_daemon_error`.
- `codegraph callers -p . ffi_daemon_error` showed the shared unary, signed
  submit, stream, and bidi C-ABI entrypoints all pass through this boundary.

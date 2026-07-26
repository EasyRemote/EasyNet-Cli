# Verification

Executed checks:

- `cargo test remote_failure`
- `cargo test native_runtime_owner_offline_status_records_descriptor_owner_offline_projection`
- `cargo test native_runtime_signer_error_records_caller_signer_projection`
- `cargo fmt --check`
- `git diff --check`
- `check-canonical-runtime-convergence-v2.sh`
- `check-architecture-convergence.sh`
- `codegraph sync .`
- `codegraph status .`

All executed checks passed.

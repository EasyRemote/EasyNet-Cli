Verification checklist:

- `cargo test native_runtime_owner_offline_status_records_descriptor_owner_offline_projection --lib`
- `cargo test daemon_transport_error_records_typed_last_error --lib`
- `cargo fmt --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check`

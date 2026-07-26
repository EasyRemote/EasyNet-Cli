# Verification

Passed:
- `cargo test native_runtime_owner_offline_status_records_descriptor_owner_offline_projection --lib`
- `cargo test native_runtime_signer_error_records_caller_signer_projection --lib`
- `cargo test projects_descriptor_owner_offline_without_adapter_message_parsing --lib`
- `cargo test remote_failure --lib`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check && git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync . && /Users/macbook.silan.tech/.local/bin/codegraph status .`

Verification matrix
===================

Planned checks:

- `cargo test runtime_descriptor_resolver_uses_explicit_provider_for_remote_receipt_read --lib`
- `cargo test runtime_descriptor_resolver_rejects_receipt_provider_device_subject --lib`
- `cargo test runtime_descriptor_resolver_rejects_receipt_provider_missing_subject --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Evidence:

- `codegraph explore -p . --max-files 12 SessionHistoryOperations RuntimeStateReadSubjectURA runtime_state_read_subject_ura validate_receipt_history_request invocation.history.list receipt_history`
  identified that Go/Python SDK receipt-history providers already enforce the
  runtime-state read subject guard.
- `cargo test runtime_descriptor_resolver_uses_explicit_provider_for_remote_receipt_read --lib`
  passed.
- `cargo test runtime_descriptor_resolver_rejects_receipt_provider_non_runtime_state_subjects --lib`
  passed.
- `cargo test runtime_descriptor_resolver_rejects_provider_ability_family_mismatch --lib`
  passed.
- `cargo test runtime_descriptor_resolver_ --lib` passed: 11 passed, 0 failed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` now asserts that the
  FFI descriptor resolver calls `provider.validate_request_subject(object)?`,
  owns `validate_receipt_history_descriptor_subject`, and has the receipt
  provider negative test.
- `cargo fmt --check` passed.
- `git diff --check` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.

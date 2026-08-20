# Verification

Executed checks:

```sh
bash tools/scripts/check-ura-construction.sh
cargo test -p easynet first_backoff_defers_one_complete_scheduled_drain
cargo test -p easynet owner_cursor_transactions_preserve_concurrent_process_writers
cargo test -p easynet prepared_recovery_applies_once_and_persists_exact_outcome
cargo test -p easynet admission_explain_projects_voice_actions_from_signed_descriptor_facts
cargo test -p easynet child_is_receipt_anchored_and_inherits_subject_trace_and_parent_deadline
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
cargo fmt --check
```

Result: all passed.

```sh
git diff --cached --check
```

Result: run after staging.

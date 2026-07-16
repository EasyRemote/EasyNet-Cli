# Verification

Planned commands:

```text
cargo test -p easynet first_backoff_defers_one_complete_scheduled_drain
cargo test -p easynet publication_retry_budget_resets_at_stage_progress
cargo test -p easynet finite_budget_enters_reconciliation_and_manual_retry_retains_evidence
cargo test -p easynet authorized_reconciliation_is_idempotent_audited_and_conflict_closed
cargo test -p easynet delayed_old_revoke_preserves_new_same_ura_incarnation_everywhere
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
git diff --cached --check
```

The focused tests cover success, failure, recovery, and ABA/replay behavior
around the purge publication and revoke state machines.

Actual results:

```text
PASS cargo test -p easynet first_backoff_defers_one_complete_scheduled_drain
PASS cargo test -p easynet publication_retry_budget_resets_at_stage_progress
PASS cargo test -p easynet finite_budget_enters_reconciliation_and_manual_retry_retains_evidence
PASS cargo test -p easynet authorized_reconciliation_is_idempotent_audited_and_conflict_closed
PASS cargo test -p easynet delayed_old_revoke_preserves_new_same_ura_incarnation_everywhere
PASS bash tools/scripts/check-architecture-convergence.sh
PASS bash tests/scripts/test_check_architecture_convergence.sh
PASS git diff --cached --check
```

Cargo emitted existing warnings about unused imports/constants and dead code in
unrelated generated/mission/FFI surfaces; no warning is introduced by this
slice.

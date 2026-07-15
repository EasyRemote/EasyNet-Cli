# Verification

Planned:

- `cargo test --features axon-pb agent_lifecycle --lib`
- `cargo test --features axon-pb daemon::ability::builtins::agents::lifecycle::tests::start_agent_authority_failure_rolls_back_all_local_segments --lib -- --exact`
- `cargo test --features axon-pb daemon::ability::builtins::agents::lifecycle::tests::stop_agent_revokes_authority_and_runtime_rows --lib -- --exact`
- `cargo test --features axon-pb daemon::ability::builtins::agents::lifecycle::tests::purge_agent_deletes_only_the_registered_agent_root --lib -- --exact`
- `tests/scripts/test_check_architecture_convergence.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

Completed:

- PASS: `rustfmt --edition 2021 --check src/daemon/ability/builtins/agents/lifecycle.rs`
- PASS: `cargo test --features axon-pb agent_lifecycle --lib`
- PASS: `cargo test --features axon-pb daemon::ability::builtins::agents::lifecycle::tests::start_agent_authority_failure_rolls_back_all_local_segments --lib -- --exact`
- PASS: `cargo test --features axon-pb daemon::ability::builtins::agents::lifecycle::tests::stop_agent_revokes_authority_and_runtime_rows --lib -- --exact`
- PASS: `cargo test --features axon-pb daemon::ability::builtins::agents::lifecycle::tests::purge_agent_deletes_only_the_registered_agent_root --lib -- --exact`
- PASS: `tests/scripts/test_check_architecture_convergence.sh`
- PASS: `tools/scripts/check-architecture-convergence.sh`
- PASS: `git diff --check -- src/daemon/ability/builtins/agents/lifecycle.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/2026-07-16-agent-lifecycle-projection-store/00-intent.md pr/2026-07-16-agent-lifecycle-projection-store/01-invariants.md pr/2026-07-16-agent-lifecycle-projection-store/02-verification.md`

Known existing failure outside this slice:

- `cargo test --features axon-pb daemon::ability::builtins::agents::lifecycle::tests:: --lib`
  fails two publication poison/outbox count tests:
  `backed_off_revoke_poison_does_not_block_later_purge_publication` and
  `restart_redrive_isolates_poisoned_transactions_and_preserves_retry_evidence`.
  The same two exact tests fail in a clean detached HEAD worktree, so they are
  not introduced by the projection-store refactor.

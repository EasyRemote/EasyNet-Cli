Verification
============

Planned checks
--------------

- `cargo test daemon::federation::read_model::authority_published_abilities --lib`
- `cargo test daemon::ability::builtins::governance::meta --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Results
-------

- `cargo test daemon::federation::read_model::authority_published_abilities --lib`
  - Passed: 8 tests.
- `cargo test daemon::ability::builtins::governance::meta --lib`
  - Passed: 23 tests.
- `cargo test daemon::invocation::bidi::session_initiator::prelude --lib`
  - Passed: 13 tests.
- `cargo test daemon::invocation::bidi::session_initiator::heartbeat --lib`
  - Passed: 6 tests.
- `cargo check --bin easynet-daemon`
  - Passed with an existing `owner_projection::AbilityCallableSummary::new`
    dead-code warning.
- `cargo fmt --check`
  - Passed.
- `git diff --check`
  - Passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Passed: `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Passed: `canonical-runtime-convergence-v2: OK`.

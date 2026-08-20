Verification
============

Planned checks
--------------

- `cargo test descriptors:: --lib`
- `cargo test daemon::ability::control_plane --lib`
- `cargo test daemon::federation::read_model::owner_projection --lib`
- `cargo test daemon::federation::advertise --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Results
-------

- `cargo test descriptors:: --lib`: passed, 47 tests.
- `cargo test daemon::ability::control_plane --lib`: passed, 13 tests.
- `cargo test daemon::federation::read_model::owner_projection --lib`: passed,
  22 tests.
- `cargo test daemon::federation::advertise --lib`: passed, 1 test.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.

Failure-path evidence
---------------------

The first compile attempt failed because `AbilityCallableSummary` derived
`Default` through a `CallMode` field. The fix removed that read-model default
and the serde fallback for missing `callable_summary` instead of restoring a
default RPC mode.

Verification
============

Planned checks
--------------

- `cargo test daemon::ability::builtins::governance::meta --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Results
-------

- `cargo test daemon::ability::builtins::governance::meta --lib`
  - Passed: 23 tests.
- `cargo fmt --check`
  - Passed.
- `git diff --check`
  - Passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Passed: `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Passed: `canonical-runtime-convergence-v2: OK`.

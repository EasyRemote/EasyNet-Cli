Verification
============

Planned checks
--------------

- `cargo test pathconstraints --lib`
- targeted shell.run pathconstraint tests if needed
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Results
-------

- `cargo test pathconstraints --lib`: passed, 20 tests.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.

Gate update
-----------

Added a SPEC v2 static contract that requires `PathVerdict::InvalidTarget`,
fallible normalization helpers, and the empty-target regression tests while
rejecting the retired root-substitution fallback.

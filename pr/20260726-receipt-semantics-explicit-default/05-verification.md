Verification
============

Planned checks
--------------

- `cargo test daemon::ability::descriptors::surface --lib`
- `cargo test daemon::ability::builtins::resources::media::abilities --lib`
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`

Results
-------

- PASS: `cargo test daemon::ability::builtins::resources::media::abilities --lib`
  after aligning canonical voice descriptor TOMLs with the media `ABILITIES`
  table.
- PASS: `cargo test daemon::ability::descriptors::surface --lib`
  (`43 passed`).
- PASS: `cargo fmt --check`.
- PASS: `git diff --check`.
- PASS: `bash tools/scripts/check-architecture-convergence.sh`
  (`architecture-convergence: OK`).
- PASS: `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  (`canonical-runtime-convergence-v2: OK`).

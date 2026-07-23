# Verification

- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `bash tests/scripts/test_check_runtime_state_read_subject_boundary.sh`
- `cargo fmt --check`
- `git diff --check`
- `cargo test -q current_user --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
- `/Users/macbook.silan.tech/.local/bin/codegraph status`

Known non-blocking verification signal:

- `cargo test -q skill --features axon-pb` currently fails 11 daemon
  `real_invoke_tests::*skill*` cases because the tests require local Device
  authority from credentials and the current test process has no paired
  credentials. This is a separate hermetic authority-fixture defect in the
  daemon real-invoke test layer, not a regression of this CLI read cutover.

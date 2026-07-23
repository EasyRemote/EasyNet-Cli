# Verification

Passed:

- `cargo fmt --check`
- `git diff --check`
- `cargo test -q governance::access_control::tests --features axon-pb`
- `cargo test -q governance::network_health::tests --features axon-pb`
- `cargo test -q governance::admin_status::tests --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
- `/Users/macbook.silan.tech/.local/bin/codegraph status`

# Verification

Passed:

- `cargo fmt --check`
- `git diff --check`
- `cargo test -q governance::meta::tests --features axon-pb`
- `cargo test -q invocation_history::tests --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
- `/Users/macbook.silan.tech/.local/bin/codegraph status`

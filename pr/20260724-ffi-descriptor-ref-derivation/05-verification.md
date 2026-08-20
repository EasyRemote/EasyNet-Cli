# Verification

Passed:

- `cargo test runtime_descriptor_resolver --features axon-pb`
- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

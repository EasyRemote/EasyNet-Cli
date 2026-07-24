# Verification

Passed:

- `cargo test default_chat_manifest --features axon-pb`
- `cargo fmt --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check`

Codegraph evidence:

- `codegraph query default_chat_manifest`

Result: `default_chat_manifest` remains the production helper, with the existing manifest regression tests still indexed.

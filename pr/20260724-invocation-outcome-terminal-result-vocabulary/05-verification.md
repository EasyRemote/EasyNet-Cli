# Verification

Passed:

- `cargo test receipt_free_admission_rejection_is_a_typed_terminal_outcome --features axon-pb`
- `cargo fmt --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check`

Codegraph evidence:

- `codegraph query source-compatible`
- `codegraph node InvocationOutcome`

Result: `source-compatible` no longer resolves to production API symbols; `InvocationOutcome` remains the canonical aggregate with existing public methods.

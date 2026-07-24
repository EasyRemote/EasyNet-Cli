# Verification

## Planned checks

- Targeted Rust tests for federation ability contract.
- Targeted Rust tests for daemon invocation dispatch/admission join parsing where available.
- `cargo fmt --check`.
- `git diff --check`.
- Canonical runtime convergence gate if local runtime permits.

## Results

- `cargo test --lib join_args -- --nocapture` — passed.
- `cargo test --lib join_request_rejects_retired_pairing_secret_field -- --nocapture` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.

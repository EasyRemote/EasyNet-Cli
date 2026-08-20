# Verification

## Planned checks

- `cargo test --lib resolve_key_receipt -- --nocapture`
- `cargo fmt --check`
- `git diff --check`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`

## Results

- `cargo test --lib resolve_key_receipt -- --nocapture` — passed.
- `cargo fmt --check` — passed after applying `cargo fmt`.
- `git diff --check` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.

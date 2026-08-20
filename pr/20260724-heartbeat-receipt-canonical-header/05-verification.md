# Verification

## Planned checks

- Targeted `cargo test --lib heartbeat_receipt`.
- `cargo fmt --check`.
- `git diff --check`.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- `tools/scripts/check-architecture-convergence.sh`.

## Results

- `cargo test --lib heartbeat_receipt -- --nocapture` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.

# Verification

## Planned checks

- Swift runtime seam tests.
- Canonical runtime convergence v2 gate.
- Architecture convergence gate.
- `cargo fmt --check`.
- `git diff --check`.

## Evidence log

- `swift test --package-path sdk/swift` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.

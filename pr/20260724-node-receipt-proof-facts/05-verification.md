# Verification

## Planned checks

- Node runtime receipt tests.
- Canonical runtime convergence v2 gate.
- Architecture convergence gate.
- `cargo fmt --check`.
- `git diff --check`.

## Evidence log

- `node --test sdk/node/test/runtime-core.test.mjs` — passed.
- `node --test sdk/node/test/conformance-cases.test.mjs` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.

# Verification

Executed checks:

- `cargo test runtime_descriptor_resolver_ --lib` — passed, 12 tests.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.

Failure-path coverage:

- Missing/invalid ability descriptor subject.
- Device subject rejected before daemon route resolution.
- Cross-realm authority subject rejected before daemon route resolution.
- Valid same-realm authority subject still resolves remote catalogue descriptors through the explicit provider source.

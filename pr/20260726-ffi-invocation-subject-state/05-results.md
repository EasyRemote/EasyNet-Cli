Results:

- Added `ReadyRuntimeCapabilities` to make daemon control discovery Ready validate
  mode-specific runtime proof facts before publishing attachable discovery.
- Device/Both Ready now rejects missing `paired_user_runtime_signer` proof instead
  of advertising Ready and deferring failure to descriptor resolution.
- Hub Ready remains independent from paired User signer custody.
- Replaced the permissive readiness regression test with a fail-closed test and
  added a Hub independence regression test.
- Updated SPEC v2 gate to enforce the new fail-closed Ready contract.

Verification:

- `cargo test --bin easynet-daemon ready_discovery` passed.
- `cargo fmt --check` passed.
- `bash tools/scripts/check-architecture-convergence.sh` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` passed.
- `git diff --check` passed.

Codegraph evidence:

- `codegraph sync .` refreshed the modified daemon entrypoint.
- `codegraph callers -p . ready_runtime_discovery` showed only the production
  daemon `main` caller and readiness regression tests.

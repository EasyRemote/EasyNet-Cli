# Verification

## Commands

- `cargo test daemon_identity --features axon-pb`
  - Result: passed. Covers boot identity projection and owning-schema rejection tests.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - Result: passed. Covers the mutation fixture that reintroduces duplicate `StoredDeviceIdentity`/`agent_ura` projection.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: passed.
- `cargo fmt --check`
  - Result: passed.
- `git diff --check`
  - Result: passed.

## Evidence

`identity.rs` no longer reads `credentials.json` directly, deserializes JSON directly, or defines boot-local retired-field sentinel types. Boot identity now consumes `load_credentials_optional()` and derives the device caller URA from validated `Credentials`.

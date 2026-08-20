# Verification

Planned checks:

- `cargo fmt --check`
- `git diff --check`
- `cargo test --lib --features axon-pb session_initiator -- --nocapture`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`

Executed checks:

- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `cargo test --lib --features axon-pb session_initiator -- --nocapture` —
  passed, 49 tests.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` — passed.
- `rg -n "warm_device_credential_for_session|CredentialWarmupOutcome|verify_device_credential|credential_warmup|REST backstop|continuing to gRPC session prelude" src tests -S`
  — no production/test source matches.

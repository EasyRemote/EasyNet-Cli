# Verification

Planned checks:

- `cargo test --features axon-pb u15_publish_registers_project_abilities_in_local_runtime -- --nocapture`
- targeted keyring/signer policy tests
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`

Executed checks:

- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` — passed.
- `bash tools/scripts/check-start-ready-signer-proof-boundary.sh` — passed.
- `cargo test --features axon-pb u15_publish_registers_project_abilities_in_local_runtime -- --nocapture` — passed.
- `cargo test --features axon-pb publish_ -- --nocapture` — passed; this reruns
  the broader publish filter that previously exposed the pages LocalRuntime
  signer-policy mismatch.
- `cargo test --lib --features axon-pb runtime_signing_requires_exact_public_projection_and_policy -- --nocapture` — passed.
- `cargo test --lib --features axon-pb policy_ref_binds_every_identity_key_component -- --nocapture` — passed.

# Verification

Planned checks:

- `cargo test --features axon-pb publish_ -- --nocapture`
- targeted CLI agent tests affected by canonical registry lookups
- `cargo fmt --check`
- `git diff --check`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`

Executed checks:

- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` — passed.
- `cargo test --lib --features axon-pb start_agent_ -- --nocapture` — passed, 16 tests.
- `cargo test --lib --features axon-pb publish_ -- --nocapture` — passed, 41 tests.
- `cargo test --lib --features axon-pb registered_agent_lookup_canonicalizes_surface_name -- --nocapture` — passed.
- `cargo test --lib --features axon-pb registry_projection_uses_canonical_key_as_storage_not_hosted_name -- --nocapture` — passed.
- `cargo test --lib --features axon-pb list_agents_projects_name_runtime_model_label -- --nocapture` — passed.

Observed but not part of this commit boundary:

- A broader non-lib `cargo test --features axon-pb publish_ -- --nocapture`
  still exposes `tests/pages_unit.rs::u15_publish_registers_project_abilities_in_local_runtime`
  with `runtime signing policy does not match owner projection`. That is a
  separate signer-custody/owner-projection seam and must not be hidden by the
  agent registry canonical-key migration.

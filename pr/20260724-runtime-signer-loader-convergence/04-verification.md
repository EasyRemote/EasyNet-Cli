# Verification

Completed checks:

- `cargo test --lib canonical_caller_ura` — passed, 2 tests.
- `cargo test --lib daemon_identity_rejects_retired_tenant_id_credentials` — passed, 1 test.
- `cargo test --lib hub_only_identity_loading` — passed, 2 tests.
- `cargo test --lib daemon_identity_from_stored_accepts_realm_only_credentials` — passed, 1 test.
- `cargo test --lib cli::commands::federation_wire` — passed, 20 tests.
- `cargo fmt --check` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `git diff --check` — passed.

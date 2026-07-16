# Verification

Passed checks:

- `cargo test --features axon-pb --lib authority_binding_rejects_nested_scalar_identity_fields -- --nocapture`
  - 1 passed.
- `cargo test --features axon-pb --lib policy_mutations_reject_scalar_only_identity_boundaries -- --nocapture`
  - 1 passed.
- `cargo test --features axon-pb --lib authority_binding_grant_derives_owner_and_user_principal_from_ura -- --nocapture`
  - 1 passed.
- `cargo test --features axon-pb --lib authority_binding_list_supports_rfc014_scope_filters -- --nocapture`
  - 1 passed.
- `cargo test --features axon-pb --lib access_control -- --nocapture`
  - 30 passed.
- `tools/scripts/check-architecture-convergence.sh`
  - Passed.
- `rustfmt --edition 2021 --check src/daemon/ability/builtins/governance/access_control.rs`
  - Passed.
- `git diff --check -- src/daemon/ability/builtins/governance/access_control.rs pr/20260716-access-control-ura-wire-dto`
  - Passed.

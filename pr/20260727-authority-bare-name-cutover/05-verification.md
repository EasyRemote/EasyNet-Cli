# Verification

- `cargo fmt --check` passed.
- `cargo test -q daemon::axon_bridge::descriptor_ref --features axon-pb`
  passed.
- `cargo test -q owner_local_ability_name --features axon-pb` passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.
- `codegraph sync .` completed after the code changes.

## Coverage

- `authority_bare_hub_prefixed_name_is_rejected` proves descriptor-wire
  construction rejects `hub.*` Authority aliases.
- `owner_local_ability_name_projects_registry_key_to_public_name` now proves
  Authority owner-local projection does not strip `hub.`.
- SPEC v2 gate `check_authority_hub_ability_alias_retirement_contract` prevents
  reintroducing the core strip-prefix alias or positive descriptor-ref alias
  test.

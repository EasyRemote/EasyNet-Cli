# Ability manifest schema-version strictness

## Goal

Remove the executable compatibility path where `AbilityManifest::from_toml_str` accepts an ability manifest without `schema_version` and treats it as an implicit v1 shape.

## Root abstraction problem

`ability.toml` is a daemon-owned persistence/import schema. A missing schema version is not an explicit state in the manifest lifecycle; accepting it creates a hidden migration path where old files can enter runtime registration without an auditable version boundary.

Agent manifests already fail closed for missing `schema_version`. Ability manifests must use the same storage rule: every persisted/imported schema carries an explicit version, and unsupported or absent versions are rejected at parse/validation time.

## Boundary invariants

1. `AbilityManifest` stores `schema_version` as a required field, not `Option<String>`.
2. Deserializing TOML or JSON without `schema_version` fails closed.
3. Unknown `schema_version` still fails closed through semantic validation.
4. Writers always stamp `CURRENT_SCHEMA_VERSION`.
5. No test or comment describes missing schema version as a supported pre-stamp/implicit-v1 compatibility path.
6. SPEC v2 gate covers ability manifest schema strictness directly.

## Verification plan

- Run targeted Rust tests for `daemon::ability::manifest`.
- Run `cargo fmt --check` and `git diff --check`.
- Run `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- Run `tools/scripts/check-architecture-convergence.sh`.

## Decisions

- Do not add a migration fallback. Old manifest files without `schema_version` must be rewritten before use.
- Keep `CURRENT_SCHEMA_VERSION` unchanged; this is not a new schema shape, it is a stricter admission boundary for the existing schema.

# EAL trace schema-version strictness

## Goal

Remove the executable compatibility path where `ExecutionTrace` JSON without `schema_version` deserializes as the current trace schema.

## Root abstraction problem

Execution traces are audit/read-model artifacts. Treating an absent schema version as current state makes old pre-stamp trace files indistinguishable from explicitly versioned current traces. That undermines replay/audit boundaries because readers cannot decide whether they are processing a known schema or an implicitly migrated artifact.

## Boundary invariants

1. Fresh traces always serialize `schema_version = EXECUTION_TRACE_SCHEMA_VERSION`.
2. Deserializing a trace without `schema_version` fails closed.
3. There is no `serde(default = "...schema_version...")` fallback on `ExecutionTrace.schema_version`.
4. Tests must pin missing-version rejection, not tolerant pre-stamp parsing.
5. SPEC v2 gate must reject pre-stamp/tolerant-read language in the EAL trace schema contract.

## Verification plan

- Run targeted Rust tests for `trace_schema_v1_is_stable`.
- Run `cargo fmt --check` and `git diff --check`.
- Run `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
- Run `tools/scripts/check-architecture-convergence.sh`.

## Decisions

- Do not add a migration fallback. Old trace files without `schema_version` are not valid current audit artifacts.
- Keep `EXECUTION_TRACE_SCHEMA_VERSION = 1`; this change tightens the read boundary for the existing stamped schema.
- Treat EAL traces as audit artifacts, not tolerant product DTOs. Added audit facts require an explicit schema-version boundary instead of relying on serde default compatibility.

## Implementation delta

- Removed the `current_trace_schema_version` fallback and the `#[serde(default = "...")]` attribute from `ExecutionTrace.schema_version`.
- Replaced the legacy readback assertion with a fail-closed missing-`schema_version` assertion.
- Added `check_eal_trace_schema_version_strict_contract` to SPEC v2 so tolerant-read language and schema-version defaults cannot return.

## Verification results

- `cargo test -q trace_schema_v1_is_stable --features axon-pb` passed.
- `cargo test -q eal::interpreter --features axon-pb` passed.
- `cargo fmt --check` passed.
- `git diff --check` passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.

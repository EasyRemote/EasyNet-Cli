# Principal Lifecycle Canonical JSON DTO Convergence

## Goal

Remove silent compatibility acceptance from the PrincipalLifecycle JSON boundary. Principal lifecycle
state participates in admission and runtime trust, so unknown persisted fields or unknown ability
argument fields must fail closed rather than being ignored.

## Root Abstraction Problem

`PrincipalStore`, `PrincipalRecord`, nested principal lifecycle facts, and principal lifecycle request
DTOs were deserialized without `deny_unknown_fields`. That means a retired or product-specific field
could remain in durable principal state or ability input without being surfaced as malformed runtime
authority state.

This is not a public source-compatibility surface. It is an authority/admission data boundary.

## Invariants

1. Durable principal lifecycle JSON is canonical-only.
2. Unknown persisted lifecycle fields fail closed before admission state is projected.
3. Unknown principal lifecycle ability argument fields fail closed before command execution.
4. No migration, alias, or fallback parser is introduced for retired principal lifecycle shapes.
5. Existing canonical principal lifecycle behavior and public ability names remain unchanged.

## Boundary Proof

- Principal lifecycle request decoding remains in `decode_args`; adding strict DTO validation changes
  only malformed inputs.
- Store loading remains the single durable read path through `PrincipalStore::load(_unlocked)`.
- Runtime trust writes remain behind successful command validation and state-machine transitions.

## Planned Change

Add `#[serde(deny_unknown_fields)]` to the principal lifecycle store, nested persisted records, and
request DTOs, then add regression tests proving unknown fields are rejected.

## Verification

- Focused principal lifecycle tests.
- `cargo fmt --check`.
- `git diff --check`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Results

- `cargo test -q --lib principal_lifecycle --features axon-pb -- --test-threads=1`
  - Result: 21 passed, 0 failed.
- `cargo fmt --check`
  - Result: passed after rustfmt normalized the new assertion.
- `git diff --check`
  - Result: passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: `canonical-runtime-convergence-v2: OK`.
- `tools/scripts/check-architecture-convergence.sh`
  - Result: `architecture-convergence: OK`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
  - Result: synced changed Rust files.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
  - Result: index up to date.

## Out-of-Scope Observation

`cargo test -q principal_lifecycle --features axon-pb -- --test-threads=1` also selects
`tests/principal_lifecycle_daemon_e2e.rs::principal_lifecycle_runs_through_real_daemon_and_survives_restart`.
That e2e failed before command dispatch with:

`invocation attempt audit ledger is not wired; refusing to dispatch without pre-runtime failure observability`

The failure is not caused by the strict JSON DTO change; the focused lib tests and architecture gates
cover this canonical JSON boundary.

## Decision Record

Principal lifecycle JSON is treated as an admission/runtime trust boundary, not a compatibility API.
Unknown persisted or request fields now fail closed through serde DTO validation. Tests that manually
fabricated lifecycle stores were migrated to the canonical record shape instead of restoring legacy
deserialization tolerance.

# Seven-Axes Canonical Daemon Fixture Convergence

## Goal

Converge the shared seven-axes daemon E2E fixture onto the same boot facts required by production
daemon invocation transport:

- an invocation attempt audit ledger is wired before dispatch;
- seeded agent specs use the canonical schema-versioned shape.

## Root Abstraction Problem

The fixture constructed `DaemonInvocationService` directly and attached the canonical invocation
ledger, but omitted the transport-boundary attempt audit ledger that production boot now requires.
As a result, the real daemon E2E failed before principal lifecycle dispatch with:

`invocation attempt audit ledger is not wired; refusing to dispatch without pre-runtime failure observability`

The same fixture also seeded `agent.toml` files without `schema_version`, leaving a retired agent
spec shape in product E2E setup.

## Invariants

1. Every daemon Invocation transport used by product E2E has pre-runtime attempt observability.
2. The dispatcher must continue to fail closed when no attempt audit ledger is configured.
3. Product E2E fixtures must not seed retired agent spec shapes.
4. The fixture must use one HOME-owned attempt ledger path across daemon restarts.
5. No production fallback or optional audit bypass is introduced.

## Boundary Proof

- Production boot already opens the required attempt audit ledger before serving Invocation.
- Integration fixtures that bypass production boot must explicitly supply equivalent boot facts.
- Agent specs are test input data; canonicalizing them does not alter public product behavior.

## Planned Change

Update `tests/seven_axes_fixture/mod.rs` to:

- store a HOME-owned `invocation-attempts.jsonl` path;
- pass it to `DaemonInvocationService::with_invocation_attempt_ledger_path`;
- seed `schema_version = "1"` in fixture `agent.toml` files.

## Verification

- `cargo test -q --test principal_lifecycle_daemon_e2e principal_lifecycle_runs_through_real_daemon_and_survives_restart --features axon-pb -- --test-threads=1`
- Focused SevenAxes fixture-adjacent tests if needed.
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`

## Results

- `cargo test -q --test principal_lifecycle_daemon_e2e principal_lifecycle_runs_through_real_daemon_and_survives_restart --features axon-pb -- --test-threads=1`
  - Result: 1 passed, 0 failed.
- `cargo test -q principal_lifecycle --features axon-pb -- --test-threads=1`
  - Result: selected principal lifecycle lib tests and daemon e2e passed.
- `cargo fmt --check`
  - Result: passed.
- `git diff --check`
  - Result: passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: `canonical-runtime-convergence-v2: OK`.
- `tools/scripts/check-architecture-convergence.sh`
  - Result: `architecture-convergence: OK`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
  - Result: synced the changed fixture file.
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`
  - Result: index up to date.

## Decision Record

The daemon dispatcher remains fail-closed when `attempt_ledger` is absent. The fixture now supplies
the same boot fact production boot supplies, rather than adding a test bypass or weakening dispatch.
The seeded agent specs were updated to the canonical schema-versioned shape so product E2E no longer
starts from retired agent directory data.

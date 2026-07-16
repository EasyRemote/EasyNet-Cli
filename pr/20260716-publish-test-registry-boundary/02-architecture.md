# Architecture

## Boundary

`publish.rs` has two responsibilities:

- production ability handlers;
- local unit-test fixtures.

Production handler code should depend on `AgentAggregateRepository` for owner
workspace lookup. The test fixture may still use `agent_registry` because it
constructs isolated fake agents under `HomeGuard`.

## Change

Move the `agent_registry as agents` alias from production module scope to the
test module. This keeps the dependency visible only where it is actually used.

## Effect

The source graph now reflects the intended boundary: publish/unpublish runtime
logic is aggregate-backed; direct registry writes are test fixture mechanics.

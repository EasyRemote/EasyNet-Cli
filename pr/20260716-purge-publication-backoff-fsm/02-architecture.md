# Architecture

## Ownership

The purge publication FSM belongs to the EasyNet-Cli daemon persistence/domain
layer. It coordinates product runtime cleanup after local Agent purge. It does
not define Axon Invocation semantics and performs no network I/O.

## State Machine Boundary

- `src/daemon/persistence/agent_lifecycle.rs` owns the device outbox FSM:
  `ready`, `claimed`, `backing_off`, and `reconciliation_required`.
- `src/daemon/persistence/federation_revoke.rs` owns the Hub durable revoke FSM:
  `Prepared` and `Applied`.
- `src/daemon/invocation/dispatch/federation_wrappers.rs` adapts the daemon
  ability request to the durable Hub FSM.

## Convergence

This slice removes an implicit immediate-retry edge from the scheduled-drain
path. Backoff eligibility is now expressed by the state machine itself rather
than by caller timing or local retry heuristics.

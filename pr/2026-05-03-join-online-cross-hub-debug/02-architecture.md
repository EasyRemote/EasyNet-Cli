# Architecture

## Boundaries

- `src/facade/cli/join.rs`
  Persists pairing output and triggers best-effort local auto-wire helpers.
- `src/facade/cli/federation_wire.rs`
  Owns join-time daemon-config / realm-trust edits.
- `src/support/federation_invoke.rs`
  Owns CLI `--node` parsing and migration-window normalization.
- `src/services/axon_serve/daemon_invocation_service.rs`
  Owns hub-side forward-invoke routing and presence lookup.
- `src/services/axon_serve/federation_wrappers.rs`
  Owns presence-backed list/projection handlers.
- `src/runtime/agents/federation_probe.rs`
  Owns operator-facing live node/probe view used by monitoring surfaces.

## Constraint

Normalize at the boundary, not deep inside every caller. Monitoring code should consume canonical device URIs, not re-invent them.

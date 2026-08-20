# Boundary Proof

## Ownership

The Node facade owns:

- DTO construction and validation for Events profile request objects.
- JSON serialization into injected transport methods.
- DTO projection of daemon-provided Events profile responses.
- Binding daemon stream handles into the existing bounded `StreamHandle`.

The injected transport owns:

- Building complete Invocation carriers for daemon system abilities.
- Opening live event streams.
- Projecting raw daemon event payloads, drop reports, terminal frames, and
  history pages according to the daemon/Axon provider contract.

## Rejected Designs

- SDK-local event bus: rejected because ordering, filtering, replay, and
  terminal facts belong to the daemon runtime.
- Product session URA parsing: rejected because session stream attachment is a
  daemon operation keyed by explicit `session_id`.
- Backend SSE/WebSocket helpers: rejected because backend fan-out and browser
  auth are product responsibilities outside the daemon SDK.
- Duplicated stream state machine: rejected because Node already has a bounded
  `StreamHandle` with terminal and overflow behavior.

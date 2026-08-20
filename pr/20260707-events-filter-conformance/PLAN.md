# Events Filter Conformance Plan

## Goal

Promote Events profile filtering from scattered request fields to a shared
Go/Python/Rust SDK value object that is lowered into daemon-owned event
subscription Invocation args.

## Boundary Proof

- `EventFilter` is a carrier DTO, not an event bus or policy engine.
- The SDK validates shape, stream compatibility, and bounded request fields.
- Filtering is executed by daemon abilities such as `events.device.subscribe`,
  `events.invocation.subscribe`, and `events.device.history`.
- The SDK must not post-filter live frames or redefine daemon stream semantics.

## Invariants

1. A typed filter and legacy request fields normalize to one daemon args shape.
2. Device filters require Device URAs where the stream requires device scope.
3. Invocation filters require an explicit invocation id for invocation streams.
4. Runtime-backed subscriptions carry the same filter args as carrier builders.
5. Go and Python execute the same shared conformance fixture.

## Verification

- `cargo test events_filter`
- `go test ./...` in `sdk/go`.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`.
- SDK parity, scaffold, and cutover self-test scripts.

## Remaining After This Slice

- Backend SSE/WebSocket cutover to SDK event streams.
- Daemon implementation of broader live stream filtering beyond current system
  abilities.
- Authority minting transport.
- RFC-007 receipt URA builder after Axon/daemon decision lands.
- Trust/certificate policy persistence.

# Federation heartbeat canonical contract strictness

## Goal

Remove the remaining `federation.heartbeat` compatibility-shaped ingress contract and converge the wire request onto the fields the current session heartbeat producer actually sends and the hub handler actually consumes.

## Root abstraction problem

`federation.heartbeat` had two competing request models:

- the session producer sends `since_abilities_revision` and `refresh_owner_uras`;
- the daemon manifest still advertised `node_id`, `agent_ura`, `owner_ura`, and `generation`, with open additional properties;
- the Rust DTO accepted unknown fields and carried an unused optional `agent_ura` field.

That is a compatibility layer: product callers can send old or misspelled fields and the hub silently ignores them. It also makes the hub abilities revision look provider-backed even though the hub-side handler only returns an explicit empty diff.

## Invariants

1. `federation.heartbeat` request shape is closed.
2. The only canonical request fields are `since_abilities_revision` and `refresh_owner_uras`.
3. `refresh_owner_uras` is mandatory; an empty array is valid only as an explicit empty lease-refresh batch.
4. `agent_ura`, `node_id`, `owner_ura`, and `generation` are retired ingress fields and must fail closed.
5. The response `hub_abilities_diff.revision` must be bound to the request revision until a provider-backed hub ability diff source is wired.

## Boundary proof

- Liveness identity comes from the signed invocation envelope and presence registry, not request `agent_ura`.
- Lease refresh ownership comes only from `refresh_owner_uras`, and bootstrap authority separately restricts each owner to the envelope caller.
- Hub ability broadcast remains an explicit seam: heartbeat acknowledges the caller revision with an empty diff instead of silently discarding the field.

## Implementation delta

- Remove `HeartbeatRequest.agent_ura`.
- Add `HeartbeatRequest.since_abilities_revision`.
- Add `#[serde(deny_unknown_fields)]` to `HeartbeatRequest`.
- Update the daemon ability schema to the closed canonical heartbeat request fields.
- Pin regression tests for canonical request acceptance, retired field rejection, and response revision binding.
- Extend SPEC v2 gate so the legacy heartbeat request shape cannot return.

## Verification

Completed:

- `cargo test -q daemon::invocation::dispatch::federation_wrappers --features axon-pb`
- `cargo test -q invoke_dispatches_federation_heartbeat --features axon-pb`
- `cargo test -q heartbeat_args_are_closed_canonical_request_shape --features axon-pb`
- `cargo test -q paired_device_can_refresh_own_heartbeat_projection --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph affected ...`

## Decisions

- `federation.heartbeat` remains a hub ability broadcast seam, not a fake provider-backed diff source.
- The seam is now explicit: the hub-side wrapper echoes `since_abilities_revision` in an empty diff so the caller's revision fact is not silently ignored.
- Request identity remains envelope-owned. `agent_ura`, `node_id`, `owner_ura`, and `generation` are not heartbeat request authority facts.
- The producer now uses typed `HeartbeatArgs` instead of hand-written JSON, keeping client and daemon DTOs aligned.

## Result

- Removed the unused heartbeat `agent_ura` request field.
- Closed heartbeat request deserialization with `deny_unknown_fields`.
- Replaced the legacy/open manifest schema with the canonical heartbeat request schema.
- Added SPEC v2 coverage for DTO, schema, producer, and retired field tests.

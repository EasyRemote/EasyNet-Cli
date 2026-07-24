# Architecture

## Boundary

The federation client contract is the canonical runtime receipt projection for federation abilities. It is not a compatibility layer for older hub wrapper JSON.

## Module ownership

- `src/daemon/federation/client/ability_contract.rs` owns heartbeat receipt decoding.
- `src/daemon/invocation/bidi/session_initiator/heartbeat.rs` owns session heartbeat behavior over the decoded contract.
- Hub dispatch wrappers own producing canonical receipt fields.

## Refactoring direction

Delete alias fields from the data type and make `deny_unknown_fields` enforce the boundary, rather than keeping unused optional members.
